//! Tests for `fft_test`: app configuration, lease tracking, event/call
//! recording, and failure injection (dev-only coverage).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use fft_app::{
    AnalysisId, AppCapabilities, AppEvent, AppInfo, AppMode, FftApp, SpectrogramDisplayOptions,
    Subscription,
};
use fft_core::settings::AnalysisSettings;
use fft_runtime::{EntityId, FftRuntime, LoadSignalRequest, MockRuntime, OrderCutRequest};

use super::*;

fn ready_analysis(runtime: &Arc<MockRuntime>) -> AnalysisId {
    let signal = runtime
        .load_signal(LoadSignalRequest::new(vec![0.0; 32], 8_000.0))
        .unwrap();
    runtime
        .analyze(signal, &AnalysisSettings::default())
        .unwrap()
}

#[test]
fn host_mints_unique_leases_over_one_shared_app() {
    let host = TestHost::new();
    let first = host.lease();
    let second = host.lease();

    assert_eq!(first.lease_id(), LeaseId(1));
    assert_eq!(second.lease_id(), LeaseId(2));
    assert_eq!(host.created_lease_ids(), vec![LeaseId(1), LeaseId(2)]);
    host.assert_active_leases(2);

    let host_app: Arc<dyn FftApp> = host.app_arc() as Arc<dyn FftApp>;
    assert!(Arc::ptr_eq(&host_app, &first.app_arc()));
    assert!(Arc::ptr_eq(&first.app_arc(), &second.app_arc()));
    assert_eq!(first.app().info().name, "fft_audio-test");
    assert_eq!(second.app_arc().info().mode, AppMode::Headless);

    // Obtaining an app handle does not mint a lease or change the id.
    let id = first.lease_id();
    let _ = first.app_arc();
    assert_eq!(first.lease_id(), id);
}

#[test]
fn dropping_one_lease_leaves_the_app_and_other_leases_usable() {
    let host = TestHost::new();
    let first = host.lease();
    let second = host.lease();
    let runtime = Arc::clone(host.app().runtime_handle());
    let analysis = ready_analysis(&runtime);

    let display = first
        .app()
        .show_spectrogram(analysis, &SpectrogramDisplayOptions::default())
        .unwrap();
    drop(first);
    host.assert_active_leases(1);

    assert_eq!(second.app().display(display).unwrap().id, display);
    second.app().close_display(display).unwrap();
    assert!(host.app().displays().unwrap().is_empty());
    assert!(runtime.analysis_ids().is_empty());
}

#[test]
fn lease_tracking_reports_created_dropped_and_active_ids() {
    let host = TestHost::new();
    let first = host.lease();
    let second = host.lease();
    let first_id = first.lease_id();
    let second_id = second.lease_id();

    assert!(host.is_lease_active(first_id));
    drop(first);
    assert!(!host.is_lease_active(first_id));
    assert!(host.is_lease_active(second_id));
    host.assert_active_leases(1);
    host.assert_lease_dropped(first_id);

    drop(second);
    host.assert_all_leases_dropped();
    host.assert_lease_dropped(second_id);
    assert_eq!(host.created_lease_ids(), vec![first_id, second_id]);
    assert_eq!(host.dropped_lease_ids(), vec![first_id, second_id]);
}

#[test]
fn events_and_subscriptions_are_recorded() {
    let host = TestHost::new();
    let lease = host.lease();
    let runtime = Arc::clone(host.app().runtime_handle());
    let analysis = ready_analysis(&runtime);
    host.app().clear_events();

    let captured = Arc::new(Mutex::new(Vec::<AppEvent>::new()));
    let target = Arc::clone(&captured);
    let subscription: Subscription = lease.app().subscribe(Arc::new(move |event| {
        target.lock().unwrap().push(event);
    }));
    assert_eq!(host.app().subscribe_calls(), 1);
    assert_eq!(host.app().user_subscription_count(), 1);
    assert_eq!(
        host.app().subscription_count(),
        2,
        "the internal recorder plus the test sink"
    );

    let display = lease
        .app()
        .show_spectrogram(analysis, &SpectrogramDisplayOptions::default())
        .unwrap();
    assert_eq!(host.app().display_open_count(), 1);
    lease.app().set_display_visible(display, false).unwrap();
    assert_eq!(host.app().display_update_count(), 1);
    lease.app().close_display(display).unwrap();
    assert_eq!(host.app().display_close_count(), 1);

    let events = host.app().events();
    assert_eq!(events.len(), 3);
    assert!(matches!(events[0], AppEvent::DisplayOpened { .. }));
    assert!(matches!(events[1], AppEvent::DisplayUpdated { .. }));
    assert!(matches!(events[2], AppEvent::DisplayClosed { .. }));
    assert_eq!(*captured.lock().unwrap(), events);

    assert!(host.app().unsubscribe(subscription));
    assert_eq!(host.app().user_subscription_count(), 0);
}

#[test]
fn runtime_job_events_are_recorded_as_app_events() {
    let host = TestHost::new();
    let runtime = Arc::clone(host.app().runtime_handle());
    host.app().clear_events();
    let signal = runtime
        .load_signal(LoadSignalRequest::new(vec![0.0; 16], 8_000.0))
        .unwrap();
    runtime
        .analyze(signal, &AnalysisSettings::default())
        .unwrap();

    let events = host.app().events();
    assert!(events
        .iter()
        .any(|event| matches!(event, AppEvent::JobProgress { .. })));
    assert!(events
        .iter()
        .any(|event| matches!(event, AppEvent::JobDone { .. })));
}

