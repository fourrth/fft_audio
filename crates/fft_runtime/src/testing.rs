//! Test double for [`FftRuntime`].
//!
//! This module is always compiled (not `cfg(test)`) so `fft_app`/`fft_script`
//! tests and adapter tests can use it.
//!
//! [`MockRuntime`] stores typed signals/analyses/cuts in memory and schedules
//! jobs deterministically. By default a submitted analysis/cut completes
//! immediately. Use [`MockRuntime::new_held`] (or `set_auto_complete(false)`)
//! to keep jobs pending until [`MockRuntime::complete`],
//! [`MockRuntime::fail`], or [`MockRuntime::cancel`] is called.
//!
//! Computed payloads are empty-but-shaped placeholders: they keep the typed
//! buffers and event flow, not real DSP. `save_image` reports `Unsupported`
//! because the mock has no PNG encoder.

use std::collections::{HashMap, VecDeque};
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use fft_core::orders::parse_orders;
use fft_core::settings::AnalysisSettings;
use fft_core::spectrogram::{RenderOptions, Spectrogram, SpectrogramGrid};

use crate::data::{
    AnalysisInfo, CutData, CutInfo, CutKind, LineCutData, LineCutRequest, LoadFileRequest,
    LoadSignalRequest, OrderCutData, OrderCutRequest, SignalInfo, SpectrogramData,
};
use crate::error::RuntimeError;
use crate::events::{JobStatus, RuntimeEvent, RuntimeEventSink};
use crate::ids::{AnalysisId, CutId, EntityId, JobId, SignalId};
use crate::FftRuntime;

/// In-memory [`FftRuntime`] test implementation.
pub struct MockRuntime {
    state: Mutex<State>,
    wake: Condvar,
    events: Mutex<Vec<RuntimeEvent>>,
    sink: Mutex<Option<Arc<dyn RuntimeEventSink>>>,
}

#[derive(Default)]
struct State {
    next_signal: u64,
    next_analysis: u64,
    next_cut: u64,
    next_job: u64,
    signals: HashMap<SignalId, MockSignal>,
    analyses: HashMap<AnalysisId, MockAnalysis>,
    cuts: HashMap<CutId, MockCut>,
    jobs: HashMap<JobId, JobRecord>,
    entity_jobs: HashMap<EntityId, JobId>,
    auto_complete: bool,
    fail_analyses: VecDeque<String>,
    fail_cuts: VecDeque<String>,
}

struct MockSignal {
    info: SignalInfo,
    samples: Arc<Vec<f32>>,
}

struct MockAnalysis {
    info: AnalysisInfo,
    data: Arc<SpectrogramData>,
    job: JobId,
}

struct MockCut {
    info: CutInfo,
    data: Arc<CutData>,
    job: JobId,
}

struct JobRecord {
    status: JobStatus,
    error: Option<RuntimeError>,
    entity: EntityId,
}

impl Default for MockRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl MockRuntime {
    /// Auto-completing mock runtime.
    pub fn new() -> Self {
        Self::with_auto_complete(true)
    }

    /// Mock runtime whose new jobs stay `Pending` until explicitly completed,
    /// failed, or cancelled.
    pub fn new_held() -> Self {
        Self::with_auto_complete(false)
    }

    fn with_auto_complete(auto_complete: bool) -> Self {
        Self {
            state: Mutex::new(State {
                next_signal: 1,
                next_analysis: 1,
                next_cut: 1,
                next_job: 1,
                auto_complete,
                ..State::default()
            }),
            wake: Condvar::new(),
            events: Mutex::new(Vec::new()),
            sink: Mutex::new(None),
        }
    }

    /// Register an observer that receives every event in addition to the
    /// internal capture returned by [`MockRuntime::events`].
    pub fn with_event_sink(self, sink: Arc<dyn RuntimeEventSink>) -> Self {
        self.set_event_sink(sink);
        self
    }

    pub fn set_event_sink(&self, sink: Arc<dyn RuntimeEventSink>) {
        *self.sink.lock().unwrap_or_else(|e| e.into_inner()) = Some(sink);
    }

    pub fn auto_complete(&self) -> bool {
        self.lock().auto_complete
    }

    /// Change behavior for future jobs. Already pending jobs are unaffected.
    pub fn set_auto_complete(&self, enabled: bool) {
        self.lock().auto_complete = enabled;
    }

    /// Make the next analysis fail at submission time.
    pub fn fail_next_analysis(&self, message: impl Into<String>) {
        self.lock().fail_analyses.push_back(message.into());
    }

    /// Make the next order/line cut fail at submission time.
    pub fn fail_next_cut(&self, message: impl Into<String>) {
        self.lock().fail_cuts.push_back(message.into());
    }

    /// Complete a pending/running job. Terminal jobs are left unchanged.
    pub fn complete(&self, job: JobId) -> Result<(), RuntimeError> {
        let events = {
            let mut state = self.lock();
            let Some(record) = state.jobs.get_mut(&job) else {
                return Err(RuntimeError::not_found("job not found"));
            };
            if !record.status.is_active() {
                return Ok(());
            }
            record.status = JobStatus::Ready;
            self.wake.notify_all();
            vec![
                RuntimeEvent::Progress {
                    job,
                    frames_done: 1,
                    frames_total: Some(1),
                    eta_s: Some(0.0),
                },
                RuntimeEvent::Done { job },
            ]
        };
        self.emit(events);
        Ok(())
    }

