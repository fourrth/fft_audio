//! Transport-independent command core for the Tauri IPC adapter.
//!
//! Every method borrows the one lease held by the GUI adapter. Commands never
//! mint leases. Host-specific display operations that the `FftApp` contract
//! does not expose (registering a caller-owned display id, abandoning a failed
//! display, typed records) go through [`DisplayHostOps`].

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use fft_app::{
    AppCapabilities, AppError, AppInfo as CoreAppInfo, AppLease, DisplayId, DisplayRecord,
    SpectrogramDisplayOptions, SCRIPTED_DISPLAY_ID_BASE,
};
use fft_app_host::AppHost;
use fft_core::cutsession::{LineCutSession, OrderCutSession};
use fft_core::orders::{Cut, LineSpec, RpmProfile};
use fft_core::settings::{AnalysisSettings, LineCutSettings, OrderCutSettings, RpmSource};
use fft_core::spectrogram::{Image, Spectrogram, SpectrogramGrid};
use fft_runtime::{
    AnalysisId, CutId, EntityId, FftRuntime, JobId, LineCutRequest, LoadFileRequest,
    OrderCutRequest, RpmProfileSpec, SpectrogramData,
};

use crate::dto::{
    AppCapabilitiesInfo, AppInfo, CancelResult, CutsPayload, OpenResult, SpectrogramMeta,
    SpectrogramPayload,
};
use crate::events::TransportSink;
use crate::paths::FilePaths;

/// Host-side display operations used by command cores.
///
/// This is a registry capability, not a lease factory. The product root owns
/// the `AppHost` and passes this trait object separately from the GUI lease.
pub trait DisplayHostOps: Send + Sync {
    fn record(&self, id: DisplayId) -> Option<DisplayRecord>;
    fn records(&self) -> Vec<DisplayRecord>;
    fn register_user_spectrogram_with_id(
        &self,
        id: DisplayId,
        analysis: AnalysisId,
        signal: Option<fft_app::SignalId>,
        options: &SpectrogramDisplayOptions,
    ) -> Result<DisplayId, AppError>;
    fn abandon_display(&self, id: DisplayId) -> Option<DisplayRecord>;
}

impl DisplayHostOps for AppHost {
    fn record(&self, id: DisplayId) -> Option<DisplayRecord> {
        self.registry().record(id)
    }

    fn records(&self) -> Vec<DisplayRecord> {
        self.registry().records()
    }

    fn register_user_spectrogram_with_id(
        &self,
        id: DisplayId,
        analysis: AnalysisId,
        signal: Option<fft_app::SignalId>,
        options: &SpectrogramDisplayOptions,
    ) -> Result<DisplayId, AppError> {
        AppHost::register_user_spectrogram_with_id(self, id, analysis, signal, options)
    }

    fn abandon_display(&self, id: DisplayId) -> Option<DisplayRecord> {
        self.registry().remove_silent(id)
    }
}

pub(crate) struct CommandCore {
    lease: Mutex<Option<Arc<dyn AppLease>>>,
    ops: Arc<dyn DisplayHostOps>,
    transport: Arc<TransportSink>,
    paths: FilePaths,
    closed: AtomicBool,
}

impl CommandCore {
    pub(crate) fn new(
        lease: Arc<dyn AppLease>,
        ops: Arc<dyn DisplayHostOps>,
        transport: Arc<TransportSink>,
        paths: FilePaths,
    ) -> Self {
        Self {
            lease: Mutex::new(Some(lease)),
            ops,
            transport,
            paths,
            closed: AtomicBool::new(false),
        }
    }

    #[cfg(test)]
    pub(crate) fn transport(&self) -> &Arc<TransportSink> {
        &self.transport
    }

    pub(crate) fn paths(&self) -> &FilePaths {
        &self.paths
    }

    /// Borrow the adapter's one lease (cloning an `Arc` shares it, it does not
    /// mint a lease).
    pub(crate) fn lease(&self) -> Result<Arc<dyn AppLease>, String> {
        if self.closed.load(Ordering::SeqCst) {
            return Err("app has been shut down".to_string());
        }
        self.lease
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
            .cloned()
            .ok_or_else(|| "app has been shut down".to_string())
    }

    pub(crate) fn subscribe(&self) {
        if let Ok(lease) = self.lease() {
            let sink: Arc<dyn fft_app::AppEventSink> = self.transport.clone();
            lease.app().subscribe(sink);
        }
    }

    /// Reject new command work. The lease is kept until the translator has
    /// stopped.
    pub(crate) fn begin_shutdown(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }

    /// Drop the GUI-lifetime lease. The host/runtime itself stays owned by the
    /// product and is dropped after this adapter.
    pub(crate) fn drop_lease(&self) {
        self.lease
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
    }

    #[cfg(test)]
    pub(crate) fn app_info(&self) -> Result<AppInfo, String> {
        let lease = self.lease()?;
        Ok(app_info_from(
            lease.app().info(),
            lease.app().capabilities(),
        ))
    }

    pub(crate) fn open_analysis(
        &self,
        display_id: u64,
        path: String,
        settings: AnalysisSettings,
    ) -> Result<OpenResult, String> {
        if display_id == 0 || display_id >= SCRIPTED_DISPLAY_ID_BASE {
            return Err(format!(
                "user display id {display_id} must be between 1 and {}",
                SCRIPTED_DISPLAY_ID_BASE - 1
            ));
        }
        settings
            .validate()
            .map_err(|error| format!("invalid analysis settings: {error}"))?;

        let lease = self.lease()?;
        let app = lease.app();
        let runtime = app.runtime();

        // Remove the old display first so the cancellation cascade cannot look
        // like a failure of the newly submitted analysis.
        self.close_replaced_display(display_id);

        let signal = runtime
            .load_file(LoadFileRequest::new(path.clone()))
            .map_err(|error| error.to_string())?;
        if let Some(job) = runtime.job_for(EntityId::Signal(signal)) {
            self.transport.ignore(job);
        }

        let analysis = match runtime.analyze(signal, &settings) {
            Ok(analysis) => analysis,
            Err(error) => {
                let _ = runtime.close(EntityId::Signal(signal));
                return Err(error.to_string());
            }
        };
        let job = runtime.job_for(EntityId::Analysis(analysis));
        let options = user_display_options(&path);
        if let Err(error) = self.ops.register_user_spectrogram_with_id(
            DisplayId(display_id),
            analysis,
            Some(signal),
            &options,
        ) {
            if let Some(job) = job {
                self.transport.ignore(job);
            }
            let _ = runtime.close(EntityId::Signal(signal));
            return Err(error.to_string());
        }

        // Register after the display record exists: a terminal failure that
        // raced the analysis is flushed now and can clean up the whole display.
        if let Some(job) = job {
            self.transport.register(job, display_id);
        }
        Ok(OpenResult {
            display_id,
            analysis_id: analysis.get(),
        })
    }

    /// Create a backend display for an existing ready analysis.
    pub(crate) fn show_backend_spectrogram(
        &self,
        analysis: AnalysisId,
        ephemeral: bool,
    ) -> Result<fft_app::DisplayInfo, String> {
        let lease = self.lease()?;
        let options = SpectrogramDisplayOptions {
            ephemeral,
            ..SpectrogramDisplayOptions::default()
        };
        let id = lease
            .app()
            .show_spectrogram(analysis, &options)
            .map_err(|error| error.to_string())?;
        lease.app().display(id).map_err(|error| error.to_string())
    }

    pub(crate) fn list_displays(&self) -> Result<Vec<fft_app::DisplayInfo>, String> {
        self.lease()?
            .app()
            .displays()
            .map_err(|error| error.to_string())
    }

    pub(crate) fn get_display(&self, display_id: u64) -> Result<fft_app::DisplayInfo, String> {
        self.lease()?
            .app()
            .display(DisplayId(display_id))
            .map_err(|error| error.to_string())
    }

    /// Close a display subtree. Unknown ids are a successful no-op.
    pub(crate) fn close_display(&self, display_id: u64) -> Result<(), String> {
        let lease = self.lease()?;
        let id = DisplayId(display_id);
        if let Some(record) = self.ops.record(id) {
            self.ignore_display_jobs(&record, lease.app().runtime());
        }
        lease
            .app()
            .close_display(id)
            .map_err(|error| error.to_string())
    }

    fn close_replaced_display(&self, display_id: u64) {
        let _ = self.close_display(display_id);
    }

    pub(crate) fn spectrogram_payload(
        &self,
        display_id: u64,
    ) -> Result<SpectrogramPayload, String> {
        let not_ready = || "the analysis is not ready yet".to_string();
        let lease = self.lease()?;
        let analysis = self
            .ops
            .record(DisplayId(display_id))
            .and_then(|record| record.analysis)
            .ok_or_else(not_ready)?;
        let data = lease
            .app()
            .runtime()
            .spectrogram_data(analysis)
            .map_err(|_| not_ready())?;
        let grid = data
            .spectrogram
            .to_grid(GRID_MAX_FRAMES, grid_bins_budget(data.frames()));
        Ok(payload_from_grid(
            grid,
            data.sample_rate,
            data.analysis.clone(),
            data.table.clone(),
        ))
    }

