//! App-event translation and the transport event sink for Tauri IPC.
//!
//! `fft_app` events are translated to the existing hyphenated Tauri event
//! names. Script output/exit events use a separate sink owned by the scripting
//! layer; they are never injected into [`fft_app::AppEvent`].
//!
//! A long-lived translator thread owns the only strong `AppHandle` (via
//! [`TauriEmitter`]); the managed adapter never stores one. `TransportSink`
//! serializes messages to that thread and shutdown sends a FIFO `Stop` marker
//! before joining, so queued events are flushed and the handle is dropped.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use fft_app::{AppEvent, AppEventSink};
use fft_runtime::{JobId, ProgressCoalescer, RuntimeEvent};
use serde::Serialize;

use crate::core::CommandCore;
use crate::dto::{
    AnalysisDoneEvent, AnalysisErrorEvent, AnalysisProgressEvent, DisplayClosedEvent,
    DisplayEventPayload,
};

/// App-level event names kept identical to the current desktop shell.
pub const APP_EVENT_NAMES: &[&str] = &[
    "analysis-progress",
    "analysis-done",
    "analysis-error",
    "display-open",
    "display-updated",
    "display-closed",
];

/// Scripting event names (owned by the scripting layer).
pub const SCRIPT_EVENT_NAMES: &[&str] = &["script-output", "script-exit"];

/// All event names the desktop transport may deliver.
pub const EVENT_NAMES: &[&str] = &[
    "analysis-progress",
    "analysis-done",
    "analysis-error",
    "display-open",
    "display-updated",
    "display-closed",
    "script-output",
    "script-exit",
    "notification",
];

/// Small transport abstraction so event translation can be tested without a
/// Tauri `AppHandle` or a live window.
pub(crate) trait EventEmitter: Send + Sync {
    fn emit(&self, name: &str, payload: serde_json::Value);
}

/// Production emitter. The handle lives only on the translator thread.
pub(crate) struct TauriEmitter {
    app: tauri::AppHandle,
}

impl TauriEmitter {
    pub(crate) fn new(app: tauri::AppHandle) -> Self {
        Self { app }
    }
}

impl EventEmitter for TauriEmitter {
    fn emit(&self, name: &str, payload: serde_json::Value) {
        use tauri::Emitter as _;
        let _ = self.app.emit(name, payload);
    }
}

/// Events for a job without a display id are buffered until the adapter
/// registers the analysis job. The queue is bounded so a busy process cannot
/// grow it without bound; load/transient-cut jobs are ignored instead.
const MAX_PENDING_EVENTS: usize = 512;

pub(crate) enum SinkMessage {
    AppEvent(AppEvent),
    Register {
        job: JobId,
        display_id: u64,
    },
    Ignore {
        job: JobId,
    },
    /// App hosts do not publish cancellation as an app event; the adapter
    /// reports an `analysis-error` payload itself.
    Cancelled {
        job: JobId,
        display_id: u64,
    },
    AttachCleanup(Weak<CommandCore>),
    Stop,
}

enum SinkBackend {
    Channel(Sender<SinkMessage>),
    /// Test-only synchronous path; no Tauri app or thread is created.
    #[cfg(test)]
    Inline(Translator),
}

/// Sink side of the desktop event bridge.
pub(crate) struct TransportSink {
    backend: Mutex<Option<SinkBackend>>,
}

impl TransportSink {
    pub(crate) fn channel(sender: Sender<SinkMessage>) -> Self {
        Self {
            backend: Mutex::new(Some(SinkBackend::Channel(sender))),
        }
    }

    #[cfg(test)]
    pub(crate) fn inline(emitter: Arc<dyn EventEmitter>) -> Self {
        Self {
            backend: Mutex::new(Some(SinkBackend::Inline(Translator::new(emitter)))),
        }
    }

    fn dispatch(&self, message: SinkMessage) {
        let mut backend = self
            .backend
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        match backend.as_mut() {
            Some(SinkBackend::Channel(sender)) => {
                // Unbounded channel: emitting from a runtime worker never
                // blocks. A disconnected receiver simply drops the message.
                let _ = sender.send(message);
            }
            #[cfg(test)]
            Some(SinkBackend::Inline(translator)) => translator.handle(message),
            None => {}
        }
    }

    pub(crate) fn register(&self, job: JobId, display_id: u64) {
        self.dispatch(SinkMessage::Register { job, display_id });
    }

