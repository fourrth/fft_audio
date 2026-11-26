//! Test implementations for the scripting contract.
//!
//! `MockScript` proves the wiring an embedded language engine will use: it
//! receives an [`AppLease`], reaches the leased app's runtime to load and
//! analyse a signal, and reports ordered output/exit events. It is synchronous
//! and has no Python/Lua dependency.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fft_app::{AppLease, LeaseId};
use fft_core::settings::AnalysisSettings;
use fft_runtime::{AnalysisId, EntityId, LoadSignalRequest, ScriptRunId, SignalId};

#[cfg(test)]
use fft_runtime::MockRuntime;

use crate::error::ScriptError;
use crate::events::{ScriptEvent, ScriptEventSink, ScriptStream};
use crate::{FftScript, ScriptSource};

/// Deterministic recording of one mock script execution.
#[derive(Clone, Debug, PartialEq)]
pub struct MockScriptRecord {
    pub run: ScriptRunId,
    pub lease_id: LeaseId,
    pub source: ScriptSource,
    pub signal: Option<SignalId>,
    pub analysis: Option<AnalysisId>,
    pub success: bool,
    pub error: Option<ScriptError>,
}

/// Capturing [`ScriptEventSink`] for tests.
#[derive(Default)]
pub struct CapturingScriptEvents {
    events: Mutex<Vec<ScriptEvent>>,
}

impl CapturingScriptEvents {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn events(&self) -> Vec<ScriptEvent> {
        self.events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn clear(&self) {
        self.events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }

