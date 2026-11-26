//! App-event to WebSocket bridge.
//!
//! The bridge subscribes once to the leased app. `display.*` events are
//! forwarded directly; `AppEvent::Job*` events are translated to the
//! `analysis.*` payloads by mapping the job back to its display through the
//! app/runtime contract. A failed analysis closes its display after the
//! advisory error. There is deliberately no script event translation.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Instant;

use fft_app::{AppEvent, AppEventSink, AppLease, DisplayId, DisplayInfo, FftApp};
use fft_runtime::{AnalysisId, EntityId, JobId, ProgressCoalescer, RuntimeEvent};
use serde_json::{json, Value};
use tokio::sync::broadcast;

/// Broadcast ring for WS clients. Progress is coalesced before it reaches the
/// ring, so a slow client only misses advisory progress.
pub(crate) const EVENT_RING_CAPACITY: usize = 1024;

/// Job events may arrive before the adapter registers the analysis display
/// (fast jobs can finish first). The pending queue is bounded.
const MAX_PENDING_EVENTS: usize = 512;

/// Translates [`AppEvent`]s into the WS JSON payloads used by the Web UI.
pub(crate) struct EventBridge {
    events: broadcast::Sender<Value>,
    /// The lease is held weakly to avoid an app -> registry sink -> lease ->
    /// app reference cycle. The server state owns the strong lease.
    lease: Weak<dyn AppLease>,
    state: Mutex<BridgeState>,
}

#[derive(Default)]
struct BridgeState {
    /// Displays whose `display.open` payload has already been sent. Job events
    /// are never mapped before this entry exists, so a fast job cannot be
    /// delivered ahead of its display.
    opened: BTreeMap<u64, AnalysisId>,
    ignored: HashSet<JobId>,
    pending: VecDeque<AppEvent>,
    progress: ProgressCoalescer,
}

