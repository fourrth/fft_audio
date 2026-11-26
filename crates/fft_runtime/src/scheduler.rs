//! Scheduler primitives: job graph, ready queue, cancellation and progress.
//!
//! Jobs only become runnable when every dependency is `Ready`. A worker never
//! blocks on a dependent: completing a job enqueues any children whose last
//! dependency just became ready. Failures/cancellations cascade through the
//! dependents without running them.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use fft_core::orders::Cut;

use crate::data::{CutData, SpectrogramData};
use crate::error::RuntimeError;
use crate::events::{JobStatus, RuntimeEvent, RuntimeEventSink};
use crate::ids::{AnalysisId, CutId, JobId, SignalId};
use crate::state::SignalData;

/// Result produced by a scheduled job. Applied to the owned stores only after
/// the job is confirmed successful (a closed/cancelled job discards it).
pub(crate) enum JobOutput {
    /// No entity payload (used by internal tests).
    #[allow(dead_code)] // constructed only from `#[cfg(test)]` helpers
    None,
    Signal(SignalId, SignalData),
    Analysis(AnalysisId, Box<SpectrogramData>),
    Cut(CutId, Arc<CutData>, Vec<Cut>),
}

/// Work owned by the scheduler.
pub(crate) type JobRunner = Box<dyn FnOnce(&JobContext) -> Result<JobOutput, RuntimeError> + Send>;

/// Per-run context handed to a [`JobRunner`]. Progress checks the cancellation
/// flag, so cooperative jobs stop at the next frame/buffer boundary.
pub(crate) struct JobContext {
    job: JobId,
    cancel: Arc<AtomicBool>,
    started: Instant,
    events: Arc<dyn RuntimeEventSink>,
}

impl JobContext {
    pub(crate) fn new(
        job: JobId,
        cancel: Arc<AtomicBool>,
        started: Instant,
        events: Arc<dyn RuntimeEventSink>,
    ) -> Self {
        Self {
            job,
            cancel,
            started,
            events,
        }
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// Emit a progress event and return `false` if the job was cancelled.
    pub(crate) fn progress(&self, frames_done: usize, frames_total: Option<usize>) -> bool {
        if self.is_cancelled() {
            return false;
        }
        let eta_s = match frames_total {
            Some(total) if total > 0 && frames_done > 0 => {
                let elapsed = self.started.elapsed().as_secs_f64();
                let fraction = (frames_done.min(total) as f64) / total as f64;
                Some((elapsed / fraction - elapsed).max(0.0))
            }
            _ => None,
        };
        self.events.emit(RuntimeEvent::Progress {
            job: self.job,
            frames_done,
            frames_total,
            eta_s,
        });
        !self.is_cancelled()
    }
}

/// One job in the dependency graph.
pub(crate) struct JobRecord {
    pub(crate) status: JobStatus,
    pub(crate) error: Option<RuntimeError>,
    pub(crate) cancel: Arc<AtomicBool>,
    pub(crate) runner: Option<JobRunner>,
    /// Dependencies that were still `Pending`/`Running` at submission time.
    pub(crate) deps_remaining: usize,
    pub(crate) dependents: Vec<JobId>,
    /// True once a Rayon task has been spawned for this job.
    pub(crate) scheduled: bool,
    pub(crate) started: Option<Instant>,
}

/// Job graph state, owned under the runtime's single state mutex.
pub(crate) struct SchedulerState {
    pub(crate) jobs: HashMap<JobId, JobRecord>,
    /// Jobs whose dependencies were satisfied; workers pick from here.
    pub(crate) ready: VecDeque<JobId>,
    pub(crate) next_job: u64,
    pub(crate) shutdown: bool,
}

impl Default for SchedulerState {
    fn default() -> Self {
        Self {
            jobs: HashMap::new(),
            ready: VecDeque::new(),
            next_job: 1,
            shutdown: false,
        }
    }
}

impl SchedulerState {
    pub(crate) fn allocate_job(&mut self) -> JobId {
        let id = JobId(self.next_job.max(1));
        self.next_job = id.0.saturating_add(1);
        id
    }

    /// Mark a dependency as complete for every child, returning the children
    /// that became runnable. A child with several dependencies is only
    /// returned by the completion that decrements its counter to zero.
    pub(crate) fn release_dependents(&mut self, parents: &[JobId]) -> Vec<JobId> {
        let mut ready = Vec::new();
        for parent in parents {
            let dependents = match self.jobs.get_mut(parent) {
                Some(record) => std::mem::take(&mut record.dependents),
                None => Vec::new(),
            };
            for child in dependents {
                let Some(record) = self.jobs.get_mut(&child) else {
                    continue;
                };
                if record.status != JobStatus::Pending || record.deps_remaining == 0 {
                    continue;
                }
                record.deps_remaining -= 1;
                if record.deps_remaining == 0 {
                    ready.push(child);
                }
            }
        }
        ready
    }