    pub(crate) fn ignore(&self, job: JobId) {
        self.dispatch(SinkMessage::Ignore { job });
    }

    pub(crate) fn cancelled(&self, job: JobId, display_id: u64) {
        self.dispatch(SinkMessage::Cancelled { job, display_id });
    }

    /// Attach the host cleanup hook. The weak reference avoids a
    /// sink -> core -> sink cycle.
    pub(crate) fn attach_cleanup(&self, core: Weak<CommandCore>) {
        self.dispatch(SinkMessage::AttachCleanup(core));
    }

    /// Close the channel and ask the translator to flush queued messages and
    /// exit. Dropping the sender disconnects the receiver after `Stop`.
    pub(crate) fn shutdown(&self) {
        let mut backend = self
            .backend
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(SinkBackend::Channel(sender)) = backend.take() {
            let _ = sender.send(SinkMessage::Stop);
        }
    }
}

impl AppEventSink for TransportSink {
    fn emit(&self, event: AppEvent) {
        self.dispatch(SinkMessage::AppEvent(event));
    }
}

struct Translator {
    emitter: Arc<dyn EventEmitter>,
    job_displays: HashMap<JobId, u64>,
    /// Jobs whose events must never be translated (loads and transient cuts).
    ignored: HashSet<JobId>,
    pending: VecDeque<AppEvent>,
    progress: ProgressCoalescer,
    cleanup: Option<Weak<CommandCore>>,
}

impl Translator {
    fn new(emitter: Arc<dyn EventEmitter>) -> Self {
        Self {
            emitter,
            job_displays: HashMap::new(),
            ignored: HashSet::new(),
            pending: VecDeque::new(),
            progress: ProgressCoalescer::new(Duration::from_millis(75)),
            cleanup: None,
        }
    }

    fn handle(&mut self, message: SinkMessage) {
        match message {
            SinkMessage::AppEvent(event) => self.dispatch(event),
            SinkMessage::Register { job, display_id } => self.register(job, display_id),
            SinkMessage::Ignore { job } => self.ignore(job),
            SinkMessage::Cancelled { job, display_id } => {
                self.send_analysis_error(display_id, "analysis cancelled");
                self.finish(job);
                self.cleanup_display(display_id, job);
            }
            SinkMessage::AttachCleanup(core) => self.cleanup = Some(core),
            SinkMessage::Stop => {}
        }
    }

    fn dispatch(&mut self, event: AppEvent) {
        match event {
            AppEvent::DisplayOpened { display } => {
                self.send("display-open", DisplayEventPayload { display })
            }
            AppEvent::DisplayUpdated { display } => {
                self.send("display-updated", DisplayEventPayload { display })
            }
            AppEvent::DisplayClosed { id, children } => self.send(
                "display-closed",
                DisplayClosedEvent {
                    id: id.get(),
                    children: children.iter().map(|child| child.get()).collect(),
                },
            ),
            AppEvent::JobProgress { job, .. }
            | AppEvent::JobDone { job }
            | AppEvent::JobFailed { job, .. } => self.dispatch_job_event(event, job),
        }
    }

    fn dispatch_job_event(&mut self, event: AppEvent, job: JobId) {
        let Some(display_id) = self.job_displays.get(&job).copied() else {
            if self.ignored.contains(&job) {
                if app_event_is_terminal(&event) {
                    self.ignored.remove(&job);
                }
                return;
            }
            if self.pending.len() >= MAX_PENDING_EVENTS {
                self.pending.pop_front();
            }
            self.pending.push_back(event);
            return;
        };

        if !self.should_forward(&event) {
            return;
        }
        self.send_job_event(&event, display_id);
        if app_event_is_terminal(&event) {
            self.job_displays.remove(&job);
            if matches!(event, AppEvent::JobFailed { .. }) {
                self.cleanup_display(display_id, job);
            }
        }
    }