    /// Full-resolution render used by the save-image command.
    pub(crate) fn render_spectrogram(&self, display_id: u64) -> Result<Image, String> {
        let not_ready = || "no spectrogram to export".to_string();
        let lease = self.lease()?;
        let analysis = self
            .ops
            .record(DisplayId(display_id))
            .and_then(|record| record.analysis)
            .ok_or_else(not_ready)?;
        let data = lease
            .app()
            .runtime()
            .spectrogram_data(analysis)
            .map_err(|_| not_ready())?;
        Ok(data
            .spectrogram
            .render(&fft_core::spectrogram::RenderOptions {
                width: 1600,
                height: 900,
                log_freq: true,
                min_hz: 10.0,
                max_hz: None,
                dynamic_range_db: 80.0,
                db_min: None,
                db_max: None,
            }))
    }

    #[cfg(test)]
    pub(crate) fn order_cuts(
        &self,
        parent_id: u64,
        settings: OrderCutSettings,
        rpm: Option<RpmProfileSpec>,
    ) -> Result<CutsPayload, String> {
        let (analysis, data) = self.ready_parent(parent_id)?;
        self.order_cuts_from_parent(analysis, data, settings, rpm)
    }

    /// Compute order cuts for an already-validated ready parent. Commands call
    /// this from their blocking pool after a readiness-first check.
    pub(crate) fn order_cuts_from_parent(
        &self,
        analysis: AnalysisId,
        data: Arc<SpectrogramData>,
        settings: OrderCutSettings,
        rpm: Option<RpmProfileSpec>,
    ) -> Result<CutsPayload, String> {
        let lease = self.lease()?;
        let runtime = lease.app().runtime();
        let request = OrderCutRequest {
            settings: settings.clone(),
            rpm: rpm.clone(),
        };
        let cut = runtime
            .order_cut(analysis, &request)
            .map_err(|error| error.to_string())?;
        let job = runtime.job_for(EntityId::Cut(cut));
        let _transient = TransientCut::new(Arc::clone(&runtime), cut);
        if let Some(job) = job {
            self.transport.ignore(job);
            runtime.wait(job, None).map_err(|error| error.to_string())?;
        }
        let cuts = order_cut_payload(&data, &settings, rpm.as_ref())?;
        Ok(cuts_payload(&data, cuts))
    }

    #[cfg(test)]
    pub(crate) fn line_cuts(
        &self,
        parent_id: u64,
        settings: LineCutSettings,
        lines: Vec<LineSpec>,
    ) -> Result<CutsPayload, String> {
        let (analysis, data) = self.ready_parent(parent_id)?;
        self.line_cuts_from_parent(analysis, data, settings, lines)
    }

    /// Compute line cuts for an already-validated ready parent.
    pub(crate) fn line_cuts_from_parent(
        &self,
        analysis: AnalysisId,
        data: Arc<SpectrogramData>,
        settings: LineCutSettings,
        lines: Vec<LineSpec>,
    ) -> Result<CutsPayload, String> {
        let lease = self.lease()?;
        let runtime = lease.app().runtime();
        let request = LineCutRequest::new(settings.clone(), lines.clone());
        let cut = runtime
            .line_cut(analysis, &request)
            .map_err(|error| error.to_string())?;
        let job = runtime.job_for(EntityId::Cut(cut));
        let _transient = TransientCut::new(Arc::clone(&runtime), cut);
        if let Some(job) = job {
            self.transport.ignore(job);
            runtime.wait(job, None).map_err(|error| error.to_string())?;
        }
        let mut session = LineCutSession::new(settings);
        session.set_lines(lines);
        session.compute(&data.spectrogram);
        Ok(cuts_payload(&data, session.cuts().to_vec()))
    }

    /// Resolve only ready parent analyses; cut commands never block waiting
    /// for the parent.
    pub(crate) fn ready_parent(
        &self,
        parent_id: u64,
    ) -> Result<(AnalysisId, Arc<SpectrogramData>), String> {
        let not_ready = || "the parent spectrogram is not ready yet".to_string();
        let lease = self.lease()?;
        let analysis = self
            .ops
            .record(DisplayId(parent_id))
            .and_then(|record| record.analysis)
            .ok_or_else(not_ready)?;
        let data = lease
            .app()
            .runtime()
            .spectrogram_data(analysis)
            .map_err(|error| match error.code() {
                "busy" | "not_found" | "closed" | "cancelled" => not_ready(),
                _ => error.to_string(),
            })?;
        Ok((analysis, data))
    }

