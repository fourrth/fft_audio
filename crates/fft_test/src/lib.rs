//! Shared test support for adapter crates.
//!
//! `TestApp` implements [`FftApp`] on top of the in-memory
//! [`MockRuntime`] and a reusable
//! [`DisplayRegistry`]. It records app events and [`FftApp`] calls. `TestHost`
//! owns one `TestApp` and is the only place that mints [`TestLease`]s; a
//! `TestLease` has no minting method of its own.

use std::collections::{BTreeSet, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use fft_app::{
    AnalysisId, AppCapabilities, AppError, AppEvent, AppEventSink, AppInfo, AppLease, AppMode,
    CutDisplayOptions, CutDisplaySettings, CutId, DisplayId, DisplayInfo, DisplayKind,
    DisplayOwner, DisplayRecord, DisplayRegistry, FftApp, LeaseId, PlotDisplayOptions, SignalId,
    SpectrogramDisplayOptions, Subscription,
};
use fft_runtime::{
    CutKind, EntityId, FftRuntime, JobId, MockRuntime, RuntimeError, RuntimeEvent, RuntimeEventSink,
};

#[cfg(test)]
mod tests;

/// One recorded call into [`TestApp`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TestCall {
    Info,
    Capabilities,
    Runtime,
    Displays,
    Display(DisplayId),
    ShowSpectrogram(AnalysisId),
    ShowOrderCut {
        parent: DisplayId,
        cut: CutId,
    },
    ShowLineCut {
        parent: DisplayId,
        cut: CutId,
    },
    ShowPlot,
    ShowSpectrogramPending {
        analysis: AnalysisId,
        signal: Option<SignalId>,
    },
    RegisterUserSpectrogram {
        id: DisplayId,
        analysis: AnalysisId,
        signal: Option<SignalId>,
    },
    ShowOrderCutPending {
        parent: DisplayId,
        cut: CutId,
    },
    ShowLineCutPending {
        parent: DisplayId,
        cut: CutId,
    },
    SetDisplayVisible {
        id: DisplayId,
        visible: bool,
    },
    CloseDisplay(DisplayId),
    Subscribe,
    Unsubscribe,
    WaitAll(Option<Duration>),
    Shutdown,
    UpdateDisplay(DisplayId),
    AbandonDisplay(DisplayId),
}

/// Bridges mock runtime job events into app-level events.
/// Mirrors the app host's job-event contract for adapter tests.
struct TestRuntimeSink {
    registry: Arc<DisplayRegistry>,
}

impl RuntimeEventSink for TestRuntimeSink {
    fn emit(&self, event: RuntimeEvent) {
        let event = match event {
            RuntimeEvent::Progress {
                job,
                frames_done,
                frames_total,
                eta_s,
            } => AppEvent::JobProgress {
                job,
                frames_done,
                frames_total,
                eta_s,
            },
            RuntimeEvent::Done { job } => AppEvent::JobDone { job },
            RuntimeEvent::Failed { job, message } => AppEvent::JobFailed { job, message },
            RuntimeEvent::Cancelled { .. } => return,
        };
        self.registry.publish(event);
    }
}

/// Instrumented in-memory app used by adapter tests.
/// It owns only the reusable registry and a mock runtime.
pub struct TestApp {
    info: AppInfo,
    capabilities: AppCapabilities,
    runtime: Arc<MockRuntime>,
    registry: Arc<DisplayRegistry>,
    closed: AtomicBool,
    events: Arc<Mutex<Vec<AppEvent>>>,
    calls: Mutex<Vec<TestCall>>,
    subscribe_calls: AtomicU64,
}

impl Default for TestApp {
    fn default() -> Self {
        Self::new()
    }
}

impl TestApp {
    /// Auto-completing mock runtime and default headless info/capabilities.
    pub fn new() -> Self {
        Self::with_runtime(Arc::new(MockRuntime::new()))
    }

    /// Mock runtime whose new jobs stay pending until explicitly completed.
    pub fn held() -> Self {
        Self::with_runtime(Arc::new(MockRuntime::new_held()))
    }