    /// Fail a pending/running job with an `AnalysisFailed` error.
    pub fn fail(&self, job: JobId, message: impl Into<String>) -> Result<(), RuntimeError> {
        let message = message.into();
        let event = {
            let mut state = self.lock();
            let Some(record) = state.jobs.get_mut(&job) else {
                return Err(RuntimeError::not_found("job not found"));
            };
            if !record.status.is_active() {
                return Ok(());
            }
            record.status = JobStatus::Failed;
            record.error = Some(RuntimeError::analysis_failed(message.clone()));
            self.wake.notify_all();
            RuntimeEvent::Failed { job, message }
        };
        self.emit(vec![event]);
        Ok(())
    }

    pub fn events(&self) -> Vec<RuntimeEvent> {
        self.events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn clear_events(&self) {
        self.events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }

    pub fn job_ids(&self) -> Vec<JobId> {
        let mut ids: Vec<_> = self.lock().jobs.keys().copied().collect();
        ids.sort();
        ids
    }

    pub fn signal_ids(&self) -> Vec<SignalId> {
        let mut ids: Vec<_> = self.lock().signals.keys().copied().collect();
        ids.sort();
        ids
    }

    pub fn analysis_ids(&self) -> Vec<AnalysisId> {
        let mut ids: Vec<_> = self.lock().analyses.keys().copied().collect();
        ids.sort();
        ids
    }

    pub fn cut_ids(&self) -> Vec<CutId> {
        let mut ids: Vec<_> = self.lock().cuts.keys().copied().collect();
        ids.sort();
        ids
    }

    pub fn signal_info(&self, id: SignalId) -> Option<SignalInfo> {
        self.lock().signals.get(&id).map(|s| s.info.clone())
    }

    /// Stored samples; useful for script tests that want to inspect input.
    pub fn signal_samples(&self, id: SignalId) -> Option<Arc<Vec<f32>>> {
        self.lock().signals.get(&id).map(|s| Arc::clone(&s.samples))
    }

    pub fn analysis_info(&self, id: AnalysisId) -> Option<AnalysisInfo> {
        self.lock().analyses.get(&id).map(|a| a.info.clone())
    }

    pub fn cut_info(&self, id: CutId) -> Option<CutInfo> {
        self.lock().cuts.get(&id).map(|c| c.info.clone())
    }

    /// Move a ready cut out of the mock store when the store owns the only
    /// remaining `Arc`. Borrowed cuts return `busy` and stay in the store.
    pub fn take_cut_data(&self, id: CutId) -> Result<CutData, RuntimeError> {
        let mut state = self.lock();
        let Some(cut) = state.cuts.get(&id) else {
            return Err(RuntimeError::not_found(format!("cut {id} not found")));
        };
        let job = cut.job;
        let (status, error) = match state.jobs.get(&job) {
            Some(job) => (job.status, job.error.clone()),
            None => return Err(RuntimeError::closed("cut job is closed")),
        };
        let data = match status {
            JobStatus::Ready => Arc::clone(&cut.data),
            JobStatus::Pending | JobStatus::Running => {
                return Err(RuntimeError::busy(format!("cut {id} is not ready")))
            }
            JobStatus::Failed => {
                return Err(error.unwrap_or_else(|| RuntimeError::analysis_failed("cut failed")))
            }
            JobStatus::Closed => {
                return Err(error.unwrap_or_else(|| RuntimeError::cancelled("cut cancelled")))
            }
        };
        if Arc::strong_count(&data) != 2 {
            return Err(RuntimeError::busy(format!(
                "cut {id} data is borrowed; close read views before taking ownership"
            )));
        }
        state.cuts.remove(&id);
        state.entity_jobs.remove(&EntityId::Cut(id));
        state.jobs.remove(&job);
        self.wake.notify_all();
        drop(state);
        Arc::try_unwrap(data).map_err(|_| {
            RuntimeError::busy(format!(
                "cut {id} data is borrowed during ownership transfer"
            ))
        })
    }

    /// Wait until every mock job is terminal. `None` waits indefinitely.
    pub fn wait_all(&self, timeout: Option<Duration>) -> Result<(), RuntimeError> {
        let deadline = timeout.map(|timeout| Instant::now() + timeout);
        let mut state = self.lock();
        loop {
            let active = state.jobs.values().any(|job| job.status.is_active());
            if !active {
                return Ok(());
            }
            state = match deadline {
                Some(deadline) => {
                    let now = Instant::now();
                    if now >= deadline {
                        return Err(RuntimeError::busy("timed out waiting for jobs"));
                    }
                    self.wake
                        .wait_timeout(state, deadline - now)
                        .unwrap_or_else(|error| error.into_inner())
                        .0
                }
                None => self
                    .wake
                    .wait(state)
                    .unwrap_or_else(|error| error.into_inner()),
            };
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn emit(&self, batch: Vec<RuntimeEvent>) {
        if batch.is_empty() {
            return;
        }
        {
            let mut captured = self.events.lock().unwrap_or_else(|e| e.into_inner());
            captured.extend(batch.iter().cloned());
        }
        let sink = self.sink.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(sink) = sink {
            for event in batch {
                sink.emit(event);
            }
        }
    }
}

impl FftRuntime for MockRuntime {
    fn load_file(&self, request: LoadFileRequest) -> std::result::Result<SignalId, RuntimeError> {
        if request.path.as_os_str().is_empty() {
            return Err(RuntimeError::invalid_input("path must not be empty"));
        }
        let name = request.name.unwrap_or_else(|| {
            request
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("file")
                .to_string()
        });
        let mut state = self.lock();
        let id = SignalId(state.next_signal);
        state.next_signal += 1;
        let info = SignalInfo {
            id,
            name,
            path: Some(request.path),
            // The mock does not open the file; keep deterministic metadata.
            sample_rate: 1.0,
            sample_count: 0,
            duration_s: 0.0,
        };
        state.signals.insert(
            id,
            MockSignal {
                info,
                samples: Arc::new(Vec::new()),
            },
        );
        Ok(id)
    }

    fn load_signal(
        &self,
        request: LoadSignalRequest,
    ) -> std::result::Result<SignalId, RuntimeError> {
        if !request.sample_rate.is_finite() || request.sample_rate <= 0.0 {
            return Err(RuntimeError::invalid_input(
                "sample rate must be a positive finite number",
            ));
        }
        let mut state = self.lock();
        let id = SignalId(state.next_signal);
        state.next_signal += 1;
        let sample_count = request.samples.len();
        let info = SignalInfo {
            id,
            name: request.name.unwrap_or_else(|| "signal".to_string()),
            path: None,
            sample_rate: request.sample_rate,
            sample_count,
            duration_s: sample_count as f64 / request.sample_rate,
        };
        state.signals.insert(
            id,
            MockSignal {
                info,
                samples: Arc::new(request.samples),
            },
        );
        Ok(id)
    }

    fn analyze(
        &self,
        signal: SignalId,
        settings: &AnalysisSettings,
    ) -> std::result::Result<AnalysisId, RuntimeError> {
        settings.validate().map_err(|error| {
            RuntimeError::invalid_input(format!("invalid analysis settings: {error}"))
        })?;

        let (analysis_id, events) = {
            let mut state = self.lock();
            let Some(source) = state.signals.get(&signal) else {
                return Err(RuntimeError::not_found(format!(
                    "signal {signal} not found"
                )));
            };
            let source = source.info.clone();
            let analysis_id = AnalysisId(state.next_analysis);
            state.next_analysis += 1;
            let job = JobId(state.next_job);
            state.next_job += 1;
            let failure = state.fail_analyses.pop_front();
            let status = if failure.is_some() {
                JobStatus::Failed
            } else if state.auto_complete {
                JobStatus::Ready
            } else {
                JobStatus::Pending
            };
            let frame_s = if source.sample_rate > 0.0 {
                settings.frame as f64 / source.sample_rate
            } else {
                0.0
            };
            let info = AnalysisInfo {
                id: analysis_id,
                signal,
                name: source.name.clone(),
                path: source.path.clone(),
                sample_rate: source.sample_rate,
                frame_s,
                // The mock has no DSP payload; dimensions stay deterministic.
                frames: 0,
                bins: 0,
                bin_hz: 0.0,
                hop_s: 0.0,
                duration_s: 0.0,
                nyquist: 0.0,
                max_db: 0.0,
                min_db: 0.0,
                table: None,
            };
            let data = Arc::new(SpectrogramData {
                spectrogram: Spectrogram::new(),
                name: source.name,
                path: source.path,
                sample_rate: source.sample_rate,
                table: None,
                generation: 1,
                analysis: settings.clone(),
            });
            state.analyses.insert(
                analysis_id,
                MockAnalysis {
                    info,
                    data: Arc::clone(&data),
                    job,
                },
            );
            let error = failure
                .as_ref()
                .map(|message| RuntimeError::analysis_failed(message.clone()));
            state.jobs.insert(
                job,
                JobRecord {
                    status,
                    error,
                    entity: EntityId::Analysis(analysis_id),
                },
            );
            state
                .entity_jobs
                .insert(EntityId::Analysis(analysis_id), job);
            let events = match (status, failure) {
                (JobStatus::Failed, Some(message)) => vec![RuntimeEvent::Failed { job, message }],
                (JobStatus::Ready, _) => vec![
                    RuntimeEvent::Progress {
                        job,
                        frames_done: 1,
                        frames_total: Some(1),
                        eta_s: Some(0.0),
                    },
                    RuntimeEvent::Done { job },
                ],
                _ => Vec::new(),
            };
            (analysis_id, events)
        };
        self.emit(events);
        Ok(analysis_id)
    }

    fn job_for(&self, id: EntityId) -> Option<JobId> {
        self.lock().entity_jobs.get(&id).copied()
    }

    fn job_status(&self, job: JobId) -> JobStatus {
        self.lock()
            .jobs
            .get(&job)
            .map_or(JobStatus::Closed, |record| record.status)
    }

    fn wait(&self, job: JobId, timeout: Option<Duration>) -> std::result::Result<(), RuntimeError> {
        let deadline = timeout.map(|timeout| Instant::now() + timeout);
        let mut state = self.lock();
        loop {
            let (status, error) = match state.jobs.get(&job) {
                Some(record) => (record.status, record.error.clone()),
                None => return Err(RuntimeError::not_found("job not found")),
            };
            match status {
                JobStatus::Ready => return Ok(()),
                JobStatus::Failed => {
                    return Err(error.unwrap_or_else(|| {
                        RuntimeError::analysis_failed("job failed without a message")
                    }))
                }
                JobStatus::Closed => {
                    return Err(error.unwrap_or_else(|| RuntimeError::closed("job is closed")))
                }
                JobStatus::Pending | JobStatus::Running => {}
            }
            state = match deadline {
                Some(deadline) => {
                    let now = Instant::now();
                    if now >= deadline {
                        return Err(RuntimeError::busy("timed out waiting for job"));
                    }
                    self.wake
                        .wait_timeout(state, deadline - now)
                        .map_err(|_| RuntimeError::internal("runtime lock poisoned"))?
                        .0
                }
                None => self
                    .wake
                    .wait(state)
                    .map_err(|_| RuntimeError::internal("runtime lock poisoned"))?,
            };
        }
    }

    fn cancel(&self, job: JobId) -> std::result::Result<(), RuntimeError> {
        let event = {
            let mut state = self.lock();
            let Some(record) = state.jobs.get_mut(&job) else {
                return Err(RuntimeError::not_found("job not found"));
            };
            if !record.status.is_active() {
                return Ok(());
            }
            record.status = JobStatus::Closed;
            record.error = Some(RuntimeError::cancelled("job cancelled"));
            self.wake.notify_all();
            RuntimeEvent::Cancelled { job }
        };
        self.emit(vec![event]);
        Ok(())
    }

    fn spectrogram_grid(
        &self,
        id: AnalysisId,
        max_frames: usize,
        max_bins: usize,
    ) -> std::result::Result<SpectrogramGrid, RuntimeError> {
        let data = self.spectrogram_data(id)?;
        Ok(data.spectrogram.to_grid(max_frames, max_bins))
    }

    fn spectrogram_data(
        &self,
        id: AnalysisId,
    ) -> std::result::Result<Arc<SpectrogramData>, RuntimeError> {
        let state = self.lock();
        let Some(analysis) = state.analyses.get(&id) else {
            return Err(RuntimeError::not_found(format!("analysis {id} not found")));
        };
        let (status, error) = match state.jobs.get(&analysis.job) {
            Some(job) => (job.status, job.error.clone()),
            None => return Err(RuntimeError::closed("analysis job is closed")),
        };
        match status {
            JobStatus::Ready => Ok(Arc::clone(&analysis.data)),
            JobStatus::Pending | JobStatus::Running => {
                Err(RuntimeError::busy(format!("analysis {id} is not ready")))
            }
            JobStatus::Failed => {
                Err(error.unwrap_or_else(|| RuntimeError::analysis_failed("analysis failed")))
            }
            JobStatus::Closed => {
                Err(error.unwrap_or_else(|| RuntimeError::cancelled("analysis cancelled")))
            }
        }
    }

    fn order_cut(
        &self,
        id: AnalysisId,
        request: &OrderCutRequest,
    ) -> std::result::Result<CutId, RuntimeError> {
        let mut state = self.lock();
        let (analysis_info, analysis_frames) = {
            let Some(analysis) = state.analyses.get(&id) else {
                return Err(RuntimeError::not_found(format!("analysis {id} not found")));
            };
            match state.jobs.get(&analysis.job).map(|job| job.status) {
                Some(JobStatus::Ready) => {}
                Some(JobStatus::Failed) => {
                    return Err(RuntimeError::analysis_failed(format!(
                        "analysis {id} failed"
                    )))
                }
                Some(JobStatus::Closed) => {
                    return Err(RuntimeError::closed(format!("analysis {id} is closed")))
                }
                Some(JobStatus::Pending | JobStatus::Running) | None => {
                    return Err(RuntimeError::busy(format!("analysis {id} is not ready")))
                }
            }
            (analysis.info.clone(), analysis.info.frames)
        };

        let orders = parse_orders(&request.settings.orders);
        let times: Vec<f64> = (0..analysis_frames)
            .map(|frame| frame as f64 * analysis_info.hop_s)
            .collect();
        let cells = times.len() * orders.len();
        let data = OrderCutData::new(vec![0.0; cells], times, orders, vec![0.0; cells]);
        let cut_id = CutId(state.next_cut);
        state.next_cut += 1;
        let job = JobId(state.next_job);
        state.next_job += 1;
        insert_cut(
            &mut state,
            cut_id,
            job,
            CutInfo {
                id: cut_id,
                analysis: id,
                kind: CutKind::Order,
                name: "order cuts".to_string(),
                tracks: data.order_count,
            },
            CutData::Order(data),
        );
        let failure = state.fail_cuts.pop_front();
        let events = finish_cut_job(&mut state, job, failure);
        drop(state);
        self.emit(events);
        Ok(cut_id)
    }

    fn line_cut(
        &self,
        id: AnalysisId,
        request: &LineCutRequest,
    ) -> std::result::Result<CutId, RuntimeError> {
        let mut state = self.lock();
        {
            let Some(analysis) = state.analyses.get(&id) else {
                return Err(RuntimeError::not_found(format!("analysis {id} not found")));
            };
            match state.jobs.get(&analysis.job).map(|job| job.status) {
                Some(JobStatus::Ready) => {}
                Some(JobStatus::Failed) => {
                    return Err(RuntimeError::analysis_failed(format!(
                        "analysis {id} failed"
                    )))
                }
                Some(JobStatus::Closed) => {
                    return Err(RuntimeError::closed(format!("analysis {id} is closed")))
                }
                Some(JobStatus::Pending | JobStatus::Running) | None => {
                    return Err(RuntimeError::busy(format!("analysis {id} is not ready")))
                }
            }
        }

        let tracks: Vec<LineCutData> = request
            .lines
            .iter()
            .map(|_| LineCutData::new(Vec::new(), Vec::new(), Vec::new(), Vec::new()))
            .collect();
        let track_count = tracks.len();
        let cut_id = CutId(state.next_cut);
        state.next_cut += 1;
        let job = JobId(state.next_job);
        state.next_job += 1;
        insert_cut(
            &mut state,
            cut_id,
            job,
            CutInfo {
                id: cut_id,
                analysis: id,
                kind: CutKind::Line,
                name: "line cuts".to_string(),
                tracks: track_count,
            },
            CutData::Line(tracks),
        );
        let failure = state.fail_cuts.pop_front();
        let events = finish_cut_job(&mut state, job, failure);
        drop(state);
        self.emit(events);
        Ok(cut_id)
    }

    fn cut_data(&self, id: CutId) -> std::result::Result<Arc<CutData>, RuntimeError> {
        let state = self.lock();
        let Some(cut) = state.cuts.get(&id) else {
            return Err(RuntimeError::not_found(format!("cut {id} not found")));
        };
        let (status, error) = match state.jobs.get(&cut.job) {
            Some(job) => (job.status, job.error.clone()),
            None => return Err(RuntimeError::closed("cut job is closed")),
        };
        match status {
            JobStatus::Ready => Ok(Arc::clone(&cut.data)),
            JobStatus::Pending | JobStatus::Running => {
                Err(RuntimeError::busy(format!("cut {id} is not ready")))
            }
            JobStatus::Failed => {
                Err(error.unwrap_or_else(|| RuntimeError::analysis_failed("cut failed")))
            }
            JobStatus::Closed => {
                Err(error.unwrap_or_else(|| RuntimeError::cancelled("cut cancelled")))
            }
        }
    }

    fn signal_info(&self, id: SignalId) -> std::result::Result<SignalInfo, RuntimeError> {
        MockRuntime::signal_info(self, id)
            .ok_or_else(|| RuntimeError::not_found(format!("signal {id} not found")))
    }

    fn analysis_info(&self, id: AnalysisId) -> std::result::Result<AnalysisInfo, RuntimeError> {
        MockRuntime::analysis_info(self, id)
            .ok_or_else(|| RuntimeError::not_found(format!("analysis {id} not found")))
    }

    fn cut_info(&self, id: CutId) -> std::result::Result<CutInfo, RuntimeError> {
        MockRuntime::cut_info(self, id)
            .ok_or_else(|| RuntimeError::not_found(format!("cut {id} not found")))
    }

    fn take_cut_data(&self, id: CutId) -> std::result::Result<CutData, RuntimeError> {
        MockRuntime::take_cut_data(self, id)
    }

    fn wait_all(&self, timeout: Option<Duration>) -> std::result::Result<(), RuntimeError> {
        MockRuntime::wait_all(self, timeout)
    }

    fn export_csv(&self, id: CutId, path: &Path) -> std::result::Result<(), RuntimeError> {
        let data = self.cut_data(id)?;
        let file = std::fs::File::create(path).map_err(|error| {
            RuntimeError::export(error.to_string()).with_context(path.display().to_string())
        })?;
        let mut writer = BufWriter::new(file);
        let write_result = write_cut_csv(&mut writer, &data);
        write_result.map_err(|error| {
            RuntimeError::export(error.to_string()).with_context(path.display().to_string())
        })?;
        writer.flush().map_err(|error| {
            RuntimeError::export(error.to_string()).with_context(path.display().to_string())
        })
    }

    fn save_image(
        &self,
        _id: AnalysisId,
        _path: &Path,
        _options: &RenderOptions,
    ) -> std::result::Result<(), RuntimeError> {
        Err(RuntimeError::unsupported(
            "MockRuntime does not render images; use a concrete runtime",
        ))
    }

    fn close(&self, id: EntityId) -> std::result::Result<(), RuntimeError> {
        let events = {
            let mut state = self.lock();
            let mut events = Vec::new();
            match id {
                EntityId::Signal(signal) => {
                    if state.signals.remove(&signal).is_none() {
                        return Ok(());
                    }
                    let analyses: Vec<_> = state
                        .analyses
                        .iter()
                        .filter(|(_, analysis)| analysis.info.signal == signal)
                        .map(|(id, _)| *id)
                        .collect();
                    for analysis in analyses {
                        remove_analysis(&mut state, &mut events, analysis);
                    }
                }
                EntityId::Analysis(analysis) => {
                    if !state.analyses.contains_key(&analysis) {
                        return Ok(());
                    }
                    remove_analysis(&mut state, &mut events, analysis);
                }
                EntityId::Cut(cut) => {
                    if !state.cuts.contains_key(&cut) {
                        return Ok(());
                    }
                    remove_cut(&mut state, &mut events, cut);
                }
            }
            self.wake.notify_all();
            events
        };
        self.emit(events);
        Ok(())
    }
}

fn insert_cut(state: &mut State, cut_id: CutId, job: JobId, info: CutInfo, data: CutData) {
    state.cuts.insert(
        cut_id,
        MockCut {
            info,
            data: Arc::new(data),
            job,
        },
    );
    state.jobs.insert(
        job,
        JobRecord {
            status: JobStatus::Pending,
            error: None,
            entity: EntityId::Cut(cut_id),
        },
    );
    state.entity_jobs.insert(EntityId::Cut(cut_id), job);
}

/// Apply the mock's auto/failure policy to a just-inserted cut job.
fn finish_cut_job(state: &mut State, job: JobId, failure: Option<String>) -> Vec<RuntimeEvent> {
    let status = if failure.is_some() {
        JobStatus::Failed
    } else if state.auto_complete {
        JobStatus::Ready
    } else {
        JobStatus::Pending
    };
    if let Some(record) = state.jobs.get_mut(&job) {
        record.status = status;
        record.error = failure
            .as_ref()
            .map(|message| RuntimeError::analysis_failed(message.clone()));
    }
    match (status, failure) {
        (JobStatus::Failed, Some(message)) => vec![RuntimeEvent::Failed { job, message }],
        (JobStatus::Ready, _) => vec![
            RuntimeEvent::Progress {
                job,
                frames_done: 1,
                frames_total: Some(1),
                eta_s: Some(0.0),
            },
            RuntimeEvent::Done { job },
        ],
        _ => Vec::new(),
    }
}

fn remove_analysis(state: &mut State, events: &mut Vec<RuntimeEvent>, id: AnalysisId) {
    let Some(analysis) = state.analyses.remove(&id) else {
        return;
    };
    state.entity_jobs.remove(&EntityId::Analysis(id));
    remove_job(state, events, analysis.job);
    let cuts: Vec<_> = state
        .cuts
        .iter()
        .filter(|(_, cut)| cut.info.analysis == id)
        .map(|(id, _)| *id)
        .collect();
    for cut in cuts {
        remove_cut(state, events, cut);
    }
}

fn remove_cut(state: &mut State, events: &mut Vec<RuntimeEvent>, id: CutId) {
    let Some(cut) = state.cuts.remove(&id) else {
        return;
    };
    state.entity_jobs.remove(&EntityId::Cut(id));
    remove_job(state, events, cut.job);
}

fn remove_job(state: &mut State, events: &mut Vec<RuntimeEvent>, job: JobId) {
    if let Some(record) = state.jobs.remove(&job) {
        state.entity_jobs.remove(&record.entity);
        if record.status.is_active() {
            events.push(RuntimeEvent::Cancelled { job });
        }
    }
}

fn write_cut_csv(writer: &mut impl Write, data: &CutData) -> std::io::Result<()> {
    match data {
        CutData::Order(order) => {
            writeln!(writer, "order,time_s,freq_hz,db")?;
            for frame in 0..order.frames {
                for column in 0..order.order_count {
                    let cell = frame * order.order_count + column;
                    writeln!(
                        writer,
                        "{},{:.6},{:.3},{:.3}",
                        order.orders[column],
                        order.times[frame],
                        order.frequencies[cell],
                        order.values[cell]
                    )?;
                }
            }
        }
        CutData::Line(lines) => {
            writeln!(writer, "line,time_s,freq_hz,fraction,db")?;
            for (index, line) in lines.iter().enumerate() {
                for point in 0..line.len() {
                    writeln!(
                        writer,
                        "{},{:.6},{:.3},{:.6},{:.3}",
                        index + 1,
                        line.times[point],
                        line.frequencies[point],
                        line.fractions[point],
                        line.db[point]
                    )?;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{LineCutRequest, LoadSignalRequest, OrderCutRequest};
    use fft_core::orders::LineSpec;
    use fft_core::settings::LineCutSettings;
    use std::sync::Mutex as StdMutex;

    fn signal(rt: &MockRuntime) -> SignalId {
        rt.load_signal(LoadSignalRequest::new(vec![0.0; 16], 8_000.0))
            .unwrap()
    }

    fn analysis(rt: &MockRuntime) -> AnalysisId {
        let signal = signal(rt);
        rt.analyze(signal, &AnalysisSettings::default()).unwrap()
    }

    #[test]
    fn submit_wait_and_status_are_typed_and_deterministic() {
        let rt = MockRuntime::new_held();
        let signal = signal(&rt);
        let analysis = rt.analyze(signal, &AnalysisSettings::default()).unwrap();
        assert_eq!(signal, SignalId(1));
        assert_eq!(analysis, AnalysisId(1));

        let job = rt.job_for(EntityId::Analysis(analysis)).unwrap();
        assert_eq!(job, JobId(1));
        assert_eq!(rt.job_status(job), JobStatus::Pending);
        assert_eq!(rt.spectrogram_data(analysis).unwrap_err().code(), "busy");

        rt.complete(job).unwrap();
        assert_eq!(rt.job_status(job), JobStatus::Ready);
        rt.wait(job, Some(Duration::from_secs(1))).unwrap();
        assert!(rt
            .spectrogram_data(analysis)
            .unwrap()
            .spectrogram
            .is_empty());
        assert_eq!(
            rt.events(),
            vec![
                RuntimeEvent::Progress {
                    job,
                    frames_done: 1,
                    frames_total: Some(1),
                    eta_s: Some(0.0),
                },
                RuntimeEvent::Done { job },
            ]
        );
    }

    #[test]
    fn auto_completion_reports_ready_and_progress() {
        let rt = MockRuntime::new();
        let signal = signal(&rt);
        let analysis = rt.analyze(signal, &AnalysisSettings::default()).unwrap();
        let job = rt.job_for(EntityId::Analysis(analysis)).unwrap();
        assert_eq!(rt.job_status(job), JobStatus::Ready);
        rt.wait(job, None).unwrap();
        assert!(matches!(
            rt.events().as_slice(),
            [RuntimeEvent::Progress { .. }, RuntimeEvent::Done { job: done }] if *done == job
        ));
    }

    #[test]
    fn failures_are_terminal_and_typed() {
        let rt = MockRuntime::new();
        rt.fail_next_analysis("synthetic failure");
        let analysis = analysis(&rt);
        let job = rt.job_for(EntityId::Analysis(analysis)).unwrap();
        assert_eq!(rt.job_status(job), JobStatus::Failed);
        let error = rt.wait(job, None).unwrap_err();
        assert_eq!(error.code(), "analysis_failed");
        assert!(error.message().contains("synthetic failure"));
        assert_eq!(
            rt.spectrogram_data(analysis).unwrap_err().code(),
            "analysis_failed"
        );
        assert!(matches!(
            rt.events().as_slice(),
            [RuntimeEvent::Failed { job: failed, .. }] if *failed == job
        ));
    }

    #[test]
    fn cancellation_closes_the_job_and_cascades_to_data_access() {
        let rt = MockRuntime::new_held();
        let analysis = analysis(&rt);
        let job = rt.job_for(EntityId::Analysis(analysis)).unwrap();
        rt.cancel(job).unwrap();
        assert_eq!(rt.job_status(job), JobStatus::Closed);
        assert_eq!(rt.wait(job, None).unwrap_err().code(), "cancelled");
        assert_eq!(
            rt.spectrogram_data(analysis).unwrap_err().code(),
            "cancelled"
        );
        assert_eq!(rt.events(), vec![RuntimeEvent::Cancelled { job }]);
        // Cancelling a terminal job is a no-op.
        rt.cancel(job).unwrap();
        assert_eq!(rt.events().len(), 1);
    }

    #[test]
    fn wait_timeout_is_busy_without_swallowing_the_job() {
        let rt = MockRuntime::new_held();
        let analysis = analysis(&rt);
        let job = rt.job_for(EntityId::Analysis(analysis)).unwrap();
        let error = rt.wait(job, Some(Duration::from_millis(5))).unwrap_err();
        assert_eq!(error.code(), "busy");
        assert_eq!(rt.job_status(job), JobStatus::Pending);
        rt.complete(job).unwrap();
        rt.wait(job, None).unwrap();
    }

    #[test]
    fn close_is_idempotent_and_parent_close_cascades() {
        let rt = MockRuntime::new();
        let signal = signal(&rt);
        let analysis = rt.analyze(signal, &AnalysisSettings::default()).unwrap();
        let order = rt.order_cut(analysis, &OrderCutRequest::default()).unwrap();
        let line = rt
            .line_cut(
                analysis,
                &LineCutRequest::new(
                    LineCutSettings::default(),
                    vec![LineSpec {
                        t0: 0.0,
                        f0: 0.0,
                        t1: 1.0,
                        f1: 100.0,
                        log_f: false,
                    }],
                ),
            )
            .unwrap();
        assert_eq!(rt.cut_ids(), vec![order, line]);
        assert_eq!(rt.cut_info(order).unwrap().tracks, 3);
        assert_eq!(rt.cut_info(line).unwrap().tracks, 1);

        rt.close(EntityId::Analysis(analysis)).unwrap();
        assert!(rt.cut_ids().is_empty());
        assert!(rt.analysis_ids().is_empty());
        assert_eq!(rt.job_for(EntityId::Analysis(analysis)), None);
        assert_eq!(rt.cut_data(order).unwrap_err().code(), "not_found");

        // Re-closing an already removed analysis is a successful no-op.
        rt.close(EntityId::Analysis(analysis)).unwrap();
        // Closing the signal removes anything still derived from it.
        rt.close(EntityId::Signal(signal)).unwrap();
        rt.close(EntityId::Signal(signal)).unwrap();
        assert!(rt.signal_ids().is_empty());
    }

    #[test]
    fn event_sinks_receive_the_same_stream_as_the_internal_capture() {
        let captured = Arc::new(StdMutex::new(Vec::<RuntimeEvent>::new()));
        let target = Arc::clone(&captured);
        let sink: Arc<dyn RuntimeEventSink> = Arc::new(move |event: RuntimeEvent| {
            target.lock().unwrap().push(event);
        });
        let rt = MockRuntime::new().with_event_sink(sink);
        let analysis = analysis(&rt);
        let job = rt.job_for(EntityId::Analysis(analysis)).unwrap();
        assert_eq!(
            *captured.lock().unwrap(),
            rt.events(),
            "sink and internal capture must agree"
        );
        assert_eq!(rt.job_status(job), JobStatus::Ready);
    }

    #[test]
    fn load_validation_and_store_accessors_are_typed() {
        let rt = MockRuntime::new();
        let error = rt
            .load_signal(LoadSignalRequest::new(vec![0.0], 0.0))
            .unwrap_err();
        assert_eq!(error.code(), "invalid_input");
        let signal = rt
            .load_signal(LoadSignalRequest::new(vec![1.0, 2.0], 2.0).with_name("two samples"))
            .unwrap();
        let info = rt.signal_info(signal).unwrap();
        assert_eq!(info.name, "two samples");
        assert_eq!(info.sample_count, 2);
        assert!((info.duration_s - 1.0).abs() < 1e-9);
        assert_eq!(rt.signal_samples(signal).unwrap().as_slice(), &[1.0, 2.0]);
        assert_eq!(
            rt.analyze(SignalId(99), &AnalysisSettings::default())
                .unwrap_err()
                .code(),
            "not_found"
        );
    }

    #[test]
    fn held_cuts_can_be_completed_like_analyses() {
        let rt = MockRuntime::new_held();
        let analysis = analysis(&rt);
        let analysis_job = rt.job_for(EntityId::Analysis(analysis)).unwrap();
        rt.complete(analysis_job).unwrap();

        let cut = rt.order_cut(analysis, &OrderCutRequest::default()).unwrap();
        let job = rt.job_for(EntityId::Cut(cut)).unwrap();
        assert_eq!(rt.job_status(job), JobStatus::Pending);
        assert_eq!(rt.cut_data(cut).unwrap_err().code(), "busy");
        rt.complete(job).unwrap();
        rt.wait(job, None).unwrap();
        assert!(matches!(
            &*rt.cut_data(cut).unwrap(),
            CutData::Order(data) if data.is_consistent()
        ));
    }

    #[test]
    fn take_cut_data_moves_a_ready_result_out_of_the_store() {
        let rt = MockRuntime::new();
        let analysis = analysis(&rt);
        let cut = rt.order_cut(analysis, &OrderCutRequest::default()).unwrap();
        let job = rt.job_for(EntityId::Cut(cut)).unwrap();
        let data = rt.take_cut_data(cut).unwrap();
        assert!(matches!(data, CutData::Order(ref order) if order.is_consistent()));
        assert!(rt.cut_ids().is_empty());
        assert_eq!(rt.job_for(EntityId::Cut(cut)), None);
        assert_eq!(rt.job_status(job), JobStatus::Closed);
        assert_eq!(rt.cut_data(cut).unwrap_err().code(), "not_found");
    }

    #[test]
    fn take_cut_data_rejects_borrowed_results_without_removing_them() {
        let rt = MockRuntime::new();
        let analysis = analysis(&rt);
        let cut = rt.order_cut(analysis, &OrderCutRequest::default()).unwrap();
        let borrowed = rt.cut_data(cut).unwrap();

        let error = rt.take_cut_data(cut).unwrap_err();
        assert_eq!(error.code(), "busy");
        assert_eq!(rt.cut_ids(), vec![cut], "a rejected transfer keeps the cut");

        drop(borrowed);
        let data = rt.take_cut_data(cut).unwrap();
        assert!(matches!(data, CutData::Order(ref order) if order.is_consistent()));
        assert!(rt.cut_ids().is_empty());
    }

    #[test]
    fn take_cut_data_rejects_pending_cuts_without_removing_them() {
        let rt = MockRuntime::new_held();
        let analysis = analysis(&rt);
        let job = rt.job_for(EntityId::Analysis(analysis)).unwrap();
        rt.complete(job).unwrap();
        let cut = rt.order_cut(analysis, &OrderCutRequest::default()).unwrap();
        assert_eq!(rt.take_cut_data(cut).unwrap_err().code(), "busy");
        assert_eq!(rt.cut_ids(), vec![cut]);
    }

    #[test]
    fn trait_info_accessors_and_wait_all_are_typed() {
        let rt = MockRuntime::new();
        let signal = signal(&rt);
        let analysis = rt.analyze(signal, &AnalysisSettings::default()).unwrap();
        let cut = rt.order_cut(analysis, &OrderCutRequest::default()).unwrap();
        let runtime: &dyn FftRuntime = &rt;
        assert_eq!(runtime.signal_info(signal).unwrap().sample_count, 16);
        assert_eq!(runtime.analysis_info(analysis).unwrap().id, analysis);
        assert_eq!(runtime.cut_info(cut).unwrap().kind, CutKind::Order);
        runtime.wait_all(None).unwrap();
        runtime.wait_all(Some(Duration::from_secs(1))).unwrap();
    }

    #[test]
    fn wait_all_times_out_while_jobs_are_held() {
        let rt = MockRuntime::new_held();
        let analysis = analysis(&rt);
        let job = rt.job_for(EntityId::Analysis(analysis)).unwrap();
        let error = rt.wait_all(Some(Duration::from_millis(5))).unwrap_err();
        assert_eq!(error.code(), "busy");
        rt.complete(job).unwrap();
        rt.wait_all(Some(Duration::from_secs(1))).unwrap();
    }
}
