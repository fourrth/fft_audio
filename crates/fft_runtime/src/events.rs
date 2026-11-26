//! Job status and runtime progress events.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::ids::JobId;

/// Lifecycle of a scheduled job.
///
/// A cancelled job is reported as [`JobStatus::Closed`]; the cancellation
/// reason is available through `wait` (`RuntimeError::Cancelled`) or a
/// [`RuntimeEvent::Cancelled`] event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Pending,
    Running,
    Ready,
    Failed,
    Closed,
}

impl JobStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            JobStatus::Pending => "pending",
            JobStatus::Running => "running",
            JobStatus::Ready => "ready",
            JobStatus::Failed => "failed",
            JobStatus::Closed => "closed",
        }
    }

    /// True while the job still occupies a scheduler slot.
    pub const fn is_active(self) -> bool {
        matches!(self, JobStatus::Pending | JobStatus::Running)
    }

    /// True when `wait` can return immediately.
    pub const fn is_terminal(self) -> bool {
        !self.is_active()
    }
}

impl std::fmt::Display for JobStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Progress/lifecycle notification emitted by the runtime.
///
/// Adapters translate these into WS/Tauri events. The runtime never knows
/// about transport names or display ids.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum RuntimeEvent {
    Progress {
        job: JobId,
        frames_done: usize,
        frames_total: Option<usize>,
        eta_s: Option<f64>,
    },
    Done {
        job: JobId,
    },
    Failed {
        job: JobId,
        message: String,
    },
    Cancelled {
        job: JobId,
    },
}

impl RuntimeEvent {
    pub const fn job(&self) -> JobId {
        match self {
            RuntimeEvent::Progress { job, .. }
            | RuntimeEvent::Done { job }
            | RuntimeEvent::Failed { job, .. }
            | RuntimeEvent::Cancelled { job } => *job,
        }
    }
}

/// Per-job progress throttle shared by the transport adapters.
///
/// The runtime reports progress for every analysis frame. Transports do not
/// need every frame, and an unthrottled stream can crowd a bounded ring buffer
/// so the terminal `Done`/`Failed`/`Cancelled` event is lost. [`Self::should_forward`]
/// forwards the first progress for a job and then at most one per `interval`;
/// terminal events always pass and reset that job's throttle state.
#[derive(Debug)]
pub struct ProgressCoalescer {
    interval: Duration,
    next_forward: HashMap<JobId, Instant>,
}

impl Default for ProgressCoalescer {
    fn default() -> Self {
        Self::new(Duration::from_millis(75))
    }
}

impl ProgressCoalescer {
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            next_forward: HashMap::new(),
        }
    }

    /// Decide whether `event` should be delivered now. Progress outside the
    /// interval is dropped (it is advisory); every terminal event is delivered.
    pub fn should_forward(&mut self, event: &RuntimeEvent, now: Instant) -> bool {
        match event {
            RuntimeEvent::Progress { job, .. } => {
                if self.next_forward.get(job).is_some_and(|next| now < *next) {
                    return false;
                }
                self.next_forward.insert(*job, now + self.interval);
                true
            }
            RuntimeEvent::Done { .. }
            | RuntimeEvent::Failed { .. }
            | RuntimeEvent::Cancelled { .. } => {
                self.next_forward.remove(&event.job());
                true
            }
        }
    }
}

/// Receiver for [`RuntimeEvent`]s.
pub trait RuntimeEventSink: Send + Sync {
    fn emit(&self, event: RuntimeEvent);
}

impl<F> RuntimeEventSink for F
where
    F: Fn(RuntimeEvent) + Send + Sync,
{
    fn emit(&self, event: RuntimeEvent) {
        self(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_status_names_and_activity_are_stable() {
        assert_eq!(JobStatus::Pending.as_str(), "pending");
        assert_eq!(JobStatus::Closed.to_string(), "closed");
        assert!(JobStatus::Pending.is_active());
        assert!(JobStatus::Running.is_active());
        assert!(!JobStatus::Ready.is_active());
        assert!(JobStatus::Failed.is_terminal());
        assert_eq!(
            serde_json::to_string(&JobStatus::Ready).unwrap(),
            "\"ready\""
        );
    }

    #[test]
    fn progress_coalescer_forwards_first_and_terminal_but_drops_rapid_frames() {
        let mut coalescer = ProgressCoalescer::new(Duration::from_millis(50));
        let start = Instant::now();
        let progress = |job, frame| RuntimeEvent::Progress {
            job: JobId(job),
            frames_done: frame,
            frames_total: Some(10),
            eta_s: None,
        };

        assert!(coalescer.should_forward(&progress(1, 1), start));
        assert!(!coalescer.should_forward(&progress(1, 2), start + Duration::from_millis(10)));
        assert!(!coalescer.should_forward(&progress(1, 3), start + Duration::from_millis(49)));
        assert!(coalescer.should_forward(&progress(1, 4), start + Duration::from_millis(50)));
        // Other jobs have independent windows.
        assert!(coalescer.should_forward(&progress(2, 1), start + Duration::from_millis(10)));

        // Terminal events are never throttled, and they reset the job so a
        // later progress event (e.g. a new job with the same id in a test) starts
        // a fresh window.
        let done = RuntimeEvent::Done { job: JobId(1) };
        assert!(coalescer.should_forward(&done, start + Duration::from_millis(55)));
        assert!(coalescer.should_forward(&progress(1, 1), start + Duration::from_millis(56)));
    }

    #[test]
    fn runtime_events_serialize_with_a_stable_type_tag() {
        let event = RuntimeEvent::Progress {
            job: JobId(3),
            frames_done: 1,
            frames_total: Some(2),
            eta_s: None,
        };
        assert_eq!(
            serde_json::to_string(&event).unwrap(),
            r#"{"type":"progress","job":3,"framesDone":1,"framesTotal":2,"etaS":null}"#
        );
        assert_eq!(event.job(), JobId(3));
    }
}