    pub fn with_runtime(runtime: Arc<MockRuntime>) -> Self {
        let info = AppInfo {
            name: "fft_audio-test".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            mode: AppMode::Headless,
        };
        let capabilities = AppCapabilities::for_build();
        let registry = Arc::new(DisplayRegistry::new(None));
        let events = Arc::new(Mutex::new(Vec::new()));
        let target = Arc::clone(&events);
        registry.subscribe(Arc::new(move |event: AppEvent| {
            target
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(event);
        }));
        runtime.set_event_sink(Arc::new(TestRuntimeSink {
            registry: Arc::clone(&registry),
        }));
        Self {
            info,
            capabilities,
            runtime,
            registry,
            closed: AtomicBool::new(false),
            events,
            calls: Mutex::new(Vec::new()),
            subscribe_calls: AtomicU64::new(0),
        }
    }

    /// Override the info returned by [`FftApp::info`].
    pub fn with_info(mut self, info: AppInfo) -> Self {
        self.info = info;
        self
    }

    /// Override the capabilities returned by [`FftApp::capabilities`].
    pub fn with_capabilities(mut self, capabilities: AppCapabilities) -> Self {
        self.capabilities = capabilities;
        self
    }

    /// The underlying mock runtime, for seeding data and injecting failures.
    pub fn runtime_handle(&self) -> &Arc<MockRuntime> {
        &self.runtime
    }

    /// Access the production display registry used by this test app.
    pub fn registry(&self) -> &DisplayRegistry {
        self.registry.as_ref()
    }

