//! Tests for the lease-aware scripting contract and sandboxed events.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use fft_app::{AppEvent, AppLease};
use fft_test::TestHost;

use super::*;

fn assert_object_safe(_: &dyn FftScript) {}
fn assert_send_sync<T: Send + Sync + ?Sized>() {}

#[test]
fn trait_is_object_safe_and_send_sync() {
    assert_send_sync::<dyn FftScript>();
    let script = MockScript::new();
    assert_object_safe(&script);
    let boxed: Arc<dyn FftScript> = Arc::new(script);
    drop(boxed);
}

#[test]
fn script_source_names_are_typed() {
    let inline = ScriptSource::Inline {
        name: "analysis.py".into(),
        source: "print('hi')".into(),
    };
    assert_eq!(inline.name(), "analysis.py");
    assert_eq!(inline.inline_source(), Some("print('hi')"));

    let file = ScriptSource::File(PathBuf::from("/tmp/run.lua"));
    assert_eq!(file.name(), "run.lua");
    assert_eq!(file.inline_source(), None);
}

#[test]
fn run_receives_the_lease_and_drives_the_app() {
    let host = TestHost::new();
    let lease = host.lease();
    let lease_id = lease.lease_id();
    let runtime = Arc::clone(host.app().runtime_handle());
    let script = MockScript::new();
    let events = Arc::new(CapturingScriptEvents::new());
    let sink: Arc<dyn ScriptEventSink> = events.clone();

    let run = script
        .run(
            Arc::clone(&lease),
            ScriptSource::Inline {
                name: "lease.py".into(),
                source: "print('lease')".into(),
            },
            sink,
        )
        .unwrap();

    assert_eq!(run, ScriptRunId(1));
    let record = script.last_record().unwrap();
    assert_eq!(record.lease_id, lease_id);
    assert!(record.success);
    let signal = record.signal.unwrap();
    let analysis = record.analysis.unwrap();
    assert!(runtime.signal_info(signal).is_some());
    assert!(runtime.analysis_info(analysis).is_some());

    // The lease remains usable after the run, and the run's Arc did not mint a
    // second lease.
    assert!(lease.app().capabilities().displays);
    assert_eq!(lease.lease_id(), lease_id);
    assert_eq!(host.created_lease_ids(), vec![lease_id]);
    host.assert_active_leases(1);
    drop(lease);
    host.assert_all_leases_dropped();
}

#[test]
fn script_events_do_not_become_app_event_payloads() {
    let host = TestHost::new();
    let lease = host.lease();
    let script = MockScript::new();
    let events = Arc::new(CapturingScriptEvents::new());
    let sink: Arc<dyn ScriptEventSink> = events.clone();

    let captured = Arc::new(Mutex::new(Vec::<AppEvent>::new()));
    let target = Arc::clone(&captured);
    lease.app().subscribe(Arc::new(move |event| {
        target.lock().unwrap().push(event);
    }));

    script
        .run(
            lease,
            ScriptSource::Inline {
                name: "events.py".into(),
                source: "print('events')".into(),
            },
            sink,
        )
        .unwrap();

    let app_events = captured.lock().unwrap();
    // The mock runtime may emit legitimate job events; none of them, and no
    // display event, is a script output/exit event.
    assert!(app_events.iter().all(|event| matches!(
        event,
        AppEvent::JobProgress { .. }
            | AppEvent::JobDone { .. }
            | AppEvent::JobFailed { .. }
            | AppEvent::DisplayOpened { .. }
            | AppEvent::DisplayUpdated { .. }
            | AppEvent::DisplayClosed { .. }
    )));
    let app_json = serde_json::to_string(&*app_events).unwrap();
    assert!(!app_json.contains("script_output"));
    assert!(!app_json.contains("script_exited"));

    let script_events = events.events();
    assert!(!script_events.is_empty());
    assert!(script_events
        .iter()
        .any(|event| matches!(event, ScriptEvent::Output { .. })));
    assert!(script_events
        .iter()
        .any(|event| matches!(event, ScriptEvent::Exited { .. })));
}

#[test]
fn a_second_run_uses_its_own_lease_id() {
    let host = TestHost::new();
    let script = MockScript::new();
    let first = host.lease();
    let second = host.lease();
    let first_id = first.lease_id();
    let second_id = second.lease_id();

    let run = |lease: Arc<dyn AppLease>| {
        script
            .run(
                lease,
                ScriptSource::Inline {
                    name: "run.py".into(),
                    source: "print('run')".into(),
                },
                Arc::new(CapturingScriptEvents::new()),
            )
            .unwrap()
    };
    run(first);
    run(second);

    let records = script.records();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].lease_id, first_id);
    assert_eq!(records[1].lease_id, second_id);
    assert_ne!(first_id, second_id);
}

#[test]
fn mock_script_can_run_with_the_test_lease_helper() {
    let (runtime, lease) = crate::testing::mock_app();
    let script = MockScript::new();
    let events = Arc::new(CapturingScriptEvents::new());
    let sink: Arc<dyn ScriptEventSink> = events.clone();
    script
        .run(
            lease,
            ScriptSource::Inline {
                name: "helper.py".into(),
                source: "print('helper')".into(),
            },
            sink,
        )
        .unwrap();
    assert!(script.last_record().unwrap().success);
    assert!(!runtime.signal_ids().is_empty());
    assert!(!events.is_empty());
}