    pub fn len(&self) -> usize {
        self.events.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl ScriptEventSink for CapturingScriptEvents {
    fn output(&self, run: ScriptRunId, stream: ScriptStream, line: String) {
        self.events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(ScriptEvent::Output { run, stream, line });
    }

    fn exited(&self, run: ScriptRunId, success: bool, code: Option<i32>) {
        self.events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(ScriptEvent::Exited { run, success, code });
    }
}

#[derive(Default)]
struct State {
    records: Vec<MockScriptRecord>,
    active: HashMap<ScriptRunId, Arc<dyn ScriptEventSink>>,
    cancelled: Vec<ScriptRunId>,
}

/// Language-neutral mock script engine used by contract and adapter tests.
pub struct MockScript {
    next_run: AtomicU64,
    state: Mutex<State>,
}

impl Default for MockScript {
    fn default() -> Self {
        Self::new()
    }
}

impl MockScript {
    pub fn new() -> Self {
        Self {
            next_run: AtomicU64::new(1),
            state: Mutex::new(State::default()),
        }
    }

    pub fn records(&self) -> Vec<MockScriptRecord> {
        self.lock().records.clone()
    }

    pub fn last_record(&self) -> Option<MockScriptRecord> {
        self.lock().records.last().cloned()
    }

    pub fn cancelled_runs(&self) -> Vec<ScriptRunId> {
        self.lock().cancelled.clone()
    }

    pub fn active_runs(&self) -> Vec<ScriptRunId> {
        let mut runs: Vec<_> = self.lock().active.keys().copied().collect();
        runs.sort();
        runs
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn finish(&self, record: MockScriptRecord) -> std::result::Result<ScriptRunId, ScriptError> {
        let run = record.run;
        self.lock().records.push(record);
        Ok(run)
    }
}

impl FftScript for MockScript {
    fn run(
        &self,
        lease: Arc<dyn AppLease>,
        source: ScriptSource,
        events: Arc<dyn ScriptEventSink>,
    ) -> std::result::Result<ScriptRunId, ScriptError> {
        let run = ScriptRunId(self.next_run.fetch_add(1, Ordering::SeqCst));
        let lease_id = lease.lease_id();
        self.lock().active.insert(run, Arc::clone(&events));

        let label = format!("mock: starting {}", source.name());
        events.output(run, ScriptStream::Log, label);

        let runtime = lease.app().runtime();
        let request =
            LoadSignalRequest::new(vec![0.0; 128], 8_000.0).with_name(format!("mock-script-{run}"));
        let signal = match runtime.load_signal(request) {
            Ok(signal) => signal,
            Err(error) => {
                events.output(run, ScriptStream::Stderr, format!("mock: {error}"));
                events.exited(run, false, None);
                self.lock().active.remove(&run);
                return self.finish(MockScriptRecord {
                    run,
                    lease_id,
                    source,
                    signal: None,
                    analysis: None,
                    success: false,
                    error: Some(error.into()),
                });
            }
        };
        events.output(
            run,
            ScriptStream::Log,
            format!("mock: loaded signal {}", signal.get()),
        );

        let analysis = match runtime.analyze(signal, &AnalysisSettings::default()) {
            Ok(analysis) => analysis,
            Err(error) => {
                events.output(run, ScriptStream::Stderr, format!("mock: {error}"));
                events.exited(run, false, None);
                self.lock().active.remove(&run);
                return self.finish(MockScriptRecord {
                    run,
                    lease_id,
                    source,
                    signal: Some(signal),
                    analysis: None,
                    success: false,
                    error: Some(error.into()),
                });
            }
        };
        events.output(
            run,
            ScriptStream::Log,
            format!("mock: analysing {}", analysis.get()),
        );

        if let Some(job) = runtime.job_for(EntityId::Analysis(analysis)) {
            if let Err(error) = runtime.wait(job, Some(Duration::from_secs(5))) {
                events.output(run, ScriptStream::Stderr, format!("mock: {error}"));
                events.exited(run, false, None);
                self.lock().active.remove(&run);
                return self.finish(MockScriptRecord {
                    run,
                    lease_id,
                    source,
                    signal: Some(signal),
                    analysis: Some(analysis),
                    success: false,
                    error: Some(error.into()),
                });
            }
        }

        match runtime.spectrogram_data(analysis) {
            Ok(data) => events.output(
                run,
                ScriptStream::Stdout,
                format!("mock: data ready ({} frames)", data.spectrogram.len()),
            ),
            Err(error) => {
                events.output(run, ScriptStream::Stderr, format!("mock: {error}"));
                events.exited(run, false, None);
                self.lock().active.remove(&run);
                return self.finish(MockScriptRecord {
                    run,
                    lease_id,
                    source,
                    signal: Some(signal),
                    analysis: Some(analysis),
                    success: false,
                    error: Some(error.into()),
                });
            }
        }

        events.output(run, ScriptStream::Stdout, "mock: done".to_string());
        events.exited(run, true, Some(0));
        self.lock().active.remove(&run);
        self.finish(MockScriptRecord {
            run,
            lease_id,
            source,
            signal: Some(signal),
            analysis: Some(analysis),
            success: true,
            error: None,
        })
    }

    fn cancel(&self, run: ScriptRunId) {
        let sink = self.lock().active.remove(&run);
        let Some(sink) = sink else {
            return;
        };
        self.lock().cancelled.push(run);
        sink.output(run, ScriptStream::Log, "mock: cancelled".to_string());
        sink.exited(run, false, None);
    }
}

/// A [`MockRuntime`] and one lease over the shared test host.
#[cfg(test)]
pub fn mock_app() -> (Arc<MockRuntime>, Arc<dyn AppLease>) {
    let host = fft_test::TestHost::new();
    let runtime = Arc::clone(host.app().runtime_handle());
    let lease = host.lease();
    (runtime, lease)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fft_runtime::FftRuntime;

    fn run_mock(
        script: &MockScript,
        runtime: &Arc<MockRuntime>,
    ) -> (ScriptRunId, Arc<CapturingScriptEvents>) {
        let host =
            fft_test::TestHost::with_app(fft_test::TestApp::with_runtime(Arc::clone(runtime)));
        let lease = host.lease();
        let events = Arc::new(CapturingScriptEvents::new());
        let sink: Arc<dyn ScriptEventSink> = events.clone();
        let run = script
            .run(
                lease,
                ScriptSource::Inline {
                    name: "analysis.py".into(),
                    source: "print('hi')".into(),
                },
                sink,
            )
            .unwrap();
        (run, events)
    }

    #[test]
    fn mock_script_reaches_the_runtime_and_emits_ordered_events() {
        let runtime = Arc::new(MockRuntime::new());
        let script = MockScript::new();
        let (run, events) = run_mock(&script, &runtime);
        assert_eq!(run, ScriptRunId(1));

        let record = script.last_record().unwrap();
        assert!(record.success);
        assert_eq!(record.lease_id, LeaseId(1));
        let signal = record.signal.unwrap();
        let analysis = record.analysis.unwrap();
        assert!(runtime.signal_info(signal).is_some());
        assert!(runtime.analysis_info(analysis).is_some());
        assert!(runtime.job_for(EntityId::Analysis(analysis)).is_some());

        let events = events.events();
        assert_eq!(
            events.last(),
            Some(&ScriptEvent::Exited {
                run,
                success: true,
                code: Some(0),
            })
        );
        let exit_index = events
            .iter()
            .position(|event| matches!(event, ScriptEvent::Exited { .. }))
            .unwrap();
        assert_eq!(exit_index, events.len() - 1, "exit comes last");
        assert!(events[..exit_index]
            .iter()
            .all(|event| matches!(event, ScriptEvent::Output { .. })));
        let lines: Vec<_> = events[..exit_index]
            .iter()
            .map(|event| match event {
                ScriptEvent::Output { line, .. } => line.as_str(),
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(lines.first(), Some(&"mock: starting analysis.py"));
        assert_eq!(lines.last(), Some(&"mock: done"));
        assert!(lines.iter().any(|line| line.contains("mock: analysing")));
    }

    #[test]
    fn deterministic_run_ids_are_allocated_per_call() {
        let runtime = Arc::new(MockRuntime::new());
        let script = MockScript::new();
        let (first, _) = run_mock(&script, &runtime);
        let (second, _) = run_mock(&script, &runtime);
        assert_eq!(first, ScriptRunId(1));
        assert_eq!(second, ScriptRunId(2));
        assert_eq!(script.records().len(), 2);
        assert!(script.active_runs().is_empty());
    }

    #[test]
    fn runtime_failure_makes_the_script_exit_nonzero() {
        let runtime = Arc::new(MockRuntime::new());
        runtime.fail_next_analysis("synthetic script analysis failure");
        let script = MockScript::new();
        let (run, events) = run_mock(&script, &runtime);
        let record = script.last_record().unwrap();
        assert!(!record.success);
        assert!(record.signal.is_some());
        assert!(record.analysis.is_some());
        assert!(record.error.is_some());
        let events = events.events();
        assert_eq!(
            events.last(),
            Some(&ScriptEvent::Exited {
                run,
                success: false,
                code: None,
            })
        );
        assert!(events.iter().any(|event| matches!(
            event,
            ScriptEvent::Output { stream: ScriptStream::Stderr, line, .. }
                if line.contains("synthetic script analysis failure")
        )));
    }

    #[test]
    fn cancel_after_completion_is_a_noop() {
        let runtime = Arc::new(MockRuntime::new());
        let script = MockScript::new();
        let (run, events) = run_mock(&script, &runtime);
        let before = events.len();
        script.cancel(run);
        assert_eq!(events.len(), before);
        assert!(script.cancelled_runs().is_empty());
    }

    #[test]
    fn mock_app_helper_exposes_a_lease_with_the_default_runtime() {
        let (runtime, lease) = mock_app();
        assert!(lease.app().capabilities().displays);
        assert!(runtime.signal_ids().is_empty());
        assert_eq!(lease.app().info().mode, fft_app::AppMode::Headless);
    }
}