    pub fn is_shutdown(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// Recorded app events, in emission order.
    pub fn events(&self) -> Vec<AppEvent> {
        self.events
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub fn take_events(&self) -> Vec<AppEvent> {
        std::mem::take(
            &mut *self
                .events
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
        )
    }

    pub fn clear_events(&self) {
        self.events
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clear();
    }

    pub fn event_count(&self) -> usize {
        self.events
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .len()
    }

    pub fn display_open_count(&self) -> usize {
        self.events()
            .iter()
            .filter(|event| matches!(event, AppEvent::DisplayOpened { .. }))
            .count()
    }

    pub fn display_update_count(&self) -> usize {
        self.events()
            .iter()
            .filter(|event| matches!(event, AppEvent::DisplayUpdated { .. }))
            .count()
    }

    pub fn display_close_count(&self) -> usize {
        self.events()
            .iter()
            .filter(|event| matches!(event, AppEvent::DisplayClosed { .. }))
            .count()
    }

    /// Recorded [`FftApp`] calls, in call order.
    pub fn calls(&self) -> Vec<TestCall> {
        self.calls
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub fn take_calls(&self) -> Vec<TestCall> {
        std::mem::take(&mut *self.calls.lock().unwrap_or_else(|error| error.into_inner()))
    }

    pub fn clear_calls(&self) {
        self.calls
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clear();
    }

    pub fn call_count(&self) -> usize {
        self.calls
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .len()
    }

    /// Total sinks, including the internal event recorder.
    pub fn subscription_count(&self) -> usize {
        self.registry.subscription_count()
    }

    /// Sinks added by tests (excludes the internal event recorder).
    pub fn user_subscription_count(&self) -> usize {
        self.subscription_count().saturating_sub(1)
    }

    pub fn subscribe_calls(&self) -> u64 {
        self.subscribe_calls.load(Ordering::SeqCst)
    }

    pub fn unsubscribe(&self, subscription: Subscription) -> bool {
        self.record(TestCall::Unsubscribe);
        self.registry.unsubscribe(subscription)
    }

    /// Drop a display record without publishing `DisplayClosed`.
    pub fn abandon_display(&self, id: DisplayId) -> Option<DisplayRecord> {
        self.record(TestCall::AbandonDisplay(id));
        self.registry.remove_silent(id)
    }

    /// Inherent display update (not part of [`FftApp`]).
    pub fn update_display(
        &self,
        id: DisplayId,
        state: serde_json::Value,
        visible: bool,
    ) -> Result<DisplayInfo, AppError> {
        self.record(TestCall::UpdateDisplay(id));
        self.ensure_open()?;
        self.registry.update(id, state, visible)
    }

    /// Make the next analysis fail at submission time.
    pub fn fail_next_analysis(&self, message: impl Into<String>) {
        self.runtime.fail_next_analysis(message);
    }

    /// Make the next order/line cut fail at submission time.
    pub fn fail_next_cut(&self, message: impl Into<String>) {
        self.runtime.fail_next_cut(message);
    }

    /// Keep future jobs pending until completed/failed/cancelled.
    pub fn set_auto_complete(&self, enabled: bool) {
        self.runtime.set_auto_complete(enabled);
    }

    pub fn complete_job(&self, job: JobId) -> Result<(), RuntimeError> {
        self.runtime.complete(job)
    }

    pub fn fail_job(&self, job: JobId, message: impl Into<String>) -> Result<(), RuntimeError> {
        self.runtime.fail(job, message)
    }

    fn record(&self, call: TestCall) {
        self.calls
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(call);
    }

    fn ensure_open(&self) -> Result<(), AppError> {
        if self.is_shutdown() {
            Err(AppError::closed("app has been shut down"))
        } else {
            Ok(())
        }
    }

    fn ready_spectrogram_name(
        &self,
        analysis: AnalysisId,
        options: &SpectrogramDisplayOptions,
    ) -> Result<String, AppError> {
        let data = self
            .runtime
            .spectrogram_data(analysis)
            .map_err(AppError::from_runtime)?;
        let caller = options.name.as_deref().filter(|name| !name.is_empty());
        Ok(caller
            .map(str::to_string)
            .or_else(|| (!data.name.is_empty()).then(|| data.name.clone()))
            .unwrap_or_else(|| "Spectrogram".to_string()))
    }

    fn cut_parent_analysis(&self, parent: DisplayId) -> Result<AnalysisId, AppError> {
        let record = self.registry.record(parent).ok_or_else(|| {
            AppError::not_found(format!("parent display {} not found", parent.get()))
        })?;
        if record.info.kind != DisplayKind::Spectrogram {
            return Err(AppError::invalid_input(
                "cut displays need a spectrogram parent display",
            ));
        }
        record.analysis.ok_or_else(|| {
            AppError::invalid_input(format!("parent display {} has no analysis", parent.get()))
        })
    }

    fn validate_cut_kind(&self, cut: CutId, expected: CutKind) -> Result<(), AppError> {
        match self.runtime.cut_data(cut) {
            Ok(data) if data.kind() != expected => Err(AppError::invalid_input(format!(
                "cut {} is a {:?} cut, not {:?}",
                cut.get(),
                data.kind(),
                expected
            ))),
            Ok(_) => Ok(()),
            Err(error) => match error.code() {
                "busy" => Err(AppError::busy(format!(
                    "cut {} is not ready yet",
                    cut.get()
                ))),
                "not_found" => Err(AppError::not_found(format!("cut {} not found", cut.get()))),
                _ => Err(AppError::from_runtime(error)),
            },
        }
    }

    fn show_spectrogram_inner(
        &self,
        analysis: AnalysisId,
        signal: Option<SignalId>,
        options: &SpectrogramDisplayOptions,
    ) -> Result<DisplayId, AppError> {
        self.ensure_open()?;
        let name = self.ready_spectrogram_name(analysis, options)?;
        let mut options = options.clone();
        options.name = Some(name);
        let ephemeral = options.ephemeral;
        self.registry.insert_spectrogram(
            analysis,
            signal,
            &options,
            DisplayOwner::Backend,
            ephemeral,
        )
    }

    fn show_cut_inner(
        &self,
        parent: DisplayId,
        cut: CutId,
        options: &CutDisplayOptions,
        expected: CutKind,
        order: bool,
    ) -> Result<DisplayId, AppError> {
        self.ensure_open()?;
        let analysis = self.cut_parent_analysis(parent)?;
        match (&options.settings, order) {
            (CutDisplaySettings::Order(_), true) | (CutDisplaySettings::Line(_), false) => {}
            _ => {
                return Err(AppError::invalid_input(if order {
                    "show_order_cut requires order-cut display settings"
                } else {
                    "show_line_cut requires line-cut display settings"
                }))
            }
        }
        self.validate_cut_kind(cut, expected)?;
        let ephemeral = options.ephemeral;
        if order {
            self.registry.insert_order_cut(
                parent,
                cut,
                analysis,
                options,
                DisplayOwner::Backend,
                ephemeral,
            )
        } else {
            self.registry.insert_line_cut(
                parent,
                cut,
                analysis,
                options,
                DisplayOwner::Backend,
                ephemeral,
            )
        }
    }

    fn show_cut_pending_inner(
        &self,
        parent: DisplayId,
        cut: CutId,
        options: &CutDisplayOptions,
        expected: CutKind,
        order: bool,
    ) -> Result<DisplayId, AppError> {
        self.ensure_open()?;
        let analysis = self.cut_parent_analysis(parent)?;
        match (&options.settings, order) {
            (CutDisplaySettings::Order(_), true) | (CutDisplaySettings::Line(_), false) => {}
            _ => {
                return Err(AppError::invalid_input(if order {
                    "show_order_cut_pending requires order-cut display settings"
                } else {
                    "show_line_cut_pending requires line-cut display settings"
                }))
            }
        }
        if self.runtime.job_for(EntityId::Cut(cut)).is_none() {
            return Err(AppError::not_found(format!("cut {} not found", cut.get())));
        }
        if let Some(info) = self.runtime.cut_info(cut) {
            if info.analysis != analysis {
                return Err(AppError::invalid_input(format!(
                    "cut {} belongs to analysis {}, not display parent analysis {}",
                    cut.get(),
                    info.analysis.get(),
                    analysis.get()
                )));
            }
        }
        if let Ok(data) = self.runtime.cut_data(cut) {
            if data.kind() != expected {
                return Err(AppError::invalid_input(format!(
                    "cut {} is a {:?} cut, not {:?}",
                    cut.get(),
                    data.kind(),
                    expected
                )));
            }
        }
        let ephemeral = options.ephemeral;
        if order {
            self.registry.insert_order_cut(
                parent,
                cut,
                analysis,
                options,
                DisplayOwner::Backend,
                ephemeral,
            )
        } else {
            self.registry.insert_line_cut(
                parent,
                cut,
                analysis,
                options,
                DisplayOwner::Backend,
                ephemeral,
            )
        }
    }

    fn close_display_cascade(&self, id: DisplayId) -> Result<(), AppError> {
        let close = self.registry.close(id);
        if close.removed.is_empty() {
            return Ok(());
        }

        let remaining = self.registry.records();
        let remaining_cuts: BTreeSet<CutId> =
            remaining.iter().filter_map(|record| record.cut).collect();
        let remaining_analyses: BTreeSet<AnalysisId> = remaining
            .iter()
            .filter_map(|record| record.analysis)
            .collect();
        let remaining_signals: BTreeSet<SignalId> = remaining
            .iter()
            .filter_map(|record| record.signal)
            .collect();

        let mut cuts = BTreeSet::new();
        let mut analyses = BTreeSet::new();
        let mut signals = BTreeSet::new();
        for record in &close.removed {
            if let Some(cut) = record.cut {
                cuts.insert(cut);
            }
            if record.info.kind == DisplayKind::Spectrogram {
                if let Some(analysis) = record.analysis {
                    analyses.insert(analysis);
                }
                if let Some(signal) = record.signal {
                    signals.insert(signal);
                }
            }
        }

        let mut first_error = None;
        for cut in cuts {
            if !remaining_cuts.contains(&cut) {
                record_close(&mut first_error, self.runtime.close(EntityId::Cut(cut)));
            }
        }
        for analysis in analyses {
            if !remaining_analyses.contains(&analysis) {
                record_close(
                    &mut first_error,
                    self.runtime.close(EntityId::Analysis(analysis)),
                );
            }
        }
        for signal in signals {
            let analyses_for_signal: BTreeSet<AnalysisId> = close
                .removed
                .iter()
                .filter(|record| record.signal == Some(signal))
                .filter_map(|record| record.analysis)
                .collect();
            let still_referenced = remaining_signals.contains(&signal)
                || analyses_for_signal
                    .iter()
                    .any(|analysis| remaining_analyses.contains(analysis));
            if !still_referenced {
                record_close(
                    &mut first_error,
                    self.runtime.close(EntityId::Signal(signal)),
                );
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn wait_all_inner(&self, timeout: Option<Duration>) -> Result<(), AppError> {
        self.ensure_open()?;
        if fft_runtime::is_worker_thread() {
            return Err(AppError::internal(
                "wait_all cannot block on a runtime worker thread",
            ));
        }
        let entities: Vec<EntityId> = {
            let mut entities = Vec::new();
            for record in self.registry.records() {
                if let Some(analysis) = record.analysis {
                    entities.push(EntityId::Analysis(analysis));
                }
                if let Some(cut) = record.cut {
                    entities.push(EntityId::Cut(cut));
                }
            }
            entities
        };
        let mut jobs: Vec<JobId> = entities
            .into_iter()
            .filter_map(|entity| self.runtime.job_for(entity))
            .collect();
        jobs.sort();
        jobs.dedup();

        let deadline = timeout.map(|timeout| Instant::now() + timeout);
        for job in jobs {
            let remaining = match deadline {
                Some(deadline) => {
                    let now = Instant::now();
                    if now >= deadline {
                        return Err(AppError::internal("timed out waiting for jobs"));
                    }
                    Some(deadline - now)
                }
                None => None,
            };
            self.runtime
                .wait(job, remaining)
                .map_err(AppError::from_runtime)?;
        }
        Ok(())
    }
}

fn record_close(first_error: &mut Option<AppError>, result: Result<(), RuntimeError>) {
    if let Err(error) = result {
        if first_error.is_none() {
            *first_error = Some(AppError::from_runtime(error));
        }
    }
}

impl FftApp for TestApp {
    fn info(&self) -> AppInfo {
        self.record(TestCall::Info);
        self.info.clone()
    }

    fn capabilities(&self) -> AppCapabilities {
        self.record(TestCall::Capabilities);
        self.capabilities.clone()
    }

    fn runtime(&self) -> Arc<dyn FftRuntime> {
        self.record(TestCall::Runtime);
        Arc::clone(&self.runtime) as Arc<dyn FftRuntime>
    }

    fn displays(&self) -> Result<Vec<DisplayInfo>, AppError> {
        self.record(TestCall::Displays);
        Ok(self.registry.list())
    }

    fn display(&self, id: DisplayId) -> Result<DisplayInfo, AppError> {
        self.record(TestCall::Display(id));
        self.registry
            .get(id)
            .ok_or_else(|| AppError::not_found(format!("display {} not found", id.get())))
    }

    fn show_spectrogram(
        &self,
        analysis: AnalysisId,
        options: &SpectrogramDisplayOptions,
    ) -> Result<DisplayId, AppError> {
        self.record(TestCall::ShowSpectrogram(analysis));
        self.show_spectrogram_inner(analysis, None, options)
    }

    fn show_order_cut(
        &self,
        parent: DisplayId,
        cut: CutId,
        options: &CutDisplayOptions,
    ) -> Result<DisplayId, AppError> {
        self.record(TestCall::ShowOrderCut { parent, cut });
        self.show_cut_inner(parent, cut, options, CutKind::Order, true)
    }

    fn show_line_cut(
        &self,
        parent: DisplayId,
        cut: CutId,
        options: &CutDisplayOptions,
    ) -> Result<DisplayId, AppError> {
        self.record(TestCall::ShowLineCut { parent, cut });
        self.show_cut_inner(parent, cut, options, CutKind::Line, false)
    }

    fn show_plot(&self, options: &PlotDisplayOptions) -> Result<DisplayId, AppError> {
        self.record(TestCall::ShowPlot);
        self.ensure_open()?;
        self.registry
            .insert_plot(options, DisplayOwner::Backend, options.ephemeral)
    }

    fn show_spectrogram_pending(
        &self,
        analysis: AnalysisId,
        signal: Option<SignalId>,
        options: &SpectrogramDisplayOptions,
    ) -> Result<DisplayId, AppError> {
        self.record(TestCall::ShowSpectrogramPending { analysis, signal });
        self.ensure_open()?;
        if self.runtime.job_for(EntityId::Analysis(analysis)).is_none() {
            return Err(AppError::not_found(format!(
                "analysis {} not found",
                analysis.get()
            )));
        }
        let mut options = options.clone();
        if options.name.as_deref().is_none_or(str::is_empty) {
            options.name = Some(
                self.runtime
                    .analysis_info(analysis)
                    .map(|info| info.name)
                    .unwrap_or_else(|| "Spectrogram".to_string()),
            );
        }
        let ephemeral = options.ephemeral;
        self.registry.insert_spectrogram(
            analysis,
            signal,
            &options,
            DisplayOwner::Backend,
            ephemeral,
        )
    }

    fn register_user_spectrogram_with_id(
        &self,
        id: DisplayId,
        analysis: AnalysisId,
        signal: Option<SignalId>,
        options: &SpectrogramDisplayOptions,
    ) -> Result<DisplayId, AppError> {
        self.record(TestCall::RegisterUserSpectrogram {
            id,
            analysis,
            signal,
        });
        self.ensure_open()?;
        if self.runtime.job_for(EntityId::Analysis(analysis)).is_none() {
            return Err(AppError::not_found(format!(
                "analysis {} not found",
                analysis.get()
            )));
        }
        let mut options = options.clone();
        if options.name.as_deref().is_none_or(str::is_empty) {
            options.name = Some("Spectrogram".to_string());
        }
        let ephemeral = options.ephemeral;
        self.registry.insert_spectrogram_with_id(
            id,
            analysis,
            signal,
            &options,
            DisplayOwner::User,
            ephemeral,
        )
    }

    fn show_order_cut_pending(
        &self,
        parent: DisplayId,
        cut: CutId,
        options: &CutDisplayOptions,
    ) -> Result<DisplayId, AppError> {
        self.record(TestCall::ShowOrderCutPending { parent, cut });
        self.show_cut_pending_inner(parent, cut, options, CutKind::Order, true)
    }

    fn show_line_cut_pending(
        &self,
        parent: DisplayId,
        cut: CutId,
        options: &CutDisplayOptions,
    ) -> Result<DisplayId, AppError> {
        self.record(TestCall::ShowLineCutPending { parent, cut });
        self.show_cut_pending_inner(parent, cut, options, CutKind::Line, false)
    }

    fn set_display_visible(&self, id: DisplayId, visible: bool) -> Result<DisplayInfo, AppError> {
        self.record(TestCall::SetDisplayVisible { id, visible });
        self.ensure_open()?;
        let record = self
            .registry
            .record(id)
            .ok_or_else(|| AppError::not_found(format!("display {} not found", id.get())))?;
        self.registry.update(id, record.info.state, visible)
    }

    fn close_display(&self, id: DisplayId) -> Result<(), AppError> {
        self.record(TestCall::CloseDisplay(id));
        self.close_display_cascade(id)
    }

    fn subscribe(&self, sink: Arc<dyn AppEventSink>) -> Subscription {
        self.subscribe_calls.fetch_add(1, Ordering::SeqCst);
        self.record(TestCall::Subscribe);
        self.registry.subscribe(sink)
    }

    fn wait_all(&self, timeout: Option<Duration>) -> Result<(), AppError> {
        self.record(TestCall::WaitAll(timeout));
        self.wait_all_inner(timeout)
    }

    fn shutdown(&self) {
        self.record(TestCall::Shutdown);
        if !self.closed.swap(true, Ordering::SeqCst) {
            self.registry.clear_sinks();
        }
    }
}

#[derive(Default)]
struct LeaseState {
    active: HashSet<LeaseId>,
    created: Vec<LeaseId>,
    dropped: Vec<LeaseId>,
}

#[derive(Default)]
struct LeaseTracker {
    state: Mutex<LeaseState>,
}

impl LeaseTracker {
    fn created(&self, id: LeaseId) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.active.insert(id);
        state.created.push(id);
    }

    fn dropped(&self, id: LeaseId) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.active.remove(&id);
        state.dropped.push(id);
    }

    fn active_ids(&self) -> Vec<LeaseId> {
        let mut ids: Vec<LeaseId> = self
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .active
            .iter()
            .copied()
            .collect();
        ids.sort_by_key(|id| id.0);
        ids
    }

    fn created_ids(&self) -> Vec<LeaseId> {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .created
            .clone()
    }

    fn dropped_ids(&self) -> Vec<LeaseId> {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .dropped
            .clone()
    }

    fn is_active(&self, id: LeaseId) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .active
            .contains(&id)
    }
}

/// Owner-side test host: holds one app and is the only lease minting path.
pub struct TestHost {
    app: Arc<TestApp>,
    next_lease: AtomicU64,
    leases: Arc<LeaseTracker>,
}

impl Default for TestHost {
    fn default() -> Self {
        Self::new()
    }
}

impl TestHost {
    pub fn new() -> Self {
        Self::with_app(TestApp::new())
    }

    pub fn held() -> Self {
        Self::with_app(TestApp::held())
    }

    pub fn with_app(app: TestApp) -> Self {
        Self::from_arc(Arc::new(app))
    }

    pub fn from_arc(app: Arc<TestApp>) -> Self {
        Self {
            app,
            next_lease: AtomicU64::new(1),
            leases: Arc::new(LeaseTracker::default()),
        }
    }

    pub fn app(&self) -> &TestApp {
        &self.app
    }

    pub fn app_arc(&self) -> Arc<TestApp> {
        Arc::clone(&self.app)
    }

    /// Mint the next lease. This is the only minting path.
    pub fn lease(&self) -> Arc<dyn AppLease> {
        let id = LeaseId(self.next_lease.fetch_add(1, Ordering::SeqCst));
        self.leases.created(id);
        Arc::new(TestLease {
            app: Arc::clone(&self.app),
            id,
            tracker: Arc::clone(&self.leases),
        })
    }

    pub fn active_leases(&self) -> usize {
        self.leases.active_ids().len()
    }

    pub fn active_lease_ids(&self) -> Vec<LeaseId> {
        self.leases.active_ids()
    }

    pub fn created_lease_ids(&self) -> Vec<LeaseId> {
        self.leases.created_ids()
    }

    pub fn dropped_lease_ids(&self) -> Vec<LeaseId> {
        self.leases.dropped_ids()
    }

    pub fn is_lease_active(&self, id: LeaseId) -> bool {
        self.leases.is_active(id)
    }

    pub fn assert_active_leases(&self, expected: usize) {
        let active = self.active_leases();
        assert_eq!(
            active,
            expected,
            "expected {expected} active leases, found {active} ({:?})",
            self.active_lease_ids()
        );
    }

    pub fn assert_lease_dropped(&self, id: LeaseId) {
        assert!(!self.is_lease_active(id), "lease {id:?} is still active");
        assert!(
            self.dropped_lease_ids().contains(&id),
            "lease {id:?} was never created or never dropped"
        );
    }

    pub fn assert_all_leases_dropped(&self) {
        self.assert_active_leases(0);
    }
}

/// Lease minted by [`TestHost`]. It cannot mint another lease.
pub struct TestLease {
    app: Arc<TestApp>,
    id: LeaseId,
    tracker: Arc<LeaseTracker>,
}

impl AppLease for TestLease {
    fn lease_id(&self) -> LeaseId {
        self.id
    }

    fn app(&self) -> &dyn FftApp {
        self.app.as_ref()
    }

    fn app_arc(&self) -> Arc<dyn FftApp> {
        Arc::clone(&self.app) as Arc<dyn FftApp>
    }
}

impl Drop for TestLease {
    fn drop(&mut self) {
        self.tracker.dropped(self.id);
    }
}

/// Convenience: an app plus one lease over a fresh auto-completing runtime.
pub fn test_app_and_lease() -> (Arc<MockRuntime>, Arc<dyn AppLease>) {
    let host = TestHost::new();
    let runtime = Arc::clone(host.app().runtime_handle());
    (runtime, host.lease())
}
