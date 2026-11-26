//! Concrete `FftRuntime`: owned stores plus a bounded Rayon scheduler.
//!
//! Submission is non-blocking. Jobs are placed in a dependency graph and only
//! become runnable once every dependency is `Ready`; a worker never waits for
//! a child job. Completion stores the result, notifies waiters, emits `Done`,
//! and only then enqueues newly satisfied dependents.
//!
//! One state mutex protects the stores and the job graph. It is released for
//! all DSP work, I/O, and event-sink calls, so no lock is held across a long
//! operation or a user callback.

use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use fft_core::cutsession::{LineCutSession, OrderCutSession};
use fft_core::orders::{self, Cut, CutWidth, LineSpec, RpmProfile};
use fft_core::pipeline;
use fft_core::settings::{AnalysisSettings, DataSettings, RpmSource};
use fft_core::sources::FileSource;
use fft_core::spectrogram::{RenderOptions, Spectrogram, SpectrogramGrid};
use fft_core::table::{self, TableFormat};
use fft_core::traits::SignalSource;
use rayon::{ThreadPool, ThreadPoolBuilder};

use crate::data::{
    AnalysisInfo, CutData, CutInfo, CutKind, LineCutData, LineCutRequest, LoadFileRequest,
    LoadSignalRequest, OrderCutData, OrderCutRequest, RpmProfileSpec, SignalInfo, SpectrogramData,
};
use crate::error::RuntimeError;
use crate::events::{JobStatus, RuntimeEvent, RuntimeEventSink};
use crate::ids::{AnalysisId, CutId, EntityId, JobId, SignalId};
use crate::scheduler::{
    current_job, is_worker_thread as worker_thread_active, ActiveTasks, CurrentJobGuard,
    JobContext, JobOutput, JobRecord, JobRunner, RunningTaskGuard, WorkerThreadGuard,
};
use crate::state::{RuntimeState, SignalData, StoredAnalysis, StoredCut, StoredSignal};
use crate::FftRuntime;

/// Default worker count: the machine's available parallelism, at least one.
pub fn default_worker_count() -> usize {
    std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1)
        .max(1)
}

/// Whether the calling thread is a runtime worker currently executing a job.
///
/// Worker callbacks that call [`CoreRuntime::shutdown`] or
/// [`CoreRuntime::wait_all`] get the non-blocking worker contract: they must
/// never wait for queued jobs that only their own worker could run.
pub fn is_worker_thread() -> bool {
    worker_thread_active()
}

pub(crate) struct Inner {
    pool: Arc<ThreadPool>,
    state: Mutex<RuntimeState>,
    wake: Condvar,
    events: Arc<dyn RuntimeEventSink>,
    active: Arc<ActiveTasks>,
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, RuntimeState> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn emit(&self, events: Vec<RuntimeEvent>) {
        for event in events {
            self.events.emit(event);
        }
    }
}

/// Concrete runtime with owned signals/analyses/cuts and a dependency-aware
/// worker pool.
pub struct CoreRuntime {
    inner: Arc<Inner>,
}

impl CoreRuntime {
    /// Create a runtime with `workers` compute threads (`0` is clamped to 1).
    pub fn new(workers: usize, events: Arc<dyn RuntimeEventSink>) -> Self {
        let workers = workers.max(1);
        let pool = build_pool(workers);
        Self {
            inner: Arc::new(Inner {
                pool,
                state: Mutex::new(RuntimeState::default()),
                wake: Condvar::new(),
                events,
                active: Arc::new(ActiveTasks::default()),
            }),
        }
    }

    /// Create a runtime sized from [`default_worker_count`].
    pub fn with_default_workers(events: Arc<dyn RuntimeEventSink>) -> Self {
        Self::new(default_worker_count(), events)
    }

    pub fn worker_count(&self) -> usize {
        self.inner.pool.current_num_threads()
    }

    // ------------------------------------------------------------- loading ---

    /// Submit a file load and return the signal id immediately. `analyze` on
    /// this signal automatically depends on the load job.
    pub fn load_file_with_job(
        &self,
        request: LoadFileRequest,
    ) -> Result<(SignalId, JobId), RuntimeError> {
        self.load_file_with_data_with_job(request, &DataSettings::default())
    }

    /// Like [`CoreRuntime::load_file_with_job`], with explicit table-column
    /// settings. Audio files ignore `data`; adapters can use this to preserve
    /// the table column selected in `AnalysisSettings.data`.
    pub fn load_file_with_data_with_job(
        &self,
        request: LoadFileRequest,
        data: &DataSettings,
    ) -> Result<(SignalId, JobId), RuntimeError> {
        if request.path.as_os_str().is_empty() {
            return Err(RuntimeError::invalid_input("path must not be empty"));
        }
        let path = request.path;
        let name = request.name;
        let data = data.clone();

        let (signal, job, events, should_pump) = {
            let mut state = self.inner.lock();
            if state.scheduler.shutdown {
                return Err(RuntimeError::closed("runtime has been shut down"));
            }
            let signal = state.allocate_signal();
            let job = state.scheduler.allocate_job();
            state.signals.insert(
                signal,
                StoredSignal {
                    data: None,
                    job: Some(job),
                },
            );
            state.entity_jobs.insert(EntityId::Signal(signal), job);

            let runner: JobRunner = Box::new(move |ctx| {
                let signal_data = load_file_data(&path, name.as_deref(), &data, ctx)?;
                Ok(JobOutput::Signal(signal, signal_data))
            });
            let (events, should_pump) = insert_job_locked(&mut state, job, &[], runner);
            (signal, job, events, should_pump)
        };
        self.inner.emit(events);
        if should_pump {
            pump(&self.inner);
        }
        Ok((signal, job))
    }

    /// Store already-decoded samples immediately; no job is created.
    pub fn load_signal(&self, request: LoadSignalRequest) -> Result<SignalId, RuntimeError> {
        if !(request.sample_rate.is_finite() && request.sample_rate > 0.0) {
            return Err(RuntimeError::invalid_input(
                "sample rate must be a positive finite number",
            ));
        }
        let name = request.name.unwrap_or_else(|| "signal".to_string());
        let mut state = self.inner.lock();
        if state.scheduler.shutdown {
            return Err(RuntimeError::closed("runtime has been shut down"));
        }
        let id = state.allocate_signal();
        state.signals.insert(
            id,
            StoredSignal {
                data: Some(Arc::new(SignalData {
                    samples: Arc::new(request.samples),
                    sample_rate: request.sample_rate,
                    name,
                    path: None,
                    table: None,
                })),
                job: None,
            },
        );
        Ok(id)
    }

    // ------------------------------------------------------------ analysis ---

    /// Submit an analysis. Settings are validated synchronously; the result is
    /// materialized through [`CoreRuntime::spectrogram_data`].
    pub fn analyze_with_job(
        &self,
        signal: SignalId,
        settings: &AnalysisSettings,
    ) -> Result<(AnalysisId, JobId), RuntimeError> {
        settings.validate().map_err(|error| {
            RuntimeError::invalid_input(format!("invalid analysis settings: {error}"))
        })?;

        let (analysis, job, events, should_pump) = {
            let mut state = self.inner.lock();
            if state.scheduler.shutdown {
                return Err(RuntimeError::closed("runtime has been shut down"));
            }
            let Some(source) = state.signals.get(&signal) else {
                return Err(RuntimeError::not_found(format!(
                    "signal {signal} not found"
                )));
            };
            let deps: Vec<JobId> = source.job.into_iter().collect();

            let analysis = state.allocate_analysis();
            let job = state.scheduler.allocate_job();
            state.analyses.insert(
                analysis,
                StoredAnalysis {
                    data: None,
                    signal,
                    job,
                },
            );
            state.entity_jobs.insert(EntityId::Analysis(analysis), job);

            let weak = Arc::downgrade(&self.inner);
            let settings = settings.clone();
            let runner: JobRunner = Box::new(move |ctx| {
                let inner = weak
                    .upgrade()
                    .ok_or_else(|| RuntimeError::cancelled("runtime was dropped"))?;
                let source = {
                    let state = inner.lock();
                    state.signal_data(signal)?
                };
                drop(inner);

                let samples = source.samples.as_ref().clone();
                let analysis_result = pipeline::analyze_samples_with_progress(
                    samples,
                    source.sample_rate,
                    &settings,
                    |done, total| ctx.progress(done, total),
                )
                .map_err(|error| map_analysis_error(error, analysis))?;

                Ok(JobOutput::Analysis(
                    analysis,
                    Box::new(SpectrogramData {
                        spectrogram: analysis_result.spectrogram,
                        name: source.name.clone(),
                        path: source.path.clone(),
                        sample_rate: analysis_result.sample_rate,
                        table: source.table.clone(),
                        generation: 1,
                        analysis: settings,
                    }),
                ))
            });
            let (events, should_pump) = insert_job_locked(&mut state, job, &deps, runner);
            (analysis, job, events, should_pump)
        };
        self.inner.emit(events);
        if should_pump {
            pump(&self.inner);
        }
        Ok((analysis, job))
    }

    // ---------------------------------------------------------------- cuts ---

    pub fn order_cut_with_job(
        &self,
        analysis: AnalysisId,
        request: &OrderCutRequest,
    ) -> Result<(CutId, JobId), RuntimeError> {
        let (cut, job, events, should_pump) = {
            let mut state = self.inner.lock();
            if state.scheduler.shutdown {
                return Err(RuntimeError::closed("runtime has been shut down"));
            }
            let Some(parent) = state.analyses.get(&analysis) else {
                return Err(RuntimeError::not_found(format!(
                    "analysis {analysis} not found"
                )));
            };
            let deps = vec![parent.job];

            let cut = state.allocate_cut();
            let job = state.scheduler.allocate_job();
            state.cuts.insert(
                cut,
                StoredCut {
                    data: None,
                    core_cuts: Vec::new(),
                    analysis,
                    job,
                    kind: CutKind::Order,
                    name: "order cuts".to_string(),
                },
            );
            state.entity_jobs.insert(EntityId::Cut(cut), job);

            let weak = Arc::downgrade(&self.inner);
            let request = request.clone();
            let runner: JobRunner = Box::new(move |ctx| {
                if ctx.is_cancelled() {
                    return Err(RuntimeError::cancelled("order cut cancelled"));
                }
                let inner = weak
                    .upgrade()
                    .ok_or_else(|| RuntimeError::cancelled("runtime was dropped"))?;
                let data = {
                    let state = inner.lock();
                    state.analysis_data(analysis)?
                };
                drop(inner);
                let (cut_data, core_cuts) = compute_order_cut(&data, &request, analysis)?;
                Ok(JobOutput::Cut(cut, Arc::new(cut_data), core_cuts))
            });
            let (events, should_pump) = insert_job_locked(&mut state, job, &deps, runner);
            (cut, job, events, should_pump)
        };
        self.inner.emit(events);
        if should_pump {
            pump(&self.inner);
        }
        Ok((cut, job))
    }