    /// Queue jobs that just became runnable. Returns the number queued.
    pub(crate) fn enqueue_ready(&mut self, jobs: &[JobId]) -> usize {
        let mut queued = 0;
        for &job in jobs {
            let Some(record) = self.jobs.get_mut(&job) else {
                continue;
            };
            if record.status != JobStatus::Pending || record.scheduled {
                continue;
            }
            self.ready.push_back(job);
            queued += 1;
        }
        queued
    }
}

/// Counts spawned Rayon tasks and, separately, tasks that have actually begun
/// executing. A worker-side shutdown may wait for already-running peers, but it
/// must never wait for a task that is merely spawned/queued: with one worker
/// that task can only start after the caller returns.
///
/// Tasks drop their strong `Arc<Inner>` *before* decrementing `spawned`, so a
/// shutdown wait can guarantee no worker task still owns the runtime when the
/// pool is dropped.
#[derive(Default)]
pub(crate) struct ActiveTasks {
    counts: Mutex<ActiveCounts>,
    wake: Condvar,
    /// Only one worker-side shutdown waits for peer tasks. A second worker
    /// calling shutdown returns immediately, so two workers cancelling at the
    /// same time cannot wait for each other.
    shutdown_waiter: AtomicBool,
}

#[derive(Default)]
struct ActiveCounts {
    spawned: usize,
    running: usize,
}

impl ActiveTasks {
    pub(crate) fn begin(&self) {
        let mut counts = self.counts.lock().unwrap_or_else(|e| e.into_inner());
        counts.spawned += 1;
    }

    pub(crate) fn end(&self) {
        {
            let mut counts = self.counts.lock().unwrap_or_else(|e| e.into_inner());
            counts.spawned = counts.spawned.saturating_sub(1);
        }
        self.wake.notify_all();
    }

    fn begin_running(&self) {
        let mut counts = self.counts.lock().unwrap_or_else(|e| e.into_inner());
        counts.running += 1;
    }

    fn end_running(&self) {
        {
            let mut counts = self.counts.lock().unwrap_or_else(|e| e.into_inner());
            counts.running = counts.running.saturating_sub(1);
        }
        self.wake.notify_all();
    }

    /// Claim the right to wait for peer tasks during a worker-side shutdown.
    /// `None` means another worker already claimed it and this caller must
    /// return without waiting.
    pub(crate) fn begin_worker_shutdown_wait(&self) -> Option<WorkerShutdownWaitGuard<'_>> {
        self.shutdown_waiter
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| WorkerShutdownWaitGuard(self))
    }

    /// Wait until at most `max` spawned tasks remain. A non-worker shutdown
    /// uses 0, so queued tasks drain too.
    pub(crate) fn wait_at_most(&self, max: usize, timeout: Option<Duration>) -> bool {
        self.wait_until(|counts| counts.spawned <= max, timeout)
    }

    /// Wait until at most `max` tasks are executing. A worker-side shutdown
    /// uses 1 (the caller itself) and therefore ignores spawned-but-queued
    /// tasks that cannot run until the caller's worker is free.
    pub(crate) fn wait_running_at_most(&self, max: usize, timeout: Option<Duration>) -> bool {
        self.wait_until(|counts| counts.running <= max, timeout)
    }

    fn wait_until(&self, done: impl Fn(&ActiveCounts) -> bool, timeout: Option<Duration>) -> bool {
        let deadline = timeout.map(|timeout| Instant::now() + timeout);
        let mut counts = self.counts.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if done(&counts) {
                return true;
            }
            counts = match deadline {
                Some(deadline) => {
                    let now = Instant::now();
                    if now >= deadline {
                        return false;
                    }
                    self.wake
                        .wait_timeout(counts, deadline - now)
                        .unwrap_or_else(|e| e.into_inner())
                        .0
                }
                None => self.wake.wait(counts).unwrap_or_else(|e| e.into_inner()),
            };
        }
    }
}

pub(crate) struct WorkerShutdownWaitGuard<'a>(&'a ActiveTasks);

impl Drop for WorkerShutdownWaitGuard<'_> {
    fn drop(&mut self) {
        self.0.shutdown_waiter.store(false, Ordering::Release);
    }
}

/// Counts a task while it is actually executing. Worker-side shutdown waits
/// for these (minus the caller), never for spawned-but-queued tasks.
pub(crate) struct RunningTaskGuard<'a>(&'a ActiveTasks);

impl<'a> RunningTaskGuard<'a> {
    pub(crate) fn enter(active: &'a ActiveTasks) -> Self {
        active.begin_running();
        Self(active)
    }
}

impl Drop for RunningTaskGuard<'_> {
    fn drop(&mut self) {
        self.0.end_running();
    }
}

thread_local! {
    static CURRENT_JOB: std::cell::Cell<Option<JobId>> = const { std::cell::Cell::new(None) };
    static WORKER_THREAD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub(crate) fn current_job() -> Option<JobId> {
    CURRENT_JOB.with(|current| current.get())
}

/// True when the calling thread is executing a runtime job.
pub(crate) fn is_worker_thread() -> bool {
    WORKER_THREAD.with(|worker| worker.get())
}

pub(crate) struct CurrentJobGuard(Option<JobId>);

impl CurrentJobGuard {
    pub(crate) fn enter(job: JobId) -> Self {
        let previous = CURRENT_JOB.with(|current| current.replace(Some(job)));
        Self(previous)
    }
}

impl Drop for CurrentJobGuard {
    fn drop(&mut self) {
        CURRENT_JOB.with(|current| current.set(self.0));
    }
}

/// Marks the thread while it executes a scheduled job. Used to decide whether
/// `shutdown`/`wait_all` may block: workers must not wait for work that only
/// they themselves can run.
pub(crate) struct WorkerThreadGuard(bool);

impl WorkerThreadGuard {
    pub(crate) fn enter() -> Self {
        let previous = WORKER_THREAD.with(|worker| worker.replace(true));
        Self(previous)
    }
}

impl Drop for WorkerThreadGuard {
    fn drop(&mut self) {
        WORKER_THREAD.with(|worker| worker.set(self.0));
    }
}