impl EventBridge {
    pub(crate) fn new(events: broadcast::Sender<Value>, lease: Weak<dyn AppLease>) -> Self {
        Self {
            events,
            lease,
            state: Mutex::new(BridgeState::default()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, BridgeState> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn app(&self) -> Option<Arc<dyn AppLease>> {
        self.lease.upgrade()
    }

    /// Remember displays that already existed when the server subscribed.
    ///
    /// Clients fetch `/api/displays` on connect; this only lets later job
    /// events for pre-existing analyses resolve to their display.
    pub(crate) fn prime(&self, app: &dyn FftApp) {
        let Ok(displays) = app.displays() else {
            return;
        };
        let mut state = self.lock();
        for display in displays {
            if let Some(analysis) = analysis_id_for_display(&display) {
                state.opened.entry(display.id.get()).or_insert(analysis);
            }
        }
    }

    /// Never translate job events for this job (loads and transient cuts).
    ///
    /// Any buffered events are dropped. A terminal buffered event also clears
    /// the marker because no further event can arrive for that job.
    pub(crate) fn ignore(&self, job: JobId) {
        let mut state = self.lock();
        let mut terminal = false;
        state.pending.retain(|event| {
            if event_job(event) == Some(job) {
                terminal |= event_is_terminal(event);
                false
            } else {
                true
            }
        });
        if terminal {
            state.ignored.remove(&job);
        } else {
            if state.ignored.len() >= MAX_PENDING_EVENTS {
                state.ignored.clear();
            }
            state.ignored.insert(job);
        }
    }

    fn send_value(&self, value: Value) {
        let _ = self.events.send(value);
    }

    fn send_display_event(&self, event: &AppEvent) {
        let value = match event {
            AppEvent::DisplayOpened { display } => {
                json!({ "type": "display.open", "display": display })
            }
            AppEvent::DisplayUpdated { display } => {
                json!({ "type": "display.updated", "display": display })
            }
            AppEvent::DisplayClosed { id, children } => json!({
                "type": "display.closed",
                "id": id.get(),
                "children": children.iter().map(|child| child.get()).collect::<Vec<_>>(),
            }),
            _ => return,
        };
        self.send_value(value);
    }

    /// Emit a display-open event before any job event for that display.
    ///
    /// The display id is registered under the bridge lock after the
    /// `display.open` payload is queued; job events block on that lock and
    /// therefore cannot overtake the open event.
    fn emit_display_opened(&self, event: &AppEvent, display: &DisplayInfo) {
        let analysis = analysis_id_for_display(display);
        let job = analysis.and_then(|analysis| {
            let lease = self.app()?;
            lease.app().runtime().job_for(EntityId::Analysis(analysis))
        });

        let mut state = self.lock();
        self.send_display_event(event);
        if let Some(analysis) = analysis {
            state.opened.insert(display.id.get(), analysis);
        }
        let Some(job) = job else {
            return;
        };
        if state.ignored.contains(&job) {
            return;
        }
        let mut batch = Vec::new();
        let mut remaining = VecDeque::with_capacity(state.pending.len());
        while let Some(pending) = state.pending.pop_front() {
            if event_job(&pending) == Some(job) {
                batch.push(pending);
            } else {
                remaining.push_back(pending);
            }
        }
        state.pending = remaining;
        let failed = batch
            .iter()
            .any(|pending| matches!(pending, AppEvent::JobFailed { .. }));
        for pending in batch {
            self.send_job_event(&mut state, &pending, display.id.get());
        }
        drop(state);
        if failed {
            self.close_failed_display(display.id.get(), job);
        }
    }

    /// Failed analyses are removed after the advisory `analysis.error` event:
    /// the display and its runtime entities are released. Closing is
    /// conditional on the display still pointing at the failed job.
    fn close_failed_display(&self, display_id: u64, job: JobId) {
        let Some(lease) = self.app() else {
            return;
        };
        let app = lease.app();
        let id = DisplayId(display_id);
        let Ok(info) = app.display(id) else {
            return;
        };
        let Some(analysis) = analysis_id_for_display(&info) else {
            return;
        };
        if app.runtime().job_for(EntityId::Analysis(analysis)) == Some(job) {
            let _ = app.close_display(id);
        }
    }

    /// Find the opened display whose analysis currently owns `job`.
    fn display_for_job(&self, job: JobId) -> Option<u64> {
        let lease = self.app()?;
        let opened: Vec<(u64, AnalysisId)> = {
            let state = self.lock();
            state
                .opened
                .iter()
                .map(|(id, analysis)| (*id, *analysis))
                .collect()
        };
        let runtime = lease.app().runtime();
        opened.into_iter().find_map(|(id, analysis)| {
            (runtime.job_for(EntityId::Analysis(analysis)) == Some(job)).then_some(id)
        })
    }

    /// Emit a job event, first flushing any pending events for the same job.
    ///
    /// Sending under the bridge lock keeps progress/done ordering for buffered
    /// events and live events. No app call is made while the lock is held.
    fn emit_job(&self, event: AppEvent, job: JobId) {
        let display = self.display_for_job(job);

        let mut state = self.lock();
        if state.ignored.contains(&job) {
            // Remove the marker only when this event is terminal.
            if event_is_terminal(&event) {
                state.ignored.remove(&job);
            }
            return;
        }

        // The display may exist in the app before its `display.open` event has
        // reached this bridge. Buffer until the bridge has sent it.
        let Some(display_id) = display else {
            if state.pending.len() >= MAX_PENDING_EVENTS {
                state.pending.pop_front();
            }
            state.pending.push_back(event);
            return;
        };
        if !state.opened.contains_key(&display_id) {
            if state.pending.len() >= MAX_PENDING_EVENTS {
                state.pending.pop_front();
            }
            state.pending.push_back(event);
            return;
        }

        let mut batch = Vec::new();
        let mut remaining = VecDeque::with_capacity(state.pending.len());
        while let Some(pending) = state.pending.pop_front() {
            if event_job(&pending) == Some(job) {
                batch.push(pending);
            } else {
                remaining.push_back(pending);
            }
        }
        state.pending = remaining;
        batch.push(event);
        let failed = batch
            .iter()
            .any(|pending| matches!(pending, AppEvent::JobFailed { .. }));
        for pending in batch {
            self.send_job_event(&mut state, &pending, display_id);
        }
        drop(state);
        if failed {
            self.close_failed_display(display_id, job);
        }
    }

    fn send_job_event(&self, state: &mut BridgeState, event: &AppEvent, display_id: u64) {
        let value = match event {
            AppEvent::JobProgress {
                job,
                frames_done,
                frames_total,
                eta_s,
            } => {
                let runtime_event = RuntimeEvent::Progress {
                    job: *job,
                    frames_done: *frames_done,
                    frames_total: *frames_total,
                    eta_s: *eta_s,
                };
                if !state
                    .progress
                    .should_forward(&runtime_event, Instant::now())
                {
                    return;
                }
                json!({
                    "type": "analysis.progress",
                    "displayId": display_id,
                    "framesDone": frames_done,
                    "framesTotal": frames_total,
                    "etaSeconds": eta_s,
                })
            }
            AppEvent::JobDone { .. } => json!({
                "type": "analysis.done",
                "displayId": display_id,
            }),
            AppEvent::JobFailed { message, .. } => json!({
                "type": "analysis.error",
                "displayId": display_id,
                "message": message,
            }),
            _ => return,
        };
        self.send_value(value);
    }
}

impl AppEventSink for EventBridge {
    fn emit(&self, event: AppEvent) {
        if let Some(job) = event_job(&event) {
            self.emit_job(event, job);
            return;
        }
        match &event {
            AppEvent::DisplayOpened { display } => {
                self.emit_display_opened(&event, display);
            }
            AppEvent::DisplayUpdated { display } => {
                let analysis = analysis_id_for_display(display);
                let mut state = self.lock();
                self.send_display_event(&event);
                if let (Some(slot), Some(analysis)) =
                    (state.opened.get_mut(&display.id.get()), analysis)
                {
                    *slot = analysis;
                }
            }
            AppEvent::DisplayClosed { id, children } => {
                let mut state = self.lock();
                self.send_display_event(&event);
                state.opened.remove(&id.get());
                for child in children {
                    state.opened.remove(&child.get());
                }
            }
            _ => {}
        }
    }
}

fn event_job(event: &AppEvent) -> Option<JobId> {
    match event {
        AppEvent::JobProgress { job, .. }
        | AppEvent::JobDone { job }
        | AppEvent::JobFailed { job, .. } => Some(*job),
        _ => None,
    }
}

fn event_is_terminal(event: &AppEvent) -> bool {
    matches!(event, AppEvent::JobDone { .. } | AppEvent::JobFailed { .. })
}

fn analysis_id_for_display(display: &DisplayInfo) -> Option<AnalysisId> {
    display
        .state
        .get("analysisId")
        .and_then(Value::as_u64)
        .map(AnalysisId)
}