    pub fn line_cut_with_job(
        &self,
        analysis: AnalysisId,
        request: &LineCutRequest,
    ) -> Result<(CutId, JobId), RuntimeError> {
        let (cut, job, events, should_pump) = {
            let mut state = self.inner.lock();
            if state.scheduler.shutdown {
                return Err(RuntimeError::closed("runtime has been shut down"));
            }
            let Some(parent) = state.analyses.get(&analysis) else {
                return Err(RuntimeError::not_found(format!(
                    "analysis {analysis} not found"
                )));
            };
            let deps = vec![parent.job];

            let cut = state.allocate_cut();
            let job = state.scheduler.allocate_job();
            state.cuts.insert(
                cut,
                StoredCut {
                    data: None,
                    core_cuts: Vec::new(),
                    analysis,
                    job,
                    kind: CutKind::Line,
                    name: "line cuts".to_string(),
                },
            );
            state.entity_jobs.insert(EntityId::Cut(cut), job);

            let weak = Arc::downgrade(&self.inner);
            let request = request.clone();
            let runner: JobRunner = Box::new(move |ctx| {
                if ctx.is_cancelled() {
                    return Err(RuntimeError::cancelled("line cut cancelled"));
                }
                let inner = weak
                    .upgrade()
                    .ok_or_else(|| RuntimeError::cancelled("runtime was dropped"))?;
                let data = {
                    let state = inner.lock();
                    state.analysis_data(analysis)?
                };
                drop(inner);
                let (cut_data, core_cuts) = compute_line_cut(&data, &request);
                Ok(JobOutput::Cut(cut, Arc::new(cut_data), core_cuts))
            });
            let (events, should_pump) = insert_job_locked(&mut state, job, &deps, runner);
            (cut, job, events, should_pump)
        };
        self.inner.emit(events);
        if should_pump {
            pump(&self.inner);
        }
        Ok((cut, job))
    }

    pub fn cut_points(&self, id: CutId) -> Result<Vec<Cut>, RuntimeError> {
        self.inner.lock().cut_points(id)
    }

    /// Move a ready cut result out of the store when the runtime owns the only
    /// remaining `Arc`.
    ///
    /// A borrowed result (outstanding read views/info snapshots, or another
    /// handle) is rejected with [`RuntimeError::busy`] and stays in the store.
    /// On success the cut/job ids no longer resolve and the caller receives the
    /// unique buffer; there is no hidden copy fallback.
    pub fn take_cut_data(&self, id: CutId) -> Result<CutData, RuntimeError> {
        let mut state = self.inner.lock();
        let data = state.cut_data(id)?;
        // The store holds one `Arc`; `cut_data` adds the second. Any other
        // owner means a read view or handle is still aliasing the payload.
        if Arc::strong_count(&data) != 2 {
            return Err(RuntimeError::busy(format!(
                "cut {id} data is borrowed; close read views before taking ownership"
            )));
        }
        let Some(stored) = state.cuts.remove(&id) else {
            return Err(RuntimeError::not_found(format!("cut {id} not found")));
        };
        state.entity_jobs.remove(&EntityId::Cut(id));
        state.scheduler.jobs.remove(&stored.job);
        // Drop the store's `Arc` before extracting the unique buffers.
        drop(stored);
        self.inner.wake.notify_all();
        drop(state);
        Arc::try_unwrap(data).map_err(|_| {
            RuntimeError::busy(format!(
                "cut {id} data is borrowed during ownership transfer"
            ))
        })
    }

    // --------------------------------------------------------- introspection ---

    pub fn signal_info(&self, id: SignalId) -> Result<SignalInfo, RuntimeError> {
        self.inner.lock().signal_info(id)
    }

    pub fn signal_samples(&self, id: SignalId) -> Result<Arc<Vec<f32>>, RuntimeError> {
        self.inner
            .lock()
            .signal_data(id)
            .map(|data| Arc::clone(&data.samples))
    }

    pub fn analysis_info(&self, id: AnalysisId) -> Result<AnalysisInfo, RuntimeError> {
        // Clone the Arc-backed snapshot under the lock, then scan the
        // full-resolution data without blocking the scheduler.
        let (data, signal) = {
            let state = self.inner.lock();
            let signal = state
                .analyses
                .get(&id)
                .ok_or_else(|| RuntimeError::not_found(format!("analysis {id} not found")))?
                .signal;
            (state.analysis_data(id)?, signal)
        };
        Ok(data.info(id, signal))
    }

    pub fn cut_info(&self, id: CutId) -> Result<CutInfo, RuntimeError> {
        self.inner.lock().cut_info(id)
    }

    /// Live entity/job counts for adapter lifecycle tests. Completed jobs whose
    /// entity is still open remain counted until that entity is closed.
    #[doc(hidden)]
    pub fn live_counts(&self) -> (usize, usize, usize, usize) {
        let state = self.inner.lock();
        (
            state.signals.len(),
            state.analyses.len(),
            state.cuts.len(),
            state.scheduler.jobs.len(),
        )
    }

    // -------------------------------------------------------------- waiting ---

    /// Wait until every active job finishes.
    ///
    /// From a non-worker thread this blocks until all submitted work reaches a
    /// terminal state; a timeout that expires first returns
    /// [`RuntimeError::busy`].
    ///
    /// From a worker thread it is **non-blocking by contract**: the caller's
    /// own job is excluded, and if any other job is still active (running,
    /// queued, or waiting on a dependency) it returns
    /// [`RuntimeError::busy`] immediately. Waiting would deadlock whenever the
    /// remaining work can only run on the caller's worker (for example with
    /// `workers = 1`). A worker with no other active job gets `Ok(())`.
    pub fn wait_all(&self, timeout: Option<Duration>) -> Result<(), RuntimeError> {
        let deadline = timeout.map(|timeout| Instant::now() + timeout);
        let self_job = current_job();
        let from_worker = is_worker_thread();
        let mut state = self.inner.lock();
        loop {
            let active = state
                .scheduler
                .jobs
                .iter()
                .any(|(id, record)| record.status.is_active() && Some(*id) != self_job);
            if !active {
                return Ok(());
            }
            if from_worker {
                return Err(RuntimeError::busy(
                    "wait_all cannot block on other active jobs from a runtime worker",
                ));
            }
            state = match deadline {
                Some(deadline) => {
                    let now = Instant::now();
                    if now >= deadline {
                        return Err(RuntimeError::busy("timed out waiting for jobs"));
                    }
                    self.inner
                        .wake
                        .wait_timeout(state, deadline - now)
                        .unwrap_or_else(|error| error.into_inner())
                        .0
                }
                None => self
                    .inner
                    .wake
                    .wait(state)
                    .unwrap_or_else(|error| error.into_inner()),
            };
        }
    }

    /// Cancel every active job and stop accepting new work.
    ///
    /// A non-worker caller waits for every spawned task (queued or running) to
    /// unwind. A worker caller must not wait for queued work, because with a
    /// single worker that task cannot start until the caller returns. Worker
    /// shutdown therefore sets the flag, cancels queued/running jobs, drops
    /// their runners, and returns; it waits only for already-running peer
    /// tasks, never for the caller itself or spawned-but-queued tasks. If
    /// another worker is already performing that wait, this call returns
    /// without waiting so two workers cannot wait for each other.
    ///
    /// Safe to call more than once and from inside a worker job.
    pub fn shutdown(&self) {
        let (events, from_worker) = {
            let mut state = self.inner.lock();
            state.scheduler.shutdown = true;
            let active: Vec<JobId> = state
                .scheduler
                .jobs
                .iter()
                .filter(|(_, record)| record.status.is_active())
                .map(|(id, _)| *id)
                .collect();
            let mut events = Vec::new();
            for job in active {
                events.extend(cascade_cancel(&mut state, job, "runtime shut down"));
            }
            state.scheduler.ready.clear();
            for record in state.scheduler.jobs.values_mut() {
                record.runner = None;
            }
            self.inner.wake.notify_all();
            (events, is_worker_thread())
        };
        self.inner.emit(events);
        if from_worker {
            if let Some(_waiter) = self.inner.active.begin_worker_shutdown_wait() {
                // `running` includes this caller, so a maximum of one means no
                // peer task is still executing. Queued tasks are deliberately
                // ignored: they may need this caller's worker to start.
                self.inner.active.wait_running_at_most(1, None);
            }
        } else {
            self.inner.active.wait_at_most(0, None);
        }
    }

    fn notify_and_emit(&self, events: Vec<RuntimeEvent>) {
        self.inner.wake.notify_all();
        self.inner.emit(events);
    }

    #[cfg(test)]
    pub(crate) fn submit_test_job(
        &self,
        deps: &[JobId],
        runner: JobRunner,
    ) -> Result<JobId, RuntimeError> {
        let (job, events, should_pump) = {
            let mut state = self.inner.lock();
            if state.scheduler.shutdown {
                return Err(RuntimeError::closed("runtime has been shut down"));
            }
            let job = state.scheduler.allocate_job();
            let (events, should_pump) = insert_job_locked(&mut state, job, deps, runner);
            (job, events, should_pump)
        };
        self.inner.emit(events);
        if should_pump {
            pump(&self.inner);
        }
        Ok(job)
    }
}

impl Drop for CoreRuntime {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl FftRuntime for CoreRuntime {
    fn load_file(&self, request: LoadFileRequest) -> Result<SignalId, RuntimeError> {
        self.load_file_with_job(request).map(|(signal, _)| signal)
    }

    fn load_signal(&self, request: LoadSignalRequest) -> Result<SignalId, RuntimeError> {
        CoreRuntime::load_signal(self, request)
    }

    fn analyze(
        &self,
        signal: SignalId,
        settings: &AnalysisSettings,
    ) -> Result<AnalysisId, RuntimeError> {
        self.analyze_with_job(signal, settings)
            .map(|(analysis, _)| analysis)
    }

    fn job_for(&self, id: EntityId) -> Option<JobId> {
        self.inner.lock().job_for(id)
    }

    fn job_status(&self, job: JobId) -> JobStatus {
        self.inner.lock().job_status(job)
    }