    fn register(&mut self, job: JobId, display_id: u64) {
        self.job_displays.insert(job, display_id);
        self.ignored.remove(&job);

        let pending = std::mem::take(&mut self.pending);
        let mut remaining = VecDeque::with_capacity(pending.len());
        let mut failures = Vec::new();
        for event in pending {
            let Some(event_job) = app_event_job(&event) else {
                self.dispatch(event);
                continue;
            };
            match self.job_displays.get(&event_job).copied() {
                Some(display) => {
                    if self.should_forward(&event) {
                        self.send_job_event(&event, display);
                    }
                    if app_event_is_terminal(&event) {
                        self.job_displays.remove(&event_job);
                        if matches!(event, AppEvent::JobFailed { .. }) {
                            failures.push((display, event_job));
                        }
                    }
                }
                None => remaining.push_back(event),
            }
        }
        self.pending = remaining;

        for (display, job) in failures {
            self.cleanup_display(display, job);
        }
    }

    fn ignore(&mut self, job: JobId) {
        self.job_displays.remove(&job);
        if self.ignored.len() >= MAX_PENDING_EVENTS {
            self.ignored.clear();
        }
        self.ignored.insert(job);
        self.pending
            .retain(|event| app_event_job(event) != Some(job));
    }

    fn finish(&mut self, job: JobId) {
        self.job_displays.remove(&job);
        self.ignored.remove(&job);
    }

    fn should_forward(&mut self, event: &AppEvent) -> bool {
        let Some(runtime_event) = runtime_event_for(event) else {
            return true;
        };
        self.progress.should_forward(&runtime_event, Instant::now())
    }

    fn send_job_event(&self, event: &AppEvent, display_id: u64) {
        match event {
            AppEvent::JobProgress {
                frames_done,
                frames_total,
                eta_s,
                ..
            } => self.send(
                "analysis-progress",
                AnalysisProgressEvent {
                    display_id,
                    frames_done: *frames_done,
                    frames_total: *frames_total,
                    eta_seconds: *eta_s,
                },
            ),
            AppEvent::JobDone { .. } => {
                self.send("analysis-done", AnalysisDoneEvent { display_id })
            }
            AppEvent::JobFailed { message, .. } => self.send_analysis_error(display_id, message),
            _ => {}
        }
    }

    fn send_analysis_error(&self, display_id: u64, message: &str) {
        self.send(
            "analysis-error",
            AnalysisErrorEvent {
                display_id,
                message: message.to_string(),
            },
        );
    }

    fn cleanup_display(&self, display_id: u64, job: JobId) {
        // The transport attaches this only for the channel translator. The
        // inline test sink intentionally has no cleanup hook: cleanup emits
        // app events and would re-enter the inline lock.
        if let Some(core) = self.cleanup.as_ref().and_then(Weak::upgrade) {
            core.cleanup_failed_display(display_id, job);
        }
    }

    fn send<T: Serialize>(&self, name: &str, payload: T) {
        if let Ok(payload) = serde_json::to_value(payload) {
            self.emitter.emit(name, payload);
        }
    }
}

fn app_event_job(event: &AppEvent) -> Option<JobId> {
    match event {
        AppEvent::JobProgress { job, .. }
        | AppEvent::JobDone { job }
        | AppEvent::JobFailed { job, .. } => Some(*job),
        _ => None,
    }
}

fn app_event_is_terminal(event: &AppEvent) -> bool {
    matches!(event, AppEvent::JobDone { .. } | AppEvent::JobFailed { .. })
}

fn runtime_event_for(event: &AppEvent) -> Option<RuntimeEvent> {
    match event {
        AppEvent::JobProgress {
            job,
            frames_done,
            frames_total,
            eta_s,
        } => Some(RuntimeEvent::Progress {
            job: *job,
            frames_done: *frames_done,
            frames_total: *frames_total,
            eta_s: *eta_s,
        }),
        AppEvent::JobDone { job } => Some(RuntimeEvent::Done { job: *job }),
        AppEvent::JobFailed { job, message } => Some(RuntimeEvent::Failed {
            job: *job,
            message: message.clone(),
        }),
        _ => None,
    }
}

fn run_translator(emitter: Arc<dyn EventEmitter>, receiver: Receiver<SinkMessage>) {
    let mut translator = Translator::new(emitter);
    while let Ok(message) = receiver.recv() {
        if matches!(message, SinkMessage::Stop) {
            break;
        }
        translator.handle(message);
    }
}

pub(crate) fn spawn_translator(
    emitter: Arc<dyn EventEmitter>,
    receiver: Receiver<SinkMessage>,
) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name("fft-tauri-events".into())
        .spawn(move || run_translator(emitter, receiver))
        .expect("failed to start the Tauri event translator")
}
