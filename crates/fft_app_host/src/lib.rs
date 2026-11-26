//! Concrete [`FftApp`] host.
//!
//! [`AppHost`] owns the display registry, backend display-id allocation, app
//! lifecycle, and the runtime handle. It is transport-agnostic: adapters only
//! receive an `Arc<dyn AppLease>` minted by [`AppHost::lease`] and use the
//! [`FftApp`] contract.
//!
//! The program composition root owns the runtime for its process lifetime.
//! [`FftApp::shutdown`] stops app work and drops subscriptions; it never shuts
//! the runtime down.

// `serde` is reserved for host-side persisted-session helpers; the host does
// not use it directly yet.
#![allow(unused_crate_dependencies)]

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use fft_app::{
    AnalysisId, AppCapabilities, AppClose, AppError, AppEvent, AppEventSink, AppInfo, AppLease,
    AppMode, CutDisplayOptions, CutDisplaySettings, CutId, DisplayId, DisplayInfo, DisplayKind,
    DisplayOwner, DisplayRecord, DisplayRegistry, FftApp, LeaseId, PlotDisplayOptions, SignalId,
    SpectrogramDisplayOptions, Subscription,
};
use fft_core::session::SessionDocument;
use fft_runtime::{
    CoreRuntime, CutKind, EntityId, FftRuntime, JobId, RuntimeEvent, RuntimeEventSink,
};
use serde_json::Value;

#[cfg(test)]
mod tests;

/// Construction-time configuration for [`AppHost`].
#[derive(Clone, Debug)]
pub struct AppHostConfig {
    pub name: String,
    pub version: String,
    pub mode: AppMode,
    pub capabilities: AppCapabilities,
    /// Worker count for the concrete [`CoreRuntime`] built by [`AppHost::new`].
    pub workers: usize,
    /// Persisted session used only to seed backend display-id allocation.
    pub persisted_session: Option<SessionDocument>,
}

impl Default for AppHostConfig {
    fn default() -> Self {
        Self {
            name: "fft_audio".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            mode: AppMode::Headless,
            capabilities: AppCapabilities::for_build(),
            workers: fft_runtime::default_worker_count(),
            persisted_session: None,
        }
    }
}

/// Concrete app host owning displays, lifecycle, and the runtime handle.
pub struct AppHost {
    info: AppInfo,
    capabilities: AppCapabilities,
    runtime: Arc<dyn FftRuntime>,
    registry: Arc<DisplayRegistry>,
    closed: AtomicBool,
    next_lease: AtomicU64,
}

/// Host-private lease. Adapters only see `Arc<dyn AppLease>`.
///
/// Fields and constructor are private to this crate; [`AppHost::lease`] is the
/// only construction path.
struct AppHostLease {
    app: Arc<AppHost>,
    id: LeaseId,
}

impl AppHostLease {
    fn new(app: Arc<AppHost>, id: LeaseId) -> Self {
        Self { app, id }
    }
}

impl AppLease for AppHostLease {
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

/// Bridges runtime job events into app-level events.
///
/// The registry is held weakly so runtime handles outliving the host cannot
/// resurrect a dropped registry. Job cancellation has no app-level event; like
/// the shared test app, it is dropped.
struct HostRuntimeSink {
    registry: Weak<DisplayRegistry>,
}

impl RuntimeEventSink for HostRuntimeSink {
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
        if let Some(registry) = self.registry.upgrade() {
            registry.publish(event);
        }
    }
}

fn record_close(first_error: &mut Option<AppError>, result: Result<(), fft_runtime::RuntimeError>) {
    if let Err(error) = result {
        if first_error.is_none() {
            *first_error = Some(AppError::from_runtime(error));
        }
    }
}

impl AppHost {
    /// Build a host with a concrete [`CoreRuntime`] and an internal job-event
    /// bridge to app events.
    #[allow(clippy::new_ret_no_self)] // the host is shared by construction
    pub fn new(config: AppHostConfig) -> Arc<Self> {
        let registry = Arc::new(DisplayRegistry::new(config.persisted_session.as_ref()));
        let sink = Arc::new(HostRuntimeSink {
            registry: Arc::downgrade(&registry),
        });
        let runtime: Arc<dyn FftRuntime> = Arc::new(CoreRuntime::new(config.workers, sink));
        Self::from_parts(runtime, config, registry)
    }

    /// Build a host around a supplied runtime.
    ///
    /// Used by tests and embedders. The runtime is not modified or wrapped, so
    /// job events are delivered only if that runtime already publishes to an
    /// app-level bridge.
    pub fn from_runtime(runtime: Arc<dyn FftRuntime>, config: AppHostConfig) -> Arc<Self> {
        let registry = Arc::new(DisplayRegistry::new(config.persisted_session.as_ref()));
        Self::from_parts(runtime, config, registry)
    }