    fn wait(&self, job: JobId, timeout: Option<Duration>) -> Result<(), RuntimeError> {
        let deadline = timeout.map(|timeout| Instant::now() + timeout);
        let mut state = self.inner.lock();
        loop {
            let (status, error) = match state.scheduler.jobs.get(&job) {
                Some(record) => (record.status, record.error.clone()),
                None => {
                    return Err(
                        RuntimeError::cancelled("job is closed").with_context(job.to_string())
                    )
                }
            };
            match status {
                JobStatus::Ready => return Ok(()),
                JobStatus::Failed => {
                    return Err(error.unwrap_or_else(|| {
                        RuntimeError::analysis_failed("job failed without an error")
                    }))
                }
                JobStatus::Closed => {
                    return Err(error.unwrap_or_else(|| RuntimeError::cancelled("job is closed")))
                }
                JobStatus::Pending | JobStatus::Running => {}
            }
            state = match deadline {
                Some(deadline) => {
                    let now = Instant::now();
                    if now >= deadline {
                        return Err(RuntimeError::busy("timed out waiting for job")
                            .with_context(job.to_string()));
                    }
                    self.inner
                        .wake
                        .wait_timeout(state, deadline - now)
                        .unwrap_or_else(|error| error.into_inner())
                        .0
                }
                None => self
                    .inner
                    .wake
                    .wait(state)
                    .unwrap_or_else(|error| error.into_inner()),
            };
        }
    }

    fn cancel(&self, job: JobId) -> Result<(), RuntimeError> {
        let events = {
            let mut state = self.inner.lock();
            let Some(record) = state.scheduler.jobs.get(&job) else {
                return Err(RuntimeError::not_found(format!("job {job} not found")));
            };
            if record.status.is_terminal() {
                return Ok(());
            }
            cascade_cancel(&mut state, job, "job cancelled")
        };
        self.notify_and_emit(events);
        Ok(())
    }

    fn spectrogram_grid(
        &self,
        id: AnalysisId,
        max_frames: usize,
        max_bins: usize,
    ) -> Result<SpectrogramGrid, RuntimeError> {
        let data = self.inner.lock().analysis_data(id)?;
        Ok(data.spectrogram.to_grid(max_frames, max_bins))
    }

    fn spectrogram_data(&self, id: AnalysisId) -> Result<Arc<SpectrogramData>, RuntimeError> {
        self.inner.lock().analysis_data(id)
    }

    fn order_cut(&self, id: AnalysisId, request: &OrderCutRequest) -> Result<CutId, RuntimeError> {
        self.order_cut_with_job(id, request).map(|(cut, _)| cut)
    }

    fn line_cut(&self, id: AnalysisId, request: &LineCutRequest) -> Result<CutId, RuntimeError> {
        self.line_cut_with_job(id, request).map(|(cut, _)| cut)
    }

    fn cut_data(&self, id: CutId) -> Result<Arc<CutData>, RuntimeError> {
        self.inner.lock().cut_data(id)
    }

    fn signal_info(&self, id: SignalId) -> Result<SignalInfo, RuntimeError> {
        CoreRuntime::signal_info(self, id)
    }

    fn analysis_info(&self, id: AnalysisId) -> Result<AnalysisInfo, RuntimeError> {
        CoreRuntime::analysis_info(self, id)
    }

    fn cut_info(&self, id: CutId) -> Result<CutInfo, RuntimeError> {
        CoreRuntime::cut_info(self, id)
    }

    fn take_cut_data(&self, id: CutId) -> Result<CutData, RuntimeError> {
        CoreRuntime::take_cut_data(self, id)
    }

    fn wait_all(&self, timeout: Option<Duration>) -> Result<(), RuntimeError> {
        CoreRuntime::wait_all(self, timeout)
    }

    fn export_csv(&self, id: CutId, path: &Path) -> Result<(), RuntimeError> {
        let cuts = self.cut_points(id)?;
        orders::write_csv(path, &cuts).map_err(|error| {
            RuntimeError::export(error.to_string()).with_context(path.display().to_string())
        })
    }

    fn save_image(
        &self,
        id: AnalysisId,
        path: &Path,
        options: &RenderOptions,
    ) -> Result<(), RuntimeError> {
        if options.width == 0 || options.height == 0 {
            return Err(RuntimeError::invalid_input(
                "image width and height must be greater than zero",
            ));
        }
        let data = self.spectrogram_data(id)?;
        let image = data.spectrogram.render(options);
        let mut rgb = Vec::with_capacity(image.width * image.height * 3);
        for pixel in &image.pixels {
            rgb.push(((pixel >> 16) & 0xFF) as u8);
            rgb.push(((pixel >> 8) & 0xFF) as u8);
            rgb.push((pixel & 0xFF) as u8);
        }
        image::save_buffer_with_format(
            path,
            &rgb,
            image.width as u32,
            image.height as u32,
            image::ColorType::Rgb8,
            image::ImageFormat::Png,
        )
        .map_err(|error| {
            RuntimeError::export(error.to_string()).with_context(path.display().to_string())
        })
    }

    fn close(&self, id: EntityId) -> Result<(), RuntimeError> {
        let events = {
            let mut state = self.inner.lock();
            let events = match id {
                EntityId::Signal(signal) => remove_signal_locked(&mut state, signal),
                EntityId::Analysis(analysis) => remove_analysis_locked(&mut state, analysis),
                EntityId::Cut(cut) => remove_cut_locked(&mut state, cut),
            };
            self.inner.wake.notify_all();
            events
        };
        self.inner.emit(events);
        Ok(())
    }
}

// ------------------------------------------------------------------ helpers ---

fn build_pool(workers: usize) -> Arc<ThreadPool> {
    let workers = workers.max(1);
    match ThreadPoolBuilder::new()
        .num_threads(workers)
        .thread_name(|index| format!("fft-runtime-{index}"))
        .build()
    {
        Ok(pool) => Arc::new(pool),
        Err(_) => Arc::new(
            ThreadPoolBuilder::new()
                .num_threads(1)
                .thread_name(|index| format!("fft-runtime-{index}"))
                .build()
                .expect("one-thread Rayon pool"),
        ),
    }
}

/// Insert a job into the graph, linking active dependencies. Returns any
/// immediate failure/cancellation events and whether the job was queued.
fn insert_job_locked(
    state: &mut RuntimeState,
    job: JobId,
    deps: &[JobId],
    runner: JobRunner,
) -> (Vec<RuntimeEvent>, bool) {
    let mut seen = HashSet::new();
    let mut deps_remaining = 0usize;
    let mut active_deps = Vec::new();
    let mut inherited: Option<RuntimeError> = None;

    for &dep in deps {
        if !seen.insert(dep) {
            continue;
        }
        match state.scheduler.jobs.get(&dep) {
            Some(record) => {
                match record.status {
                    JobStatus::Ready => {}
                    JobStatus::Pending | JobStatus::Running => {
                        deps_remaining += 1;
                        active_deps.push(dep);
                    }
                    JobStatus::Failed => {
                        if inherited.is_none() {
                            inherited = Some(record.error.clone().unwrap_or_else(|| {
                                RuntimeError::analysis_failed("dependency failed")
                            }));
                        }
                    }
                    JobStatus::Closed => {
                        if inherited.is_none() {
                            inherited = Some(record.error.clone().unwrap_or_else(|| {
                                RuntimeError::cancelled("dependency was closed")
                            }));
                        }
                    }
                }
            }
            None => {
                if inherited.is_none() {
                    inherited = Some(RuntimeError::cancelled("dependency was closed"));
                }
            }
        }
    }

    state.scheduler.jobs.insert(
        job,
        JobRecord {
            status: JobStatus::Pending,
            error: None,
            cancel: Arc::new(AtomicBool::new(false)),
            runner: Some(runner),
            deps_remaining,
            dependents: Vec::new(),
            scheduled: false,
            started: None,
        },
    );
    for dep in active_deps {
        if let Some(parent) = state.scheduler.jobs.get_mut(&dep) {
            parent.dependents.push(job);
        }
    }

    if let Some(error) = inherited {
        let cancelled = error.code() == "cancelled" || error.code() == "closed";
        let record = state
            .scheduler
            .jobs
            .get_mut(&job)
            .expect("job was just inserted");
        record.status = if cancelled {
            JobStatus::Closed
        } else {
            JobStatus::Failed
        };
        record.error = Some(error.clone());
        record.cancel.store(true, Ordering::Relaxed);
        record.runner = None;
        let event = if cancelled {
            RuntimeEvent::Cancelled { job }
        } else {
            RuntimeEvent::Failed {
                job,
                message: error.to_string(),
            }
        };
        return (vec![event], false);
    }

    if deps_remaining == 0 {
        let queued = state.scheduler.enqueue_ready(&[job]);
        (Vec::new(), queued > 0)
    } else {
        (Vec::new(), false)
    }
}

/// Spawn Rayon tasks for every runnable job in the ready queue. A worker never
/// waits for dependents, so this can safely be called from a worker.
fn pump(inner: &Arc<Inner>) {
    loop {
        let next = {
            let mut state = inner.lock();
            if state.scheduler.shutdown {
                state.scheduler.ready.clear();
                return;
            }
            let Some(job) = state.scheduler.ready.pop_front() else {
                return;
            };
            let runnable = match state.scheduler.jobs.get_mut(&job) {
                Some(record)
                    if record.status == JobStatus::Pending
                        && !record.scheduled
                        && record.runner.is_some() =>
                {
                    record.scheduled = true;
                    // Count the task while still holding the state lock, so a
                    // concurrent shutdown cannot observe zero active tasks
                    // between scheduling and spawning this one.
                    inner.active.begin();
                    true
                }
                _ => false,
            };
            runnable.then_some(job)
        };
        if let Some(job) = next {
            spawn_job(inner, job);
        }
    }
}

fn spawn_job(inner: &Arc<Inner>, job: JobId) {
    let pool = Arc::clone(&inner.pool);
    let task_inner = Arc::clone(inner);
    let active = Arc::clone(&inner.active);
    pool.spawn(move || {
        let _running = RunningTaskGuard::enter(&active);
        let _worker = WorkerThreadGuard::enter();
        let _current = CurrentJobGuard::enter(job);
        let outcome =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_job(&task_inner, job)));
        if outcome.is_err() {
            finish_job(
                &task_inner,
                job,
                Err(RuntimeError::internal("job panicked")),
            );
        }
        // Drop the strong runtime reference before registering completion: a
        // shutdown wait can then guarantee no task still owns `Inner`.
        drop(task_inner);
        active.end();
    });
}

fn run_job(inner: &Arc<Inner>, job: JobId) {
    let (runner, cancel, started) = {
        let mut state = inner.lock();
        let Some(record) = state.scheduler.jobs.get_mut(&job) else {
            return;
        };
        if record.status != JobStatus::Pending || !record.scheduled {
            return;
        }
        let Some(runner) = record.runner.take() else {
            return;
        };
        record.status = JobStatus::Running;
        record.started = Some(Instant::now());
        (
            runner,
            Arc::clone(&record.cancel),
            record.started.unwrap_or_else(Instant::now),
        )
    };
    let context = JobContext::new(job, cancel, started, Arc::clone(&inner.events));
    let result = runner(&context);
    finish_job(inner, job, result);
}