    /// Cancel only an analysis job that was active before this call.
    pub(crate) fn cancel_analysis(&self, display_id: u64) -> CancelResult {
        let Ok(lease) = self.lease() else {
            return CancelResult { cancelled: false };
        };
        let Some(analysis) = self
            .ops
            .record(DisplayId(display_id))
            .and_then(|record| record.analysis)
        else {
            return CancelResult { cancelled: false };
        };
        let runtime = lease.app().runtime();
        let Some(job) = runtime.job_for(EntityId::Analysis(analysis)) else {
            return CancelResult { cancelled: false };
        };
        if !runtime.job_status(job).is_active() {
            return CancelResult { cancelled: false };
        }
        let cancelled = runtime.cancel(job).is_ok();
        if cancelled {
            // `fft_app` does not expose cancellation as an app event; report
            // an `analysis-error` payload explicitly.
            self.transport.cancelled(job, display_id);
        }
        CancelResult { cancelled }
    }

    /// Remove a failed/cancelled display silently when `job` is still its
    /// current analysis job. Used by the translator's failure hook.
    pub(crate) fn cleanup_failed_display(&self, display_id: u64, job: JobId) {
        let Ok(lease) = self.lease() else {
            return;
        };
        let id = DisplayId(display_id);
        let Some(record) = self.ops.record(id) else {
            return;
        };
        let Some(analysis) = record.analysis else {
            return;
        };
        let runtime = lease.app().runtime();
        if runtime.job_for(EntityId::Analysis(analysis)) != Some(job) {
            return;
        }
        let Some(record) = self.ops.abandon_display(id) else {
            return;
        };
        if let Some(signal) = record.signal {
            if let Some(job) = runtime.job_for(EntityId::Signal(signal)) {
                self.transport.ignore(job);
            }
            let _ = runtime.close(EntityId::Signal(signal));
        } else {
            let _ = runtime.close(EntityId::Analysis(analysis));
        }
    }

    /// Never translate terminal job events for runtime entities that an
    /// explicit display close is about to release. Shared entities keep their
    /// events so a surviving display can still finish/refresh.
    fn ignore_display_jobs(&self, record: &DisplayRecord, runtime: Arc<dyn FftRuntime>) {
        let records = self.ops.records();
        let analysis_shared = record.analysis.is_some_and(|analysis| {
            records
                .iter()
                .any(|other| other.info.id != record.info.id && other.analysis == Some(analysis))
        });
        if !analysis_shared {
            if let Some(analysis) = record.analysis {
                if let Some(job) = runtime.job_for(EntityId::Analysis(analysis)) {
                    self.transport.ignore(job);
                }
            }
        }
        let signal_shared = record.signal.is_some_and(|signal| {
            records.iter().any(|other| {
                other.info.id != record.info.id
                    && (other.signal == Some(signal)
                        || (record.analysis.is_some() && other.analysis == record.analysis))
            })
        });
        if !signal_shared {
            if let Some(signal) = record.signal {
                if let Some(job) = runtime.job_for(EntityId::Signal(signal)) {
                    self.transport.ignore(job);
                }
            }
        }
    }
}

pub(crate) fn app_info_from(info: CoreAppInfo, capabilities: AppCapabilities) -> AppInfo {
    AppInfo {
        name: info.name,
        version: info.version,
        mode: info.mode,
        capabilities: AppCapabilitiesInfo::from_capabilities(capabilities),
    }
}

/// Frontend display ids stay below the reserved backend range.
fn user_display_options(path: &str) -> SpectrogramDisplayOptions {
    let name = Path::new(path)
        .file_stem()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_string);
    SpectrogramDisplayOptions {
        name,
        ..SpectrogramDisplayOptions::default()
    }
}

/// Desktop cut commands return points, not persistent cut handles. Close the
/// transient runtime cut (and its completed job) on every exit path.
struct TransientCut {
    runtime: Arc<dyn FftRuntime>,
    cut: CutId,
}

impl TransientCut {
    fn new(runtime: Arc<dyn FftRuntime>, cut: CutId) -> Self {
        Self { runtime, cut }
    }
}

impl Drop for TransientCut {
    fn drop(&mut self) {
        let _ = self.runtime.close(EntityId::Cut(self.cut));
    }
}

