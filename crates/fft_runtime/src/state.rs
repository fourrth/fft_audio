//! Runtime-owned stores for signals, analyses, and cuts.
//!
//! All maps live behind one mutex in [`crate::runtime::CoreRuntime`]. Data is
//! stored as `Arc` snapshots so language bindings and adapters can keep it
//! alive after the entity is closed.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use fft_core::orders::Cut;
use fft_core::table::TableInfo;

use crate::data::{CutData, CutInfo, CutKind as RuntimeCutKind, SignalInfo, SpectrogramData};
use crate::error::RuntimeError;
use crate::events::JobStatus;
use crate::ids::{AnalysisId, CutId, EntityId, JobId, SignalId};
use crate::scheduler::{JobOutput, SchedulerState};

/// Decoded signal samples plus the metadata every analysis shares.
#[derive(Clone, Debug)]
pub struct SignalData {
    pub samples: Arc<Vec<f32>>,
    pub sample_rate: f64,
    pub name: String,
    /// `None` for in-memory signals.
    pub path: Option<PathBuf>,
    /// Present when the source was a data table.
    pub table: Option<TableInfo>,
}

impl SignalData {
    pub fn sample_count(&self) -> usize {
        self.samples.len()
    }

    pub fn duration_s(&self) -> f64 {
        if self.sample_rate > 0.0 {
            self.samples.len() as f64 / self.sample_rate
        } else {
            0.0
        }
    }

    pub fn info(&self, id: SignalId) -> SignalInfo {
        SignalInfo {
            id,
            name: self.name.clone(),
            path: self.path.clone(),
            sample_rate: self.sample_rate,
            sample_count: self.samples.len(),
            duration_s: self.duration_s(),
        }
    }
}

pub(crate) struct StoredSignal {
    /// `None` while the load job is still pending/failed.
    pub(crate) data: Option<Arc<SignalData>>,
    pub(crate) job: Option<JobId>,
}

pub(crate) struct StoredAnalysis {
    /// `None` until the analysis job is `Ready`.
    pub(crate) data: Option<Arc<SpectrogramData>>,
    pub(crate) signal: SignalId,
    pub(crate) job: JobId,
}

pub(crate) struct StoredCut {
    pub(crate) data: Option<Arc<CutData>>,
    /// Core cut curves (orders, fixed frequencies, or lines) for transport
    /// parity and the stable CSV writer.
    pub(crate) core_cuts: Vec<Cut>,
    pub(crate) analysis: AnalysisId,
    pub(crate) job: JobId,
    pub(crate) kind: RuntimeCutKind,
    pub(crate) name: String,
}

/// All owned runtime state. Only ever accessed while holding the runtime's
/// state mutex, and never across a long computation or an event sink call.
pub(crate) struct RuntimeState {
    pub(crate) next_signal: u64,
    pub(crate) next_analysis: u64,
    pub(crate) next_cut: u64,
    pub(crate) signals: HashMap<SignalId, StoredSignal>,
    pub(crate) analyses: HashMap<AnalysisId, StoredAnalysis>,
    pub(crate) cuts: HashMap<CutId, StoredCut>,
    pub(crate) entity_jobs: HashMap<EntityId, JobId>,
    pub(crate) scheduler: SchedulerState,
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self {
            next_signal: 1,
            next_analysis: 1,
            next_cut: 1,
            signals: HashMap::new(),
            analyses: HashMap::new(),
            cuts: HashMap::new(),
            entity_jobs: HashMap::new(),
            scheduler: SchedulerState::default(),
        }
    }
}

impl RuntimeState {
    pub(crate) fn allocate_signal(&mut self) -> SignalId {
        let id = SignalId(self.next_signal.max(1));
        self.next_signal = id.0.saturating_add(1);
        id
    }

    pub(crate) fn allocate_analysis(&mut self) -> AnalysisId {
        let id = AnalysisId(self.next_analysis.max(1));
        self.next_analysis = id.0.saturating_add(1);
        id
    }

    pub(crate) fn allocate_cut(&mut self) -> CutId {
        let id = CutId(self.next_cut.max(1));
        self.next_cut = id.0.saturating_add(1);
        id
    }

    pub(crate) fn job_for(&self, entity: EntityId) -> Option<JobId> {
        self.entity_jobs.get(&entity).copied()
    }