fn finish_job(inner: &Arc<Inner>, job: JobId, result: Result<JobOutput, RuntimeError>) {
    let mut events = Vec::new();
    let mut ready_children = Vec::new();
    let mut success = false;

    {
        let mut state = inner.lock();
        let Some(record) = state.scheduler.jobs.get(&job) else {
            return;
        };
        if record.status != JobStatus::Running {
            return; // closed while running: the snapshot is discarded
        }
        let was_cancelled = record.cancel.load(Ordering::Relaxed);

        match result {
            Ok(output) => {
                let applied = state.apply_output(output);
                let record = state
                    .scheduler
                    .jobs
                    .get_mut(&job)
                    .expect("job still exists");
                if applied {
                    record.status = JobStatus::Ready;
                    record.error = None;
                    events.push(RuntimeEvent::Done { job });
                    success = true;
                } else {
                    record.status = JobStatus::Closed;
                    record.error = Some(RuntimeError::cancelled(
                        "entity was closed while the job ran",
                    ));
                }
            }
            Err(error) => {
                let cancelled =
                    was_cancelled || error.code() == "cancelled" || error.code() == "closed";
                let record = state
                    .scheduler
                    .jobs
                    .get_mut(&job)
                    .expect("job still exists");
                if cancelled {
                    record.status = JobStatus::Closed;
                    record.error = Some(if error.code() == "cancelled" {
                        error.clone()
                    } else {
                        RuntimeError::cancelled(error.to_string())
                    });
                    events.push(RuntimeEvent::Cancelled { job });
                } else {
                    record.status = JobStatus::Failed;
                    record.error = Some(error.clone());
                    events.push(RuntimeEvent::Failed {
                        job,
                        message: error.to_string(),
                    });
                }
                record.cancel.store(true, Ordering::Relaxed);
                fail_dependents_locked(&mut state, job, &error, cancelled, &mut events);
            }
        }
        if success {
            ready_children = state.scheduler.release_dependents(&[job]);
        }
        inner.wake.notify_all();
    }

    // Completion is visible to waiters and the Done event is emitted before
    // dependents are placed on the ready queue.
    inner.emit(events);
    if success && !ready_children.is_empty() {
        {
            let mut state = inner.lock();
            if !state.scheduler.shutdown {
                state.scheduler.enqueue_ready(&ready_children);
            } else {
                state.scheduler.ready.clear();
            }
        }
        pump(inner);
    }
}

fn fail_dependents_locked(
    state: &mut RuntimeState,
    parent: JobId,
    error: &RuntimeError,
    cancelled: bool,
    events: &mut Vec<RuntimeEvent>,
) {
    let dependents = match state.scheduler.jobs.get_mut(&parent) {
        Some(record) => std::mem::take(&mut record.dependents),
        None => Vec::new(),
    };
    for child in dependents {
        let active = match state.scheduler.jobs.get(&child) {
            Some(record) => record.status.is_active(),
            None => false,
        };
        if !active {
            continue;
        }
        {
            let record = state
                .scheduler
                .jobs
                .get_mut(&child)
                .expect("dependent still exists");
            if cancelled {
                record.status = JobStatus::Closed;
                record.error = Some(RuntimeError::cancelled("dependency was cancelled"));
            } else {
                record.status = JobStatus::Failed;
                record.error = Some(error.clone());
            }
            record.cancel.store(true, Ordering::Relaxed);
            record.runner = None;
        }
        if cancelled {
            events.push(RuntimeEvent::Cancelled { job: child });
        } else {
            events.push(RuntimeEvent::Failed {
                job: child,
                message: error.to_string(),
            });
        }
        fail_dependents_locked(state, child, error, cancelled, events);
    }
}

fn cascade_cancel(state: &mut RuntimeState, job: JobId, message: &str) -> Vec<RuntimeEvent> {
    fn cancel_one(
        state: &mut RuntimeState,
        job: JobId,
        message: &str,
        events: &mut Vec<RuntimeEvent>,
    ) {
        let dependents = {
            let Some(record) = state.scheduler.jobs.get_mut(&job) else {
                return;
            };
            if !record.status.is_active() {
                return;
            }
            record.status = JobStatus::Closed;
            record.error = Some(RuntimeError::cancelled(message));
            record.cancel.store(true, Ordering::Relaxed);
            record.runner = None;
            std::mem::take(&mut record.dependents)
        };
        events.push(RuntimeEvent::Cancelled { job });
        for child in dependents {
            cancel_one(state, child, message, events);
        }
    }

    let mut events = Vec::new();
    cancel_one(state, job, message, &mut events);
    events
}

fn remove_signal_locked(state: &mut RuntimeState, signal: SignalId) -> Vec<RuntimeEvent> {
    let Some(stored) = state.signals.remove(&signal) else {
        return Vec::new();
    };
    state.entity_jobs.remove(&EntityId::Signal(signal));
    let mut events = Vec::new();
    if let Some(job) = stored.job {
        events.extend(cascade_cancel(state, job, "signal closed"));
        state.scheduler.jobs.remove(&job);
    }
    let analyses: Vec<AnalysisId> = state
        .analyses
        .iter()
        .filter(|(_, analysis)| analysis.signal == signal)
        .map(|(id, _)| *id)
        .collect();
    for analysis in analyses {
        events.extend(remove_analysis_locked(state, analysis));
    }
    events
}

fn remove_analysis_locked(state: &mut RuntimeState, analysis: AnalysisId) -> Vec<RuntimeEvent> {
    let Some(stored) = state.analyses.remove(&analysis) else {
        return Vec::new();
    };
    state.entity_jobs.remove(&EntityId::Analysis(analysis));
    let mut events = Vec::new();
    events.extend(cascade_cancel(state, stored.job, "analysis closed"));
    state.scheduler.jobs.remove(&stored.job);
    let cuts: Vec<CutId> = state
        .cuts
        .iter()
        .filter(|(_, cut)| cut.analysis == analysis)
        .map(|(id, _)| *id)
        .collect();
    for cut in cuts {
        events.extend(remove_cut_locked(state, cut));
    }
    events
}

fn remove_cut_locked(state: &mut RuntimeState, cut: CutId) -> Vec<RuntimeEvent> {
    let Some(stored) = state.cuts.remove(&cut) else {
        return Vec::new();
    };
    state.entity_jobs.remove(&EntityId::Cut(cut));
    let mut events = Vec::new();
    events.extend(cascade_cancel(state, stored.job, "cut closed"));
    state.scheduler.jobs.remove(&stored.job);
    events
}

// ----------------------------------------------------------- data loading ---

fn load_file_data(
    path: &Path,
    requested_name: Option<&str>,
    data_settings: &DataSettings,
    context: &JobContext,
) -> Result<SignalData, RuntimeError> {
    let name = requested_name
        .map(str::to_string)
        .or_else(|| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "signal".to_string());

    if let Some(format) = TableFormat::from_path(path) {
        let signal = table::load(path, format, data_settings).map_err(|error| {
            let message = error.to_string();
            if message.contains("built without") {
                RuntimeError::unsupported(message)
            } else {
                RuntimeError::analysis_failed(message).with_context(path.display().to_string())
            }
        })?;
        if signal.samples.is_empty() {
            return Err(RuntimeError::analysis_failed("no samples decoded")
                .with_context(path.display().to_string()));
        }
        return Ok(SignalData {
            samples: Arc::new(signal.samples),
            sample_rate: signal.sample_rate,
            name,
            path: Some(path.to_path_buf()),
            table: Some(signal.info),
        });
    }

    let mut source = FileSource::open(path).map_err(|error| {
        RuntimeError::analysis_failed(format!("failed to open audio: {error}"))
            .with_context(path.display().to_string())
    })?;
    let sample_rate = source.sample_rate();
    let total = source.expected_samples().map(|count| count as usize);
    let capacity = total.unwrap_or(0).max(4096);
    let mut samples = Vec::with_capacity(capacity);
    let mut buffer = [0f32; 8192];
    loop {
        if context.is_cancelled() {
            return Err(RuntimeError::cancelled("load cancelled"));
        }
        let read = source.read(&mut buffer).map_err(|error| {
            RuntimeError::analysis_failed(format!("failed to decode audio: {error}"))
                .with_context(path.display().to_string())
        })?;
        if read == 0 {
            break;
        }
        samples.extend_from_slice(&buffer[..read]);
        if total.is_some() && samples.len() % (8 * 8192) < read {
            if !context.progress(samples.len(), total) {
                return Err(RuntimeError::cancelled("load cancelled"));
            }
        }
    }
    if samples.is_empty() {
        return Err(RuntimeError::analysis_failed("no samples decoded")
            .with_context(path.display().to_string()));
    }
    Ok(SignalData {
        samples: Arc::new(samples),
        sample_rate,
        name,
        path: Some(path.to_path_buf()),
        table: None,
    })
}

fn map_analysis_error(error: fft_core::Error, analysis: AnalysisId) -> RuntimeError {
    let message = error.to_string();
    if message.contains("cancelled") {
        RuntimeError::cancelled(message)
    } else {
        RuntimeError::analysis_failed(message).with_context(analysis.to_string())
    }
}

// -------------------------------------------------------------- cut maths ---

fn resolve_rpm(request: &OrderCutRequest, spec: &Spectrogram) -> Result<RpmProfile, RuntimeError> {
    if let Some(profile) = &request.rpm {
        return match profile.clone() {
            RpmProfileSpec::Constant(rpm) => {
                ensure_finite_rpm(rpm, "RPM")?;
                Ok(RpmProfile::constant(rpm))
            }
            RpmProfileSpec::Ramp {
                t0_s,
                t1_s,
                rpm0,
                rpm1,
            } => {
                ensure_finite_rpm(t0_s, "ramp start time")?;
                ensure_finite_rpm(t1_s, "ramp end time")?;
                ensure_finite_rpm(rpm0, "ramp start RPM")?;
                ensure_finite_rpm(rpm1, "ramp end RPM")?;
                Ok(RpmProfile::ramp(t0_s, t1_s, rpm0, rpm1))
            }
            RpmProfileSpec::Csv(path) => RpmProfile::from_csv(&path).map_err(|error| {
                RuntimeError::invalid_input(format!("failed to read RPM profile: {error}"))
                    .with_context(path.display().to_string())
            }),
            RpmProfileSpec::Points(points) => {
                let points = points
                    .into_iter()
                    .map(|point| (point.time_s, point.rpm))
                    .collect();
                RpmProfile::from_points(points).map_err(|error| {
                    RuntimeError::invalid_input(format!("invalid RPM points: {error}"))
                })
            }
        };
    }

    let rpm = &request.settings.rpm;
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
            let path = rpm.csv.as_ref().ok_or_else(|| {
                RuntimeError::invalid_input("RPM source is CSV but no file is configured")
            })?;
            RpmProfile::from_csv(path).map_err(|error| {
                RuntimeError::invalid_input(format!("failed to read RPM profile: {error}"))
                    .with_context(path.display().to_string())
            })
        }
    }
}