#[test]
fn app_calls_are_recorded() {
    let host = TestHost::new();
    let lease = host.lease();
    let runtime = Arc::clone(host.app().runtime_handle());
    let analysis = ready_analysis(&runtime);
    host.app().clear_calls();

    let display = lease
        .app()
        .show_spectrogram(analysis, &SpectrogramDisplayOptions::default())
        .unwrap();
    assert!(lease.app().runtime().spectrogram_data(analysis).is_ok());
    assert_eq!(lease.app().displays().unwrap().len(), 1);
    assert_eq!(lease.app().display(display).unwrap().id, display);
    lease.app().set_display_visible(display, false).unwrap();
    lease.app().close_display(display).unwrap();
    lease.app().wait_all(Some(Duration::from_secs(1))).unwrap();

    let calls = host.app().calls();
    assert_eq!(calls[0], TestCall::ShowSpectrogram(analysis));
    assert!(calls.contains(&TestCall::Runtime));
    assert!(calls.contains(&TestCall::Displays));
    assert!(calls.contains(&TestCall::Display(display)));
    assert!(calls.contains(&TestCall::SetDisplayVisible {
        id: display,
        visible: false
    }));
    assert!(calls.contains(&TestCall::CloseDisplay(display)));
    assert!(calls.contains(&TestCall::WaitAll(Some(Duration::from_secs(1)))));

    host.app().clear_calls();
    assert_eq!(host.app().call_count(), 0);
}

#[test]
fn failure_injection_covers_busy_missing_and_failed_jobs() {
    let host = TestHost::with_app(TestApp::held());
    let lease = host.lease();
    let runtime = Arc::clone(host.app().runtime_handle());
    let signal = runtime
        .load_signal(LoadSignalRequest::new(vec![0.0; 32], 8_000.0))
        .unwrap();
    let analysis = runtime
        .analyze(signal, &AnalysisSettings::default())
        .unwrap();

    let busy = lease
        .app()
        .show_spectrogram(analysis, &SpectrogramDisplayOptions::default())
        .unwrap_err();
    assert_eq!(busy.code(), "busy");
    assert!(lease.app().displays().unwrap().is_empty());

    let job = runtime.job_for(EntityId::Analysis(analysis)).unwrap();
    runtime.complete(job).unwrap();
    let display = lease
        .app()
        .show_spectrogram(analysis, &SpectrogramDisplayOptions::default())
        .unwrap();

    let _ = host.app().abandon_display(display);
    assert_eq!(
        lease.app().display(display).unwrap_err().code(),
        "not_found"
    );
    assert!(host
        .app()
        .calls()
        .contains(&TestCall::AbandonDisplay(display)));

    let signal = runtime
        .load_signal(LoadSignalRequest::new(vec![0.0; 32], 8_000.0))
        .unwrap();
    host.app()
        .fail_next_analysis("synthetic app analysis failure");
    let failed_analysis = runtime
        .analyze(signal, &AnalysisSettings::default())
        .unwrap();
    let job = runtime
        .job_for(EntityId::Analysis(failed_analysis))
        .unwrap();
    let error = runtime.wait(job, None).unwrap_err();
    assert!(error.to_string().contains("synthetic app analysis failure"));

    host.app().fail_next_cut("synthetic app cut failure");
    let cut = runtime
        .order_cut(analysis, &OrderCutRequest::default())
        .unwrap();
    let job = runtime.job_for(EntityId::Cut(cut)).unwrap();
    let error = runtime.wait(job, None).unwrap_err();
    assert!(error.to_string().contains("synthetic app cut failure"));
}

#[test]
fn info_and_capabilities_are_configurable() {
    let info = AppInfo {
        name: "custom-test-app".into(),
        version: "9.9.9".into(),
        mode: AppMode::Desktop,
    };
    let capabilities = AppCapabilities {
        displays: false,
        table_formats: Vec::new(),
    };
    let host = TestHost::with_app(
        TestApp::new()
            .with_info(info.clone())
            .with_capabilities(capabilities.clone()),
    );
    let lease = host.lease();

    assert_eq!(lease.app().info(), info);
    assert_eq!(lease.app().capabilities(), capabilities);
    assert_eq!(host.app().info(), info);
}

#[test]
fn shutdown_is_recorded_and_blocks_new_work() {
    let host = TestHost::new();
    let lease = host.lease();
    lease.app().shutdown();
    lease.app().shutdown();

    assert!(host.app().is_shutdown());
    assert_eq!(
        lease
            .app()
            .show_spectrogram(AnalysisId(1), &SpectrogramDisplayOptions::default())
            .unwrap_err()
            .code(),
        "closed"
    );
    assert!(host.app().calls().contains(&TestCall::Shutdown));
}

#[test]
fn convenience_helper_returns_a_usable_lease() {
    let (runtime, lease) = test_app_and_lease();
    assert!(lease.app().capabilities().displays);
    assert_eq!(lease.app().info().mode, AppMode::Headless);
    assert!(runtime.signal_ids().is_empty());
}