    pub(crate) fn job_status(&self, job: JobId) -> JobStatus {
        self.scheduler
            .jobs
            .get(&job)
            .map_or(JobStatus::Closed, |record| record.status)
    }

    fn job_error(&self, job: JobId, fallback: impl FnOnce() -> RuntimeError) -> RuntimeError {
        match self.scheduler.jobs.get(&job) {
            Some(record) => match record.status {
                JobStatus::Pending | JobStatus::Running => {
                    RuntimeError::busy("result is not ready").with_context(job.to_string())
                }
                JobStatus::Failed => record.error.clone().unwrap_or_else(fallback),
                JobStatus::Closed => record
                    .error
                    .clone()
                    .unwrap_or_else(|| RuntimeError::cancelled("job is closed")),
                JobStatus::Ready => RuntimeError::internal("ready job has no stored data")
                    .with_context(job.to_string()),
            },
            None => fallback(),
        }
    }

    pub(crate) fn signal_data(&self, id: SignalId) -> Result<Arc<SignalData>, RuntimeError> {
        let Some(signal) = self.signals.get(&id) else {
            return Err(RuntimeError::not_found(format!("signal {id} not found")));
        };
        if let Some(data) = &signal.data {
            return Ok(Arc::clone(data));
        }
        match signal.job {
            Some(job) => Err(self.job_error(job, || {
                RuntimeError::closed(format!("signal {id} is closed"))
            })),
            None => Err(RuntimeError::internal(format!(
                "signal {id} has neither data nor a load job"
            ))),
        }
    }

    pub(crate) fn signal_info(&self, id: SignalId) -> Result<SignalInfo, RuntimeError> {
        Ok(self.signal_data(id)?.info(id))
    }

    pub(crate) fn analysis_data(
        &self,
        id: AnalysisId,
    ) -> Result<Arc<SpectrogramData>, RuntimeError> {
        let Some(analysis) = self.analyses.get(&id) else {
            return Err(RuntimeError::not_found(format!("analysis {id} not found")));
        };
        if let Some(data) = &analysis.data {
            return Ok(Arc::clone(data));
        }
        Err(self.job_error(analysis.job, || {
            RuntimeError::closed(format!("analysis {id} is closed"))
        }))
    }

    pub(crate) fn cut_data(&self, id: CutId) -> Result<Arc<CutData>, RuntimeError> {
        let Some(cut) = self.cuts.get(&id) else {
            return Err(RuntimeError::not_found(format!("cut {id} not found")));
        };
        if let Some(data) = &cut.data {
            return Ok(Arc::clone(data));
        }
        Err(self.job_error(cut.job, || {
            RuntimeError::closed(format!("cut {id} is closed"))
        }))
    }

    pub(crate) fn cut_points(&self, id: CutId) -> Result<Vec<Cut>, RuntimeError> {
        // Resolving the data first preserves the Busy/Failed/Closed mapping.
        self.cut_data(id)?;
        let cut = self
            .cuts
            .get(&id)
            .ok_or_else(|| RuntimeError::not_found(format!("cut {id} not found")))?;
        Ok(cut.core_cuts.clone())
    }

    pub(crate) fn cut_info(&self, id: CutId) -> Result<CutInfo, RuntimeError> {
        let cut = self
            .cuts
            .get(&id)
            .ok_or_else(|| RuntimeError::not_found(format!("cut {id} not found")))?;
        let data = self.cut_data(id)?;
        Ok(CutInfo {
            id,
            analysis: cut.analysis,
            kind: cut.kind,
            name: cut.name.clone(),
            tracks: data.tracks(),
        })
    }

    /// Store a successful job result. Returns `false` when the entity was
    /// closed while the job was running (the snapshot is discarded).
    pub(crate) fn apply_output(&mut self, output: JobOutput) -> bool {
        match output {
            JobOutput::None => true,
            JobOutput::Signal(id, data) => match self.signals.get_mut(&id) {
                Some(signal) => {
                    signal.data = Some(Arc::new(data));
                    true
                }
                None => false,
            },
            JobOutput::Analysis(id, data) => match self.analyses.get_mut(&id) {
                Some(analysis) => {
                    analysis.data = Some(Arc::from(data));
                    true
                }
                None => false,
            },
            JobOutput::Cut(id, data, core_cuts) => match self.cuts.get_mut(&id) {
                Some(cut) => {
                    cut.data = Some(data);
                    cut.core_cuts = core_cuts;
                    true
                }
                None => false,
            },
        }
    }
}