fn ensure_finite_rpm(value: f64, label: &str) -> Result<(), RuntimeError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(RuntimeError::invalid_input(format!(
            "{label} must be a finite number"
        )))
    }
}

fn compute_order_cut(
    data: &SpectrogramData,
    request: &OrderCutRequest,
    analysis: AnalysisId,
) -> Result<(CutData, Vec<Cut>), RuntimeError> {
    let spec = &data.spectrogram;
    let rpm = resolve_rpm(request, spec)?;
    let orders = orders::parse_orders(&request.settings.orders);
    let width = match request.settings.order_width_hz {
        Some(hz) => CutWidth::Hz(hz),
        None => CutWidth::Orders(request.settings.order_width),
    };
    let matrix = orders::order_matrix(spec, &rpm, &orders, width);

    let mut session = OrderCutSession::new(request.settings.clone());
    session.compute_with_profile(spec, &rpm);
    let core_cuts = session.cuts().to_vec();

    let order_data = OrderCutData::new(
        matrix.values,
        matrix.times,
        matrix.orders,
        matrix.frequencies,
    );
    if !order_data.is_consistent() {
        return Err(
            RuntimeError::analysis_failed("order matrix has inconsistent dimensions")
                .with_context(analysis.to_string()),
        );
    }
    Ok((CutData::Order(order_data), core_cuts))
}

fn compute_line_cut(data: &SpectrogramData, request: &LineCutRequest) -> (CutData, Vec<Cut>) {
    let spec = &data.spectrogram;
    let mut session = LineCutSession::new(request.settings.clone());
    session.set_lines(request.lines.clone());
    session.compute(spec);
    let core_cuts = session.cuts().to_vec();

    let tracks: Vec<LineCutData> = core_cuts
        .iter()
        .enumerate()
        .map(|(index, cut)| {
            let line = request.lines.get(index).cloned().unwrap_or(LineSpec {
                t0: 0.0,
                f0: 0.0,
                t1: 0.0,
                f1: 0.0,
                log_f: false,
            });
            let mut times = Vec::with_capacity(cut.points.len());
            let mut frequencies = Vec::with_capacity(cut.points.len());
            let mut fractions = Vec::with_capacity(cut.points.len());
            let mut db = Vec::with_capacity(cut.points.len());
            for point in &cut.points {
                times.push(point.time_s);
                frequencies.push(point.freq_hz);
                fractions.push(line_fraction(&line, point.time_s, point.freq_hz));
                db.push(point.db);
            }
            LineCutData::new(times, frequencies, fractions, db)
        })
        .collect();
    (CutData::Line(tracks), core_cuts)
}