    fn from_parts(
        runtime: Arc<dyn FftRuntime>,
        config: AppHostConfig,
        registry: Arc<DisplayRegistry>,
    ) -> Arc<Self> {
        Arc::new(Self {
            info: AppInfo {
                name: config.name,
                version: config.version,
                mode: config.mode,
            },
            capabilities: config.capabilities,
            runtime,
            registry,
            closed: AtomicBool::new(false),
            next_lease: AtomicU64::new(1),
        })
    }

    /// Mint a scoped lease. This is the only minting path.
    ///
    /// # Panics
    ///
    /// Panics if the host has already been shut down: a closed app must not
    /// hand out new scoped handles.
    pub fn lease(self: &Arc<Self>) -> Arc<dyn AppLease> {
        assert!(
            !self.is_shutdown(),
            "cannot mint a lease from a shut-down app host"
        );
        let id = LeaseId(self.next_lease.fetch_add(1, Ordering::SeqCst));
        Arc::new(AppHostLease::new(Arc::clone(self), id))
    }

    /// Authoritative display registry, useful for typed queries and session
    /// conversion. It does not expose any lease-minting capability.
    pub fn registry(&self) -> &DisplayRegistry {
        self.registry.as_ref()
    }

    pub fn is_shutdown(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
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

    /// Create a spectrogram display while recording the owning signal handle.
    ///
    /// The analysis must already be materialized; otherwise the call fails
    /// before a display id is allocated or an event is published.
    pub fn show_spectrogram_with_signal(
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

    /// Create a spectrogram display under a caller-owned, user-range id.
    ///
    /// Ids must stay below [`fft_app::SCRIPTED_DISPLAY_ID_BASE`]. The analysis
    /// must be ready; [`Self::register_user_spectrogram_with_id`] is the lazy
    /// path for a user-owned pending analysis.
    pub fn show_spectrogram_with_id(
        &self,
        id: DisplayId,
        analysis: AnalysisId,
        signal: Option<SignalId>,
        options: &SpectrogramDisplayOptions,
    ) -> Result<DisplayId, AppError> {
        self.ensure_open()?;
        let name = self.ready_spectrogram_name(analysis, options)?;
        let mut options = options.clone();
        options.name = Some(name);
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

    /// Register a spectrogram display immediately for a pending analysis.
    ///
    /// The analysis job must exist; its data is not queried. This is the
    /// explicit pending path used by backend opens.
    pub fn show_spectrogram_pending(
        &self,
        analysis: AnalysisId,
        signal: Option<SignalId>,
        options: &SpectrogramDisplayOptions,
    ) -> Result<DisplayId, AppError> {
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
                    .unwrap_or_else(|_| "Spectrogram".to_string()),
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

    /// Register a caller-owned user display while its analysis is still being
    /// created by that same caller.
    ///
    /// Unlike the backend pending path, this records the display with
    /// [`DisplayOwner::User`]. The analysis job must exist.
    pub fn register_user_spectrogram_with_id(
        &self,
        id: DisplayId,
        analysis: AnalysisId,
        signal: Option<SignalId>,
        options: &SpectrogramDisplayOptions,
    ) -> Result<DisplayId, AppError> {
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

    /// Register an Order Cuts child display before its cut data is ready.
    pub fn show_order_cut_pending(
        &self,
        parent: DisplayId,
        cut: CutId,
        options: &CutDisplayOptions,
    ) -> Result<DisplayId, AppError> {
        self.show_cut_pending(parent, cut, options, CutKind::Order, true)
    }

    /// Register a Line Cuts child display before its cut data is ready.
    pub fn show_line_cut_pending(
        &self,
        parent: DisplayId,
        cut: CutId,
        options: &CutDisplayOptions,
    ) -> Result<DisplayId, AppError> {
        self.show_cut_pending(parent, cut, options, CutKind::Line, false)
    }

    /// Register a standalone generic Plot display for a table file.
    pub fn show_plot(&self, options: &PlotDisplayOptions) -> Result<DisplayId, AppError> {
        self.ensure_open()?;
        self.registry
            .insert_plot(options, DisplayOwner::Backend, options.ephemeral)
    }

    /// Change one display's visibility without touching its state.
    pub fn set_display_visible(
        &self,
        id: DisplayId,
        visible: bool,
    ) -> Result<DisplayInfo, AppError> {
        self.ensure_open()?;
        let record = self
            .registry
            .record(id)
            .ok_or_else(|| AppError::not_found(format!("display {} not found", id.get())))?;
        self.registry.update(id, record.info.state, visible)
    }

    /// Replace a display's app state and visibility, emitting
    /// `display.updated`.
    ///
    /// This is an inherent app-level command; the [`FftApp`] contract exposes
    /// only visibility changes.
    pub fn update_display(
        &self,
        id: DisplayId,
        state: Value,
        visible: bool,
    ) -> Result<DisplayInfo, AppError> {
        self.ensure_open()?;
        self.registry.update(id, state, visible)
    }

    /// Drop one display record without publishing `display.closed`.
    ///
    /// Used by callers that abandon a failed open and own runtime cleanup.
    pub fn abandon_display(&self, id: DisplayId) -> Option<DisplayRecord> {
        self.registry.remove_silent(id)
    }

    /// Close a display subtree and release only runtime entities that are no
    /// longer referenced by any remaining display.
    ///
    /// The registry lock is released before any event or runtime close call.
    /// Cuts are closed when no surviving display references them; a
    /// spectrogram's analysis and recorded signal are closed only when no
    /// surviving display references them.
    pub fn close_display_cascade(&self, id: DisplayId) -> Result<AppClose, AppError> {
        let close = self.registry.close(id);
        if close.removed.is_empty() {
            return Ok(close);
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
            // The analysis/signal are owned by the root spectrogram display.
            // A child cut records its parent analysis for attach purposes but
            // must not release it when the child alone is closed.
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
            // Closing a signal cascades through its analyses, so it is only
            // safe when neither the signal nor any of its analyses is shared.
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
            None => Ok(close),
        }
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

    /// Validate a materialized cut's kind. Busy/unknown cuts are rejected
    /// before a display id is allocated.
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

    fn show_cut(
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

    fn show_cut_pending(
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
        // When the result is already materialized, reject a mismatched kind or
        // parent analysis before allocating an id. A pending cut cannot be
        // inspected without blocking, so it is accepted and the frontend
        // resolves the data when the job completes.
        if let Ok(info) = self.runtime.cut_info(cut) {
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
                        return Err(AppError::busy("timed out waiting for jobs"));
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

impl FftApp for AppHost {
    fn info(&self) -> AppInfo {
        self.info.clone()
    }

    fn capabilities(&self) -> AppCapabilities {
        self.capabilities.clone()
    }

    fn runtime(&self) -> Arc<dyn FftRuntime> {
        Arc::clone(&self.runtime)
    }

    fn displays(&self) -> Result<Vec<DisplayInfo>, AppError> {
        Ok(self.registry.list())
    }

    fn display(&self, id: DisplayId) -> Result<DisplayInfo, AppError> {
        self.registry
            .get(id)
            .ok_or_else(|| AppError::not_found(format!("display {} not found", id.get())))
    }

    fn show_spectrogram(
        &self,
        analysis: AnalysisId,
        options: &SpectrogramDisplayOptions,
    ) -> Result<DisplayId, AppError> {
        self.show_spectrogram_with_signal(analysis, None, options)
    }

    fn show_order_cut(
        &self,
        parent: DisplayId,
        cut: CutId,
        options: &CutDisplayOptions,
    ) -> Result<DisplayId, AppError> {
        self.show_cut(parent, cut, options, CutKind::Order, true)
    }

    fn show_line_cut(
        &self,
        parent: DisplayId,
        cut: CutId,
        options: &CutDisplayOptions,
    ) -> Result<DisplayId, AppError> {
        self.show_cut(parent, cut, options, CutKind::Line, false)
    }

    fn show_plot(&self, options: &PlotDisplayOptions) -> Result<DisplayId, AppError> {
        AppHost::show_plot(self, options)
    }

    fn show_spectrogram_pending(
        &self,
        analysis: AnalysisId,
        signal: Option<SignalId>,
        options: &SpectrogramDisplayOptions,
    ) -> Result<DisplayId, AppError> {
        AppHost::show_spectrogram_pending(self, analysis, signal, options)
    }

    fn register_user_spectrogram_with_id(
        &self,
        id: DisplayId,
        analysis: AnalysisId,
        signal: Option<SignalId>,
        options: &SpectrogramDisplayOptions,
    ) -> Result<DisplayId, AppError> {
        AppHost::register_user_spectrogram_with_id(self, id, analysis, signal, options)
    }

    fn show_order_cut_pending(
        &self,
        parent: DisplayId,
        cut: CutId,
        options: &CutDisplayOptions,
    ) -> Result<DisplayId, AppError> {
        AppHost::show_order_cut_pending(self, parent, cut, options)
    }

    fn show_line_cut_pending(
        &self,
        parent: DisplayId,
        cut: CutId,
        options: &CutDisplayOptions,
    ) -> Result<DisplayId, AppError> {
        AppHost::show_line_cut_pending(self, parent, cut, options)
    }

    fn set_display_visible(&self, id: DisplayId, visible: bool) -> Result<DisplayInfo, AppError> {
        AppHost::set_display_visible(self, id, visible)
    }

    fn close_display(&self, id: DisplayId) -> Result<(), AppError> {
        self.close_display_cascade(id).map(|_| ())
    }

    fn subscribe(&self, sink: Arc<dyn AppEventSink>) -> Subscription {
        self.registry.subscribe(sink)
    }

    fn wait_all(&self, timeout: Option<Duration>) -> Result<(), AppError> {
        self.wait_all_inner(timeout)
    }

    fn shutdown(&self) {
        if !self.closed.swap(true, Ordering::SeqCst) {
            // The composition root owns the runtime; app shutdown only stops
            // display/job event delivery and rejects new app work.
            self.registry.clear_sinks();
        }
    }
}
