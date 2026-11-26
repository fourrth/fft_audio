//! Core-owned data/compute runtime contract.
//!
//! `fft_runtime` owns signals, analyses, cuts, jobs, and the bounded worker
//! scheduler. Adapters and `fft_app` interact through [`FftRuntime`]; they must
//! not keep duplicate job/analysis maps.
//!
//! Design rules:
//! - submission is non-blocking; materialization is explicit (`wait`/`*_data`);
//! - IDs are typed newtypes, never raw `u64` in public signatures;
//! - returned data is `Arc`-backed so language bindings can expose read-only
//!   zero-copy views;
//! - errors are typed and carry a stable [`RuntimeError::code`].
//!
//! The trait intentionally has no generic methods and no `async fn`, so it is
//! object-safe (`dyn FftRuntime`).

pub mod data;
pub mod error;
pub mod events;
pub mod ids;
pub mod runtime;
mod scheduler;
mod state;
pub mod testing;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use fft_core::settings::AnalysisSettings;
use fft_core::spectrogram::{RenderOptions, SpectrogramGrid};

pub use data::{
    AnalysisInfo, CutData, CutInfo, CutKind, LineCutData, LineCutRequest, LoadFileRequest,
    LoadSignalRequest, OrderCutData, OrderCutRequest, RpmPoint, RpmProfileSpec, SignalInfo,
    SpectrogramData,
};
pub use error::RuntimeError;
pub use events::{JobStatus, ProgressCoalescer, RuntimeEvent, RuntimeEventSink};
pub use ids::{AnalysisId, CutId, DisplayId, EntityId, JobId, ScriptRunId, SignalId};
pub use runtime::{default_worker_count, is_worker_thread, CoreRuntime};
pub use state::SignalData;
pub use testing::MockRuntime;

/// Data/compute service owned by the core runtime.
///
/// ## Jobs vs. data handles
///
/// `analyze`, `order_cut`, and `line_cut` return data handles (`AnalysisId`,
/// `CutId`) immediately. Their progress/cancellation is addressed by a
/// [`JobId`]. [`FftRuntime::job_for`] is the typed bridge between the two:
/// given the entity that was just submitted, it returns the job most recently
/// submitted for it. This keeps job lifetime and data lifetime independent
/// without exposing raw `u64`s or a second, redundant handle type.
///
/// `job_for` keeps returning the job after it completes (so status/error can
/// still be read). It returns `None` once the entity is closed or the runtime
/// has forgotten it.
pub trait FftRuntime: Send + Sync {
    fn load_file(&self, request: LoadFileRequest) -> std::result::Result<SignalId, RuntimeError>;

    fn load_signal(
        &self,
        request: LoadSignalRequest,
    ) -> std::result::Result<SignalId, RuntimeError>;

    fn analyze(
        &self,
        signal: SignalId,
        settings: &AnalysisSettings,
    ) -> std::result::Result<AnalysisId, RuntimeError>;

    /// Typed lookup from a data entity to the job most recently submitted for
    /// it. See the trait-level notes.
    fn job_for(&self, id: EntityId) -> Option<JobId>;

    /// Current status. Unknown/forgotten jobs report [`JobStatus::Closed`].
    fn job_status(&self, job: JobId) -> JobStatus;

    /// Block until `job` reaches a terminal state.
    ///
    /// `None` waits indefinitely. A timeout that expires first returns
    /// [`RuntimeError::busy`]; cancellation/failure return their own category.
    fn wait(&self, job: JobId, timeout: Option<Duration>) -> std::result::Result<(), RuntimeError>;

    /// Request cooperative cancellation. Cancelling an already terminal job is
    /// a no-op; unknown jobs are [`RuntimeError::NotFound`].
    fn cancel(&self, job: JobId) -> std::result::Result<(), RuntimeError>;

    /// Downsampled transport grid for an analysis. The analysis must be ready.
    fn spectrogram_grid(
        &self,
        id: AnalysisId,
        max_frames: usize,
        max_bins: usize,
    ) -> std::result::Result<SpectrogramGrid, RuntimeError>;

    /// Full-resolution, `Arc`-backed spectrogram snapshot.
    fn spectrogram_data(
        &self,
        id: AnalysisId,
    ) -> std::result::Result<Arc<SpectrogramData>, RuntimeError>;

    fn order_cut(
        &self,
        id: AnalysisId,
        request: &OrderCutRequest,
    ) -> std::result::Result<CutId, RuntimeError>;

    fn line_cut(
        &self,
        id: AnalysisId,
        request: &LineCutRequest,
    ) -> std::result::Result<CutId, RuntimeError>;

    fn cut_data(&self, id: CutId) -> std::result::Result<Arc<CutData>, RuntimeError>;

    /// Typed summary of a loaded signal. Pending/failed loads use the same
    /// Busy/Failed/Closed mapping as [`FftRuntime::spectrogram_data`].
    fn signal_info(&self, id: SignalId) -> std::result::Result<SignalInfo, RuntimeError>;

    /// Typed summary of a completed analysis.
    fn analysis_info(&self, id: AnalysisId) -> std::result::Result<AnalysisInfo, RuntimeError>;

    /// Typed summary of a cut result.
    fn cut_info(&self, id: CutId) -> std::result::Result<CutInfo, RuntimeError>;

    /// Transfer ownership of a cut result out of the runtime store.
    ///
    /// The cut must be ready and uniquely owned by the store. On success the
    /// cut and its job are forgotten: `cut_data`/`cut_info`/`job_for` no longer
    /// resolve it, so no canonical alias remains. If another `Arc` view is
    /// still alive the call returns [`RuntimeError::busy`] and leaves the cut
    /// in place; there is no copy fallback. Language bindings use this for
    /// ownership-transferring NumPy arrays (`into_numpy`).
    fn take_cut_data(&self, id: CutId) -> std::result::Result<CutData, RuntimeError>;

    /// Block until every submitted job reaches a terminal state.
    ///
    /// `None` waits indefinitely. A timeout that expires first returns
    /// [`RuntimeError::busy`]. Worker-thread semantics are runtime-specific;
    /// `CoreRuntime` returns Busy instead of deadlocking.
    fn wait_all(&self, timeout: Option<Duration>) -> std::result::Result<(), RuntimeError>;

    fn export_csv(&self, id: CutId, path: &Path) -> std::result::Result<(), RuntimeError>;

    fn save_image(
        &self,
        id: AnalysisId,
        path: &Path,
        options: &RenderOptions,
    ) -> std::result::Result<(), RuntimeError>;

    /// Close a signal, analysis, or cut. Closing an analysis also closes cuts
    /// derived from it; closing a signal cascades through its analyses. Closing
    /// an already-closed/unknown entity is a successful no-op so transport
    /// close paths are idempotent.
    fn close(&self, id: EntityId) -> std::result::Result<(), RuntimeError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_object_safe(_: &dyn FftRuntime) {}
    fn assert_send_sync<T: Send + Sync + ?Sized>() {}

    #[test]
    fn trait_is_object_safe_and_send_sync() {
        assert_send_sync::<dyn FftRuntime>();
        let runtime = MockRuntime::new();
        assert_object_safe(&runtime);
        let boxed: Arc<dyn FftRuntime> = Arc::new(runtime);
        assert_eq!(boxed.job_status(JobId(99)), JobStatus::Closed);
    }
}