/// Normalized 0..=1 position of a cut point along the requested line. Time is
/// linear regardless of the frequency mapping, so use it whenever the line has
/// a time extent; vertical lines fall back to their frequency mapping.
fn line_fraction(line: &LineSpec, time_s: f64, freq_hz: f64) -> f64 {
    let dt = line.t1 - line.t0;
    if dt.abs() > 1e-12 {
        return ((time_s - line.t0) / dt).clamp(0.0, 1.0);
    }
    if line.log_f && line.f0 > 0.0 && line.f1 > 0.0 {
        let span = (line.f1 / line.f0).ln();
        if span.abs() > 1e-12 {
            return (((freq_hz / line.f0).ln()) / span).clamp(0.0, 1.0);
        }
    }
    let df = line.f1 - line.f0;
    if df.abs() > 1e-12 {
        ((freq_hz - line.f0) / df).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::sync::Mutex as StdMutex;
    use std::thread;

    use fft_core::orders::CutKind as CoreCutKind;
    use fft_core::settings::{LineCutSettings, OrderCutSettings, RpmSettings};
    use fft_core::testutil::{tones, write_wav};

    use crate::data::{LineCutRequest, OrderCutRequest, RpmPoint};
    use crate::scheduler::{JobContext, JobOutput};

    #[derive(Clone, Default)]
    struct CaptureSink(Arc<StdMutex<Vec<RuntimeEvent>>>);

    impl RuntimeEventSink for CaptureSink {
        fn emit(&self, event: RuntimeEvent) {
            self.0
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(event);
        }
    }

    impl CaptureSink {
        fn events(&self) -> Vec<RuntimeEvent> {
            self.0
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone()
        }
    }

    fn make_runtime(workers: usize) -> (CoreRuntime, CaptureSink) {
        let sink = CaptureSink::default();
        let runtime = CoreRuntime::new(workers, Arc::new(sink.clone()));
        (runtime, sink)
    }

    fn settings() -> AnalysisSettings {
        AnalysisSettings {
            frame: 512,
            hop: Some(128),
            pad: 2,
            ..Default::default()
        }
    }

    fn small_settings() -> AnalysisSettings {
        AnalysisSettings {
            frame: 256,
            hop: Some(64),
            pad: 1,
            ..Default::default()
        }
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("fft_runtime_{}_{}", std::process::id(), tag));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("create test dir");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A test job whose runner waits until `release` works or cancellation is
    /// requested; it signals `started` once it is `Running`.
    fn blocking_job(
        runtime: &CoreRuntime,
        label: &'static str,
    ) -> (JobId, mpsc::Sender<()>, mpsc::Receiver<()>) {
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let job = runtime
            .submit_test_job(
                &[],
                Box::new(move |ctx: &JobContext| {
                    started_tx.send(()).expect("signal start");
                    while !ctx.is_cancelled() {
                        if release_rx.try_recv().is_ok() {
                            break;
                        }
                        thread::sleep(Duration::from_millis(1));
                    }
                    if ctx.is_cancelled() {
                        return Err(RuntimeError::cancelled(format!("{label} cancelled")));
                    }
                    Ok(JobOutput::None)
                }),
            )
            .expect("submit blocker");
        job_with_start(job, release_tx, started_rx)
    }

    fn job_with_start(
        job: JobId,
        release: mpsc::Sender<()>,
        started: mpsc::Receiver<()>,
    ) -> (JobId, mpsc::Sender<()>, mpsc::Receiver<()>) {
        started
            .recv_timeout(Duration::from_secs(5))
            .expect("blocking job started");
        (job, release, started)
    }

    fn order_request() -> OrderCutRequest {
        OrderCutRequest {
            settings: OrderCutSettings {
                orders: "1,2".to_string(),
                order_width: 0.1,
                order_width_hz: None,
                freqs: "440".to_string(),
                freq_width_hz: 5.0,
                smooth_s: 0.0,
                rpm: RpmSettings {
                    source: RpmSource::Constant,
                    constant: 3000.0,
                    ..Default::default()
                },
                ..Default::default()
            },
            rpm: None,
        }
    }

    fn line_request() -> LineCutRequest {
        LineCutRequest::new(
            LineCutSettings {
                line_width_hz: 0.0,
                samples: Some(64),
                ..Default::default()
            },
            vec![
                LineSpec {
                    t0: 0.05,
                    f0: 0.0,
                    t1: 0.05,
                    f1: 7_000.0,
                    log_f: false,
                },
                LineSpec {
                    t0: 0.0,
                    f0: 440.0,
                    t1: 0.2,
                    f1: 440.0,
                    log_f: false,
                },
            ],
        )
    }

    #[test]
    fn load_file_analyze_matches_core_and_reaches_a_grid() {
        let samples = tones(16_000, 0.2, &[(440.0, 0.5), (1_200.0, 0.3)]);
        let path = write_wav("runtime_core_equiv", 16_000, &samples);
        let (runtime, _sink) = make_runtime(2);

        let (signal, load_job) = runtime
            .load_file_with_job(LoadFileRequest::new(&path))
            .expect("submit load");
        // A worker can pick the job up before this assertion runs; either
        // state proves it was accepted without already completing.
        assert!(matches!(
            runtime.job_status(load_job),
            JobStatus::Pending | JobStatus::Running
        ));
        runtime
            .wait(load_job, Some(Duration::from_secs(10)))
            .expect("load");
        assert_eq!(runtime.job_status(load_job), JobStatus::Ready);

        let signal_info = runtime.signal_info(signal).expect("signal info");
        assert_eq!(signal_info.sample_rate, 16_000.0);
        assert_eq!(signal_info.sample_count, samples.len());
        assert_eq!(signal_info.path.as_deref(), Some(path.as_path()));

        let (analysis, analysis_job) = runtime
            .analyze_with_job(signal, &settings())
            .expect("submit analysis");
        assert_eq!(
            runtime.job_for(EntityId::Analysis(analysis)),
            Some(analysis_job)
        );
        runtime
            .wait(analysis_job, Some(Duration::from_secs(10)))
            .expect("analysis");
        assert_eq!(runtime.job_status(analysis_job), JobStatus::Ready);

        let data = runtime.spectrogram_data(analysis).expect("data");
        let core = fft_core::pipeline::analyze(&path, &settings()).expect("core analysis");
        assert_eq!(data.sample_rate, core.sample_rate);
        assert_eq!(data.frames(), core.spectrogram.frames());
        assert_eq!(data.bins(), core.spectrogram.bins());
        assert_eq!(data.data(), core.spectrogram.data());
        assert_eq!(data.frames() * data.bins(), data.data().len());

        let grid = runtime
            .spectrogram_grid(analysis, 8, 16)
            .expect("transport grid");
        assert!(grid.frames > 0 && grid.frames <= 8);
        assert!(grid.bins > 0 && grid.bins <= 16);
        assert_eq!(grid.data.len(), grid.frames * grid.bins);

        let info = runtime.analysis_info(analysis).expect("analysis info");
        assert_eq!(info.signal, signal);
        assert_eq!(info.frames, data.frames());
        assert_eq!(info.bins, data.bins());
        assert_eq!(info.name, path.file_name().unwrap().to_string_lossy());

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn table_load_uses_the_selected_column_and_exposes_it_on_the_analysis() {
        let dir = TempDir::new("table_column");
        let path = dir.path().join("signal.csv");
        std::fs::write(
            &path,
            "time,alpha,beta\n0.000,1,10\n0.001,2,20\n0.002,3,30\n0.003,4,40\n",
        )
        .expect("write table");
        let (runtime, _sink) = make_runtime(1);
        let data = DataSettings {
            column: Some("beta".into()),
            sample_rate: Some(1_000.0),
            remove_mean: false,
            ..Default::default()
        };

        let (signal, load_job) = runtime
            .load_file_with_data_with_job(LoadFileRequest::new(&path), &data)
            .expect("submit table load");
        runtime
            .wait(load_job, Some(Duration::from_secs(10)))
            .expect("load table");
        let samples = runtime.signal_samples(signal).expect("samples");
        assert_eq!(samples.as_slice(), &[10.0, 20.0, 30.0, 40.0]);
        assert_eq!(
            runtime.signal_info(signal).expect("info").sample_rate,
            1_000.0
        );

        let (analysis, analysis_job) = runtime
            .analyze_with_job(signal, &settings())
            .expect("submit analysis");
        runtime
            .wait(analysis_job, Some(Duration::from_secs(10)))
            .expect("analyze table signal");
        let analysis_data = runtime.spectrogram_data(analysis).expect("analysis data");
        assert_eq!(
            analysis_data.table.as_ref().expect("table metadata").signal,
            "beta"
        );
        runtime.shutdown();
    }

    #[test]
    fn many_in_memory_jobs_finish_without_deadlock_at_one_and_four_workers() {
        for workers in [1usize, 4] {
            let (runtime, _sink) = make_runtime(workers);
            let signal = runtime
                .load_signal(LoadSignalRequest::new(vec![0.1; 1_024], 16_000.0))
                .expect("load samples");
            let mut analyses = Vec::new();
            let mut jobs = Vec::new();
            for _ in 0..120 {
                let (analysis, job) = runtime
                    .analyze_with_job(signal, &small_settings())
                    .expect("submit analysis");
                analyses.push(analysis);
                jobs.push(job);
            }
            runtime
                .wait_all(Some(Duration::from_secs(30)))
                .expect("all analyses");
            for (analysis, job) in analyses.iter().zip(&jobs) {
                assert_eq!(runtime.job_status(*job), JobStatus::Ready);
                let data = runtime.spectrogram_data(*analysis).expect("data");
                assert!(data.frames() > 0);
                assert!(data.data().iter().all(|value| value.is_finite()));
            }
            runtime
                .close(EntityId::Signal(signal))
                .expect("close signal");
            assert!(runtime.spectrogram_data(analyses[0]).unwrap_err().code() == "not_found");
            runtime.shutdown();
        }
    }

    #[test]
    fn dependencies_are_gated_and_order_line_data_has_the_expected_shape() {
        let samples = tones(16_000, 0.2, &[(440.0, 0.6), (1_000.0, 0.3)]);
        let path = write_wav("runtime_dependencies", 16_000, &samples);
        let (runtime, _sink) = make_runtime(1);

        // Occupy the only worker so every submitted job stays queued.
        let (blocker, release, _started) = blocking_job(&runtime, "blocker");

        let (signal, load_job) = runtime
            .load_file_with_job(LoadFileRequest::new(&path))
            .expect("submit load");
        let (analysis, analysis_job) = runtime
            .analyze_with_job(signal, &settings())
            .expect("submit analysis");
        let (order, order_job) = runtime
            .order_cut_with_job(analysis, &order_request())
            .expect("submit order");
        let (line, line_job) = runtime
            .line_cut_with_job(analysis, &line_request())
            .expect("submit line");

        assert_eq!(runtime.job_status(load_job), JobStatus::Pending);
        assert_eq!(runtime.job_status(analysis_job), JobStatus::Pending);
        assert_eq!(runtime.job_status(order_job), JobStatus::Pending);
        assert_eq!(runtime.job_status(line_job), JobStatus::Pending);
        assert_eq!(
            runtime.spectrogram_data(analysis).unwrap_err().code(),
            "busy"
        );
        assert_eq!(runtime.cut_data(order).unwrap_err().code(), "busy");
        assert_eq!(runtime.cut_data(line).unwrap_err().code(), "busy");

        release.send(()).expect("release blocker");
        runtime
            .wait(blocker, Some(Duration::from_secs(5)))
            .expect("blocker");
        runtime
            .wait_all(Some(Duration::from_secs(30)))
            .expect("all jobs");
        for job in [load_job, analysis_job, order_job, line_job] {
            assert_eq!(runtime.job_status(job), JobStatus::Ready);
        }

        let data = runtime.spectrogram_data(analysis).expect("analysis data");
        let order_snapshot = runtime.cut_data(order).expect("order data");
        let order_data = match &*order_snapshot {
            CutData::Order(data) => data,
            other => panic!("expected order data, got {other:?}"),
        };
        assert_eq!(order_data.frames, data.frames());
        assert_eq!(order_data.order_count, 2);
        assert_eq!(order_data.times.len(), data.frames());
        assert_eq!(order_data.order_count, 2);
        assert!(order_data.is_consistent());
        for (&value, &frequency) in order_data.values.iter().zip(order_data.frequencies.iter()) {
            if frequency.is_nan() {
                assert!(value.is_nan());
            } else {
                assert!(value.is_finite(), "finite level for {frequency} Hz");
            }
        }

        let order_points = runtime.cut_points(order).expect("order points");
        assert_eq!(order_points.len(), 3, "two orders plus one fixed frequency");
        assert!(order_points
            .iter()
            .any(|cut| matches!(cut.kind, CoreCutKind::Freq(freq) if (freq - 440.0).abs() < 1e-9)));
        assert!(order_points
            .iter()
            .any(|cut| matches!(cut.kind, CoreCutKind::Order(order) if order == 1.0)));

        let line_snapshot = runtime.cut_data(line).expect("line data");
        let line_data = match &*line_snapshot {
            CutData::Line(lines) => lines,
            other => panic!("expected line data, got {other:?}"),
        };
        assert_eq!(line_data.len(), 2);
        for track in line_data {
            assert_eq!(track.len(), 64);
            assert_eq!(track.times.len(), track.db.len());
            assert_eq!(track.frequencies.len(), track.db.len());
            assert_eq!(track.fractions.len(), track.db.len());
            assert!(track.db.iter().all(|value| value.is_finite()));
            assert!(track
                .fractions
                .iter()
                .all(|fraction| (0.0..=1.0).contains(fraction)));
            assert!(track
                .fractions
                .windows(2)
                .all(|pair| pair[0] <= pair[1] + 1e-12));
        }
        let line_points = runtime.cut_points(line).expect("line points");
        assert_eq!(line_points.len(), 2);
        assert!(matches!(line_points[0].kind, CoreCutKind::Line(1)));
        assert!(matches!(line_points[1].kind, CoreCutKind::Line(2)));

        runtime.close(EntityId::Analysis(analysis)).expect("close");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn rpm_points_override_changes_the_order_frequencies() {
        let samples = tones(16_000, 0.2, &[(440.0, 0.5)]);
        let path = write_wav("runtime_rpm_points", 16_000, &samples);
        let (runtime, _sink) = make_runtime(2);
        let signal = runtime
            .load_file_with_job(LoadFileRequest::new(&path))
            .expect("load")
            .0;
        let (analysis, _) = runtime
            .analyze_with_job(signal, &settings())
            .expect("analyze");

        let mut request = order_request();
        request.settings.orders = "1".to_string();
        request.settings.freqs.clear();
        request.rpm = Some(RpmProfileSpec::Points(vec![
            RpmPoint {
                time_s: 0.0,
                rpm: 3_000.0,
            },
            RpmPoint {
                time_s: 0.2,
                rpm: 6_000.0,
            },
        ]));
        let (cut, job) = runtime
            .order_cut_with_job(analysis, &request)
            .expect("order cut");
        runtime
            .wait(job, Some(Duration::from_secs(10)))
            .expect("cut");
        let cut_snapshot = runtime.cut_data(cut).expect("cut data");
        let data = match &*cut_snapshot {
            CutData::Order(data) => data,
            other => panic!("expected order data, got {other:?}"),
        };
        assert_eq!(data.order_count, 1);
        let first = data.frequencies[0] as f64;
        let last = data.frequencies[data.frames - 1] as f64;
        assert!(first.is_finite() && last.is_finite());
        // The first point is at the first frame *centre*, so the RPM (and the
        // order-1 frequency) is already a little above the profile start.
        let first_time = data.times[0];
        let last_time = data.times[data.frames - 1];
        let expected_first = (3_000.0 + 3_000.0 * first_time / 0.2) / 60.0;
        let expected_last = (3_000.0 + 3_000.0 * last_time / 0.2) / 60.0;
        assert!(
            (first - expected_first).abs() < 1.0,
            "first order frequency {first}"
        );
        assert!(
            (last - expected_last).abs() < 1.0,
            "last order frequency {last}"
        );
        assert!(last > first);

        let points = runtime.cut_points(cut).expect("core points");
        assert_eq!(points.len(), 1);
        assert!(points[0].points.first().unwrap().rpm <= points[0].points.last().unwrap().rpm);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn progress_events_precede_done_for_analysis() {
        let samples = tones(16_000, 0.15, &[(440.0, 0.5), (900.0, 0.4)]);
        let path = write_wav("runtime_progress", 16_000, &samples);
        let (runtime, sink) = make_runtime(2);
        let signal = runtime
            .load_file_with_job(LoadFileRequest::new(&path))
            .expect("load")
            .0;
        let (analysis, job) = runtime
            .analyze_with_job(signal, &settings())
            .expect("analyze");
        runtime
            .wait(job, Some(Duration::from_secs(10)))
            .expect("analysis");
        assert!(runtime.spectrogram_data(analysis).unwrap().frames() > 0);

        // `wait` is notified just before the `Done` event is emitted; poll
        // briefly so the assertion is not a scheduling race.
        let deadline = Instant::now() + Duration::from_secs(5);
        let events: Vec<_> = loop {
            let events: Vec<_> = sink
                .events()
                .into_iter()
                .filter(|event| event.job() == job)
                .collect();
            if events
                .iter()
                .any(|event| matches!(event, RuntimeEvent::Done { .. }))
                || Instant::now() >= deadline
            {
                break events;
            }
            thread::sleep(Duration::from_millis(2));
        };
        let progress = events
            .iter()
            .position(|event| matches!(event, RuntimeEvent::Progress { .. }))
            .expect("progress event");
        let done = events
            .iter()
            .position(|event| matches!(event, RuntimeEvent::Done { .. }))
            .expect("done event");
        assert!(progress < done, "progress must arrive before done");
        assert!(matches!(events.last(), Some(RuntimeEvent::Done { .. })));
        assert!(events.iter().any(|event| matches!(
            event,
            RuntimeEvent::Progress {
                frames_total: Some(_),
                eta_s: Some(_),
                ..
            }
        )));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn load_failures_fail_dependent_analyses_and_invalid_input_is_synchronous() {
        let dir = TempDir::new("failure");
        let missing = dir.path().join("missing.wav");
        let (runtime, _sink) = make_runtime(1);
        let (signal, load_job) = runtime
            .load_file_with_job(LoadFileRequest::new(&missing))
            .expect("submit bad path");
        let (analysis, analysis_job) = runtime
            .analyze_with_job(signal, &settings())
            .expect("dependent analysis");

        let error = runtime
            .wait(load_job, Some(Duration::from_secs(10)))
            .unwrap_err();
        assert_eq!(error.code(), "analysis_failed");
        let dependent_error = runtime
            .wait(analysis_job, Some(Duration::from_secs(10)))
            .unwrap_err();
        assert_eq!(dependent_error.code(), "analysis_failed");
        assert_eq!(
            runtime.spectrogram_data(analysis).unwrap_err().code(),
            "analysis_failed"
        );

        let signal = runtime
            .load_signal(LoadSignalRequest::new(vec![0.0; 64], 8_000.0))
            .expect("in-memory signal");
        let mut bad = settings();
        bad.frame = 4;
        assert_eq!(
            runtime.analyze_with_job(signal, &bad).unwrap_err().code(),
            "invalid_input"
        );
        assert_eq!(
            runtime
                .load_signal(LoadSignalRequest::new(vec![0.0; 8], 0.0))
                .unwrap_err()
                .code(),
            "invalid_input"
        );
    }

    #[test]
    fn running_and_queued_jobs_cancel_and_dependents_do_not_run() {
        // Running job: the runner waits for the cancellation flag.
        let (runtime, sink) = make_runtime(1);
        let (started_tx, started_rx) = mpsc::channel();
        let running = runtime
            .submit_test_job(
                &[],
                Box::new(move |ctx: &JobContext| {
                    started_tx.send(()).expect("start");
                    while !ctx.is_cancelled() {
                        thread::sleep(Duration::from_millis(1));
                    }
                    ctx.progress(1, Some(1));
                    Err(RuntimeError::cancelled("cancelled from test"))
                }),
            )
            .expect("submit running job");
        started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("running job started");
        assert_eq!(runtime.job_status(running), JobStatus::Running);
        runtime.cancel(running).expect("cancel running");
        assert_eq!(runtime.job_status(running), JobStatus::Closed);
        assert_eq!(
            runtime
                .wait(running, Some(Duration::from_secs(5)))
                .unwrap_err()
                .code(),
            "cancelled"
        );
        assert!(sink
            .events()
            .iter()
            .any(|event| matches!(event, RuntimeEvent::Cancelled { job } if *job == running)));
        runtime.cancel(running).expect("terminal cancel is a no-op");
        runtime.shutdown();

        // Queued job: cancel before the single worker can start it.
        let (runtime, _sink) = make_runtime(1);
        let (blocker, release, _started) = blocking_job(&runtime, "blocker");
        let ran = Arc::new(AtomicBool::new(false));
        let ran_flag = Arc::clone(&ran);
        let queued = runtime
            .submit_test_job(
                &[],
                Box::new(move |_ctx: &JobContext| {
                    ran_flag.store(true, Ordering::SeqCst);
                    Ok(JobOutput::None)
                }),
            )
            .expect("submit queued");
        assert_eq!(runtime.job_status(queued), JobStatus::Pending);
        runtime.cancel(queued).expect("cancel queued");
        assert_eq!(
            runtime
                .wait(queued, Some(Duration::from_secs(5)))
                .unwrap_err()
                .code(),
            "cancelled"
        );
        release.send(()).expect("release blocker");
        runtime
            .wait(blocker, Some(Duration::from_secs(5)))
            .expect("blocker");
        assert!(
            runtime
                .inner
                .active
                .wait_at_most(0, Some(Duration::from_secs(5))),
            "queued Rayon task should have unwound"
        );
        assert!(
            !ran.load(Ordering::SeqCst),
            "cancelled queued job must not run"
        );

        // Cancelling a parent cancels its dependent without running it.
        let (parent, _release, _started) = blocking_job(&runtime, "parent");
        let child_ran = Arc::new(AtomicBool::new(false));
        let child_flag = Arc::clone(&child_ran);
        let child = runtime
            .submit_test_job(
                &[parent],
                Box::new(move |_ctx: &JobContext| {
                    child_flag.store(true, Ordering::SeqCst);
                    Ok(JobOutput::None)
                }),
            )
            .expect("submit child");
        assert_eq!(runtime.job_status(child), JobStatus::Pending);
        runtime.cancel(parent).expect("cancel parent");
        assert_eq!(runtime.job_status(child), JobStatus::Closed);
        assert_eq!(
            runtime
                .wait(child, Some(Duration::from_secs(5)))
                .unwrap_err()
                .code(),
            "cancelled"
        );
        runtime
            .wait(parent, Some(Duration::from_secs(5)))
            .expect_err("parent was cancelled");
        assert!(
            runtime
                .inner
                .active
                .wait_at_most(0, Some(Duration::from_secs(5))),
            "cancelled dependent must unwind"
        );
        assert!(!child_ran.load(Ordering::SeqCst), "dependent must not run");
        runtime.shutdown();
    }

    #[test]
    fn close_cascades_idempotently_and_unknown_ids_are_typed() {
        let (runtime, _sink) = make_runtime(2);
        let signal = runtime
            .load_signal(LoadSignalRequest::new(vec![0.1; 2_048], 16_000.0))
            .expect("load");
        let (analysis, analysis_job) = runtime
            .analyze_with_job(signal, &small_settings())
            .expect("analyze");
        runtime
            .wait(analysis_job, Some(Duration::from_secs(10)))
            .expect("analysis");
        let (order, order_job) = runtime
            .order_cut_with_job(analysis, &order_request())
            .expect("order");
        let (line, line_job) = runtime
            .line_cut_with_job(analysis, &line_request())
            .expect("line");
        runtime
            .wait_all(Some(Duration::from_secs(10)))
            .expect("cuts");

        runtime
            .close(EntityId::Analysis(analysis))
            .expect("close analysis");
        assert_eq!(runtime.job_for(EntityId::Analysis(analysis)), None);
        assert_eq!(runtime.job_for(EntityId::Cut(order)), None);
        assert_eq!(runtime.job_for(EntityId::Cut(line)), None);
        assert_eq!(
            runtime.spectrogram_data(analysis).unwrap_err().code(),
            "not_found"
        );
        assert_eq!(runtime.cut_data(order).unwrap_err().code(), "not_found");
        assert_eq!(runtime.cut_data(line).unwrap_err().code(), "not_found");
        assert_eq!(runtime.job_status(order_job), JobStatus::Closed);
        assert_eq!(runtime.job_status(line_job), JobStatus::Closed);
        runtime
            .close(EntityId::Analysis(analysis))
            .expect("idempotent close");
        runtime
            .close(EntityId::Cut(order))
            .expect("unknown cut close");

        runtime
            .close(EntityId::Signal(signal))
            .expect("close signal");
        runtime
            .close(EntityId::Signal(signal))
            .expect("idempotent signal close");
        assert_eq!(runtime.signal_info(signal).unwrap_err().code(), "not_found");

        assert_eq!(
            runtime
                .spectrogram_data(AnalysisId(999))
                .unwrap_err()
                .code(),
            "not_found"
        );
        assert_eq!(
            runtime.cut_data(CutId(999)).unwrap_err().code(),
            "not_found"
        );
        assert_eq!(runtime.job_status(JobId(999)), JobStatus::Closed);
        assert_eq!(
            runtime
                .wait(JobId(999), Some(Duration::from_millis(1)))
                .unwrap_err()
                .code(),
            "cancelled"
        );
        assert_eq!(runtime.cancel(JobId(999)).unwrap_err().code(), "not_found");
        assert_eq!(
            runtime
                .order_cut_with_job(AnalysisId(999), &order_request())
                .unwrap_err()
                .code(),
            "not_found"
        );
        assert_eq!(
            runtime
                .analyze_with_job(SignalId(999), &settings())
                .unwrap_err()
                .code(),
            "not_found"
        );
    }

    #[test]
    fn csv_and_png_exports_have_the_stable_shapes() {
        let dir = TempDir::new("exports");
        let samples = tones(16_000, 0.15, &[(440.0, 0.5)]);
        let path = write_wav("runtime_exports", 16_000, &samples);
        let (runtime, _sink) = make_runtime(2);
        let signal = runtime
            .load_file_with_job(LoadFileRequest::new(&path))
            .expect("load")
            .0;
        let (analysis, _) = runtime
            .analyze_with_job(signal, &settings())
            .expect("analyze");
        let (cut, _) = runtime
            .order_cut_with_job(analysis, &order_request())
            .expect("order");
        runtime
            .wait_all(Some(Duration::from_secs(15)))
            .expect("compute");

        let csv = dir.path().join("cuts.csv");
        runtime.export_csv(cut, &csv).expect("export csv");
        let text = std::fs::read_to_string(&csv).expect("read csv");
        assert!(text.starts_with("kind,value,time_s,rpm,freq_hz,db\n"));
        assert!(text.lines().count() > 1);

        let png = dir.path().join("spectrogram.png");
        let options = RenderOptions {
            width: 64,
            height: 32,
            ..Default::default()
        };
        runtime
            .save_image(analysis, &png, &options)
            .expect("save png");
        let bytes = std::fs::read(&png).expect("read png");
        assert!(!bytes.is_empty());
        assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));

        assert_eq!(
            runtime
                .export_csv(CutId(999), &dir.path().join("none.csv"))
                .unwrap_err()
                .code(),
            "not_found"
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn wait_all_and_shutdown_return_without_hanging() {
        let (runtime, _sink) = make_runtime(2);
        let signal = runtime
            .load_signal(LoadSignalRequest::new(vec![0.2; 4_096], 16_000.0))
            .expect("load");
        let mut jobs = Vec::new();
        for _ in 0..12 {
            let (_, job) = runtime
                .analyze_with_job(signal, &small_settings())
                .expect("analyze");
            jobs.push(job);
        }
        runtime
            .wait_all(Some(Duration::from_secs(20)))
            .expect("wait all");
        assert!(jobs
            .iter()
            .all(|job| runtime.job_status(*job) == JobStatus::Ready));

        runtime.shutdown();
        runtime.shutdown();
        runtime
            .wait_all(Some(Duration::from_secs(1)))
            .expect("after shutdown");
        assert_eq!(
            runtime
                .analyze_with_job(signal, &settings())
                .unwrap_err()
                .code(),
            "closed"
        );
        assert_eq!(
            runtime
                .load_signal(LoadSignalRequest::new(vec![0.0; 8], 8_000.0))
                .unwrap_err()
                .code(),
            "closed"
        );
    }

    #[test]
    fn pending_jobs_are_cancelled_by_shutdown() {
        let (runtime, sink) = make_runtime(1);
        let (blocker, release, _started) = blocking_job(&runtime, "blocker");
        let (_, analysis_job) = runtime
            .analyze_with_job(
                runtime
                    .load_signal(LoadSignalRequest::new(vec![0.1; 1_024], 16_000.0))
                    .expect("signal"),
                &small_settings(),
            )
            .expect("analysis");
        runtime.shutdown();
        assert_eq!(runtime.job_status(blocker), JobStatus::Closed);
        assert_eq!(runtime.job_status(analysis_job), JobStatus::Closed);
        assert!(sink
            .events()
            .iter()
            .any(|event| matches!(event, RuntimeEvent::Cancelled { job } if *job == analysis_job)));
        // The blocker observes the flag and exits even though its release
        // channel was never used.
        drop(release);
    }

    #[test]
    fn failed_dependents_do_not_run() {
        let (runtime, _sink) = make_runtime(1);
        let parent = runtime
            .submit_test_job(
                &[],
                Box::new(|_ctx: &JobContext| {
                    Err(RuntimeError::analysis_failed("synthetic failure"))
                }),
            )
            .expect("parent");
        let ran = Arc::new(AtomicBool::new(false));
        let ran_flag = Arc::clone(&ran);
        let child = runtime
            .submit_test_job(
                &[parent],
                Box::new(move |_ctx: &JobContext| {
                    ran_flag.store(true, Ordering::SeqCst);
                    Ok(JobOutput::None)
                }),
            )
            .expect("child");
        let error = runtime
            .wait(parent, Some(Duration::from_secs(5)))
            .unwrap_err();
        assert_eq!(error.code(), "analysis_failed");
        let error = runtime
            .wait(child, Some(Duration::from_secs(5)))
            .unwrap_err();
        assert_eq!(error.code(), "analysis_failed");
        assert_eq!(runtime.job_status(child), JobStatus::Failed);
        assert!(!ran.load(Ordering::SeqCst), "failed dependent must not run");
    }

    #[test]
    fn shutdown_from_a_worker_does_not_deadlock() {
        let runtime = Arc::new(CoreRuntime::new(1, Arc::new(CaptureSink::default())));
        let clone = Arc::clone(&runtime);
        let job = runtime
            .submit_test_job(
                &[],
                Box::new(move |_ctx: &JobContext| {
                    clone.shutdown();
                    Ok(JobOutput::None)
                }),
            )
            .expect("submit");
        let error = runtime.wait(job, Some(Duration::from_secs(5))).unwrap_err();
        assert_eq!(error.code(), "cancelled");
        runtime.shutdown();
        runtime
            .wait_all(Some(Duration::from_secs(5)))
            .expect("worker shutdown settled");
    }

    #[test]
    fn worker_thread_helper_tracks_job_execution() {
        assert!(!is_worker_thread());
        let (runtime, _sink) = make_runtime(1);
        let (marker_tx, marker_rx) = mpsc::channel();
        let job = runtime
            .submit_test_job(
                &[],
                Box::new(move |_ctx: &JobContext| {
                    marker_tx
                        .send(is_worker_thread())
                        .expect("send worker marker");
                    Ok(JobOutput::None)
                }),
            )
            .expect("submit");
        assert!(
            marker_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("worker marker"),
            "jobs must run with the worker marker set"
        );
        runtime
            .wait(job, Some(Duration::from_secs(5)))
            .expect("job completes");
        assert!(!is_worker_thread());
    }

    #[test]
    fn concurrent_worker_shutdowns_do_not_wait_for_each_other() {
        let runtime = Arc::new(CoreRuntime::new(2, Arc::new(CaptureSink::default())));
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let (done_tx, done_rx) = mpsc::channel();
        let mut jobs = Vec::new();
        for label in ["a", "b"] {
            let shutdown_runtime = Arc::clone(&runtime);
            let barrier = Arc::clone(&barrier);
            let done_tx = done_tx.clone();
            jobs.push(
                runtime
                    .submit_test_job(
                        &[],
                        Box::new(move |_ctx: &JobContext| {
                            barrier.wait();
                            shutdown_runtime.shutdown();
                            done_tx.send(label).expect("signal shutdown done");
                            Ok(JobOutput::None)
                        }),
                    )
                    .expect("submit"),
            );
        }
        drop(done_tx);
        for _ in 0..2 {
            done_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("both worker shutdowns must return");
        }
        for job in jobs {
            assert_eq!(runtime.job_status(job), JobStatus::Closed);
        }
    }

    #[test]
    fn worker_shutdown_returns_without_waiting_for_a_spawned_queued_task() {
        // Regression: with one worker, job A calls shutdown while B is already
        // spawned. Waiting for all active tasks includes B, but B cannot start
        // until A returns. Worker shutdown must wait only for running peers.
        let runtime = Arc::new(CoreRuntime::new(1, Arc::new(CaptureSink::default())));
        let b_ran = Arc::new(AtomicBool::new(false));

        let (a_started_tx, a_started_rx) = mpsc::channel();
        let (go_tx, go_rx) = mpsc::channel();
        let (a_done_tx, a_done_rx) = mpsc::channel();
        let shutdown_runtime = Arc::clone(&runtime);
        let job_a = runtime
            .submit_test_job(
                &[],
                Box::new(move |_ctx: &JobContext| {
                    a_started_tx.send(()).expect("signal A start");
                    go_rx
                        .recv_timeout(Duration::from_secs(5))
                        .expect("wait for B submission");
                    shutdown_runtime.shutdown();
                    a_done_tx.send(()).expect("signal A done");
                    Ok(JobOutput::None)
                }),
            )
            .expect("submit A");
        a_started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("A must be running");

        let b_flag = Arc::clone(&b_ran);
        let job_b = runtime
            .submit_test_job(
                &[],
                Box::new(move |_ctx: &JobContext| {
                    b_flag.store(true, Ordering::SeqCst);
                    Ok(JobOutput::None)
                }),
            )
            .expect("submit B");
        assert_eq!(runtime.job_status(job_b), JobStatus::Pending);

        go_tx.send(()).expect("release A");
        a_done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("worker shutdown must not wait for queued B");

        assert!(!b_ran.load(Ordering::SeqCst), "queued B must not run");
        assert_eq!(runtime.job_status(job_a), JobStatus::Closed);
        assert_eq!(runtime.job_status(job_b), JobStatus::Closed);
        assert!(
            runtime
                .inner
                .active
                .wait_at_most(0, Some(Duration::from_secs(5))),
            "A and the cancelled B task must both unwind"
        );
    }

    #[test]
    fn worker_wait_all_reports_busy_instead_of_waiting_for_queued_work() {
        // Regression: with one worker, A calling wait_all while B is queued
        // must return Busy immediately instead of waiting for B to run on A's
        // own worker.
        let runtime = Arc::new(CoreRuntime::new(1, Arc::new(CaptureSink::default())));
        let (a_started_tx, a_started_rx) = mpsc::channel();
        let (go_tx, go_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let wait_runtime = Arc::clone(&runtime);
        let job_a = runtime
            .submit_test_job(
                &[],
                Box::new(move |_ctx: &JobContext| {
                    a_started_tx.send(()).expect("signal A start");
                    go_rx
                        .recv_timeout(Duration::from_secs(5))
                        .expect("wait for B submission");
                    result_tx
                        .send(wait_runtime.wait_all(None))
                        .expect("send wait_all result");
                    Ok(JobOutput::None)
                }),
            )
            .expect("submit A");
        a_started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("A must be running");

        let job_b = runtime
            .submit_test_job(&[], Box::new(|_ctx: &JobContext| Ok(JobOutput::None)))
            .expect("submit B");
        assert_eq!(runtime.job_status(job_b), JobStatus::Pending);

        go_tx.send(()).expect("release A");
        let result = result_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("worker wait_all must not hang");
        let error = result.expect_err("queued work must be reported as busy");
        assert_eq!(error.code(), "busy");

        // A still finishes normally and B runs after the worker is free.
        runtime
            .wait(job_a, Some(Duration::from_secs(5)))
            .expect("A finishes");
        runtime
            .wait(job_b, Some(Duration::from_secs(5)))
            .expect("B eventually runs");
        runtime
            .wait_all(Some(Duration::from_secs(5)))
            .expect("non-worker wait_all settles");
    }

    #[test]
    fn worker_wait_all_is_ok_when_the_caller_is_the_only_active_job() {
        let runtime = Arc::new(CoreRuntime::new(1, Arc::new(CaptureSink::default())));
        let (result_tx, result_rx) = mpsc::channel();
        let wait_runtime = Arc::clone(&runtime);
        let job = runtime
            .submit_test_job(
                &[],
                Box::new(move |_ctx: &JobContext| {
                    result_tx
                        .send(wait_runtime.wait_all(Some(Duration::from_secs(5))))
                        .expect("send wait_all result");
                    Ok(JobOutput::None)
                }),
            )
            .expect("submit");
        result_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("worker wait_all must not hang")
            .expect("the caller is the only active job");
        runtime
            .wait(job, Some(Duration::from_secs(5)))
            .expect("job completes");
    }

    #[test]
    fn wait_timeout_is_busy_and_does_not_cancel() {
        let (runtime, _sink) = make_runtime(1);
        let (blocker, release, _started) = blocking_job(&runtime, "blocker");
        let error = runtime
            .wait(blocker, Some(Duration::from_millis(10)))
            .unwrap_err();
        assert_eq!(error.code(), "busy");
        assert_eq!(runtime.job_status(blocker), JobStatus::Running);
        release.send(()).expect("release");
        runtime
            .wait(blocker, Some(Duration::from_secs(5)))
            .expect("blocker done");
    }

    #[test]
    fn test_job_counter_is_shared_across_worker_counts() {
        // Guard against silently reusing IDs when several runtimes exist.
        let (first, _) = make_runtime(1);
        let (second, _) = make_runtime(4);
        let a = first
            .submit_test_job(&[], Box::new(|_ctx: &JobContext| Ok(JobOutput::None)))
            .expect("a");
        let b = second
            .submit_test_job(&[], Box::new(|_ctx: &JobContext| Ok(JobOutput::None)))
            .expect("b");
        assert_eq!(a, JobId(1));
        assert_eq!(b, JobId(1));
    }
}