fn order_cut_payload(
    data: &SpectrogramData,
    settings: &OrderCutSettings,
    rpm: Option<&RpmProfileSpec>,
) -> Result<Vec<Cut>, String> {
    let spec = &data.spectrogram;
    let profile = resolve_rpm(settings, rpm, spec)?;
    let mut session = OrderCutSession::new(settings.clone());
    session.compute_with_profile(spec, &profile);
    Ok(session.cuts().to_vec())
}

fn resolve_rpm(
    settings: &OrderCutSettings,
    rpm: Option<&RpmProfileSpec>,
    spec: &Spectrogram,
) -> Result<RpmProfile, String> {
    if let Some(profile) = rpm {
        return match profile {
            RpmProfileSpec::Constant(rpm) => {
                ensure_finite_rpm(*rpm, "RPM")?;
                Ok(RpmProfile::constant(*rpm))
            }
            RpmProfileSpec::Ramp {
                t0_s,
                t1_s,
                rpm0,
                rpm1,
            } => {
                ensure_finite_rpm(*t0_s, "ramp start time")?;
                ensure_finite_rpm(*t1_s, "ramp end time")?;
                ensure_finite_rpm(*rpm0, "ramp start RPM")?;
                ensure_finite_rpm(*rpm1, "ramp end RPM")?;
                Ok(RpmProfile::ramp(*t0_s, *t1_s, *rpm0, *rpm1))
            }
            RpmProfileSpec::Csv(path) => RpmProfile::from_csv(path).map_err(|error| {
                format!("failed to read RPM profile: {error} ({})", path.display())
            }),
            RpmProfileSpec::Points(points) => {
                let points = points
                    .iter()
                    .map(|point| (point.time_s, point.rpm))
                    .collect();
                RpmProfile::from_points(points)
                    .map_err(|error| format!("invalid RPM points: {error}"))
            }
        };
    }

    let rpm = &settings.rpm;
    match rpm.source {
        RpmSource::Constant => {
            ensure_finite_rpm(rpm.constant, "constant RPM")?;
            Ok(RpmProfile::constant(rpm.constant))
        }
        RpmSource::Ramp => {
            ensure_finite_rpm(rpm.start, "ramp start RPM")?;
            ensure_finite_rpm(rpm.end, "ramp end RPM")?;
            Ok(RpmProfile::ramp(0.0, spec.end_s(), rpm.start, rpm.end))
        }
        RpmSource::Csv => {
            let path = rpm
                .csv
                .as_ref()
                .ok_or_else(|| "RPM source is CSV but no file is configured".to_string())?;
            RpmProfile::from_csv(path).map_err(|error| {
                format!("failed to read RPM profile: {error} ({})", path.display())
            })
        }
    }
}

fn ensure_finite_rpm(value: f64, label: &str) -> Result<(), String> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(format!("{label} must be finite"))
    }
}

pub(crate) fn cuts_payload(data: &SpectrogramData, cuts: Vec<Cut>) -> CutsPayload {
    let spectrogram = &data.spectrogram;
    CutsPayload {
        cuts,
        duration_s: spectrogram.end_s(),
        nyquist: spectrogram.nyquist(),
        bin_hz: spectrogram.bin_hz(),
        hop_s: spectrogram.hop_s(),
    }
}

const GRID_MAX_FRAMES: usize = 2048;
/// Cap the transport grid at roughly 8 MiB of raw i16 samples; bins adapt to
/// the frame count so short analyses retain full frequency detail.
const GRID_SAMPLE_BUDGET: usize = 4_000_000;

fn grid_bins_budget(frames: usize) -> usize {
    let out_frames = frames.clamp(1, GRID_MAX_FRAMES);
    (GRID_SAMPLE_BUDGET / out_frames).max(256)
}

fn payload_from_grid(
    grid: SpectrogramGrid,
    sample_rate: f64,
    analysis_settings: AnalysisSettings,
    table: Option<fft_core::table::TableInfo>,
) -> SpectrogramPayload {
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    let mut bytes = Vec::with_capacity(grid.data.len() * 2);
    for value in &grid.data {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    SpectrogramPayload {
        meta: SpectrogramMeta {
            frames: grid.frames,
            bins: grid.bins,
            bin_hz: grid.bin_hz,
            frame_s: grid.frame_s,
            duration_s: grid.duration_s,
            nyquist: grid.nyquist,
            max_db: grid.max_db,
            min_db: grid.min_db,
            sample_rate,
            analysis_settings,
            table,
        },
        data: STANDARD.encode(bytes),
    }
}
