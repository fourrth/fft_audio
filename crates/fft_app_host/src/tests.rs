//! Tests for `fft_app_host`.
//!
//! The host is exercised against `fft_runtime::MockRuntime` for deterministic
//! display/lease/lifecycle coverage, plus one concrete `CoreRuntime` test for
//! the job-event bridge.

use super::{AppHost, AppHostConfig, HostRuntimeSink};

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use fft_app::{
    AnalysisId, AppCapabilities, AppEvent, AppInfo, AppLease, AppMode, CutDisplayOptions,
    DisplayId, DisplayKind, DisplayOwner, DisplayRegistry, FftApp, LeaseId, PlotDisplayOptions,
    SignalId, SpectrogramDisplayOptions, SCRIPTED_DISPLAY_ID_BASE,
};
use fft_core::orders::LineSpec;
use fft_core::session::{JsonSessionDisplay, SessionDocument};
use fft_core::settings::{AnalysisSettings, LineCutSettings, OrderCutSettings};
use fft_core::table::PlotLayout;
use fft_runtime::{
    EntityId, FftRuntime, JobId, LineCutRequest, LoadSignalRequest, MockRuntime, OrderCutRequest,
    RuntimeEvent, RuntimeEventSink,
};
use serde_json::json;

fn test_config() -> AppHostConfig {
    AppHostConfig {
        name: "fft_audio-test".to_string(),
        version: "0.1.0".to_string(),
        mode: AppMode::Headless,
        capabilities: AppCapabilities::for_build(),
        workers: 1,
        persisted_session: None,
    }
}

fn mock_host() -> (Arc<AppHost>, Arc<MockRuntime>) {
    let runtime = Arc::new(MockRuntime::new());
    let host = AppHost::from_runtime(runtime.clone(), test_config());
    (host, runtime)
}

fn held_host() -> (Arc<AppHost>, Arc<MockRuntime>) {
    let runtime = Arc::new(MockRuntime::new_held());
    let host = AppHost::from_runtime(runtime.clone(), test_config());
    (host, runtime)
}

fn ready_analysis(runtime: &MockRuntime) -> (SignalId, AnalysisId) {
    let signal = runtime
        .load_signal(LoadSignalRequest::new(vec![0.0; 16], 8_000.0))
        .unwrap();
    let analysis = runtime
        .analyze(signal, &AnalysisSettings::default())
        .unwrap();
    (signal, analysis)
}

fn capture(host: &AppHost) -> Arc<StdMutex<Vec<AppEvent>>> {
    let events = Arc::new(StdMutex::new(Vec::new()));
    let target = Arc::clone(&events);
    host.subscribe(Arc::new(move |event: AppEvent| {
        target
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(event);
    }));
    events
}

fn line() -> LineSpec {
    LineSpec {
        t0: 0.0,
        f0: 100.0,
        t1: 1.0,
        f1: 200.0,
        log_f: false,
    }
}

fn live(runtime: &MockRuntime) -> (usize, usize, usize, usize) {
    (
        runtime.signal_ids().len(),
        runtime.analysis_ids().len(),
        runtime.cut_ids().len(),
        runtime.job_ids().len(),
    )
}

#[test]
fn new_reports_config_and_starts_empty() {
    let capabilities = AppCapabilities {
        displays: true,
        table_formats: vec![],
    };
    let config = AppHostConfig {
        name: "custom-host".to_string(),
        version: "9.9.9".to_string(),
        mode: AppMode::Desktop,
        capabilities: capabilities.clone(),
        workers: 1,
        persisted_session: None,
    };
    let host = AppHost::new(config);

    assert_eq!(
        host.info(),
        AppInfo {
            name: "custom-host".to_string(),
            version: "9.9.9".to_string(),
            mode: AppMode::Desktop,
        }
    );
    assert_eq!(host.capabilities(), capabilities);
    assert!(!host.is_shutdown());
    assert!(host.displays().unwrap().is_empty());
    assert!(host.registry().list().is_empty());
    assert_eq!(host.display(DisplayId(7)).unwrap_err().code(), "not_found");
    host.runtime()
        .wait_all(Some(Duration::from_secs(1)))
        .unwrap();
}

#[test]
fn runtime_event_bridge_maps_events_and_drops_cancellations() {
    let registry = Arc::new(DisplayRegistry::new(None));
    let events = Arc::new(StdMutex::new(Vec::new()));
    let target = Arc::clone(&events);
    registry.subscribe(Arc::new(move |event: AppEvent| {
        target
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(event);
    }));

    let sink = HostRuntimeSink {
        registry: Arc::downgrade(&registry),
    };
    sink.emit(RuntimeEvent::Progress {
        job: JobId(7),
        frames_done: 2,
        frames_total: Some(4),
        eta_s: Some(0.5),
    });
    sink.emit(RuntimeEvent::Done { job: JobId(7) });
    sink.emit(RuntimeEvent::Failed {
        job: JobId(8),
        message: "boom".to_string(),
    });
    sink.emit(RuntimeEvent::Cancelled { job: JobId(9) });

    assert_eq!(
        *events.lock().unwrap(),
        vec![
            AppEvent::JobProgress {
                job: JobId(7),
                frames_done: 2,
                frames_total: Some(4),
                eta_s: Some(0.5),
            },
            AppEvent::JobDone { job: JobId(7) },
            AppEvent::JobFailed {
                job: JobId(8),
                message: "boom".to_string(),
            },
        ]
    );

    drop(registry);
    sink.emit(RuntimeEvent::Done { job: JobId(10) });
    assert_eq!(
        events.lock().unwrap().len(),
        3,
        "a dropped registry is not resurrected by a runtime handle"
    );
}

#[test]
fn new_wires_concrete_runtime_job_events_into_app_events() {
    let host = AppHost::new(AppHostConfig {
        workers: 1,
        ..test_config()
    });
    let captured = capture(&host);
    let runtime = host.runtime();
    let signal = runtime
        .load_signal(LoadSignalRequest::new(vec![1.0; 8192], 8_000.0))
        .unwrap();
    let analysis = runtime
        .analyze(signal, &AnalysisSettings::default())
        .unwrap();
    let job = runtime.job_for(EntityId::Analysis(analysis)).unwrap();
    runtime.wait(job, Some(Duration::from_secs(5))).unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let done = captured
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event, AppEvent::JobDone { job: done } if *done == job));
        if done {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the runtime job completion was not bridged to an app event"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn from_runtime_does_not_create_a_job_event_bridge() {
    let runtime = Arc::new(MockRuntime::new());
    let host = AppHost::from_runtime(runtime.clone(), test_config());
    let captured = capture(&host);

    let signal = runtime
        .load_signal(LoadSignalRequest::new(vec![0.0; 16], 8_000.0))
        .unwrap();
    let analysis = runtime
        .analyze(signal, &AnalysisSettings::default())
        .unwrap();
    let job = runtime.job_for(EntityId::Analysis(analysis)).unwrap();
    runtime.wait(job, None).unwrap();
    assert!(
        runtime
            .events()
            .iter()
            .any(|event| matches!(event, RuntimeEvent::Done { job: done } if *done == job)),
        "the mock runtime did emit job events"
    );

    let id = host
        .show_spectrogram(analysis, &SpectrogramDisplayOptions::default())
        .unwrap();
    let events = captured.lock().unwrap().clone();
    assert_eq!(
        events,
        vec![AppEvent::DisplayOpened {
            display: host.display(id).unwrap()
        }],
        "from_runtime must not invent a runtime sink"
    );
}

#[test]
fn leases_are_unique_independent_and_reject_minting_after_shutdown() {
    fn assert_send_sync<T: Send + Sync + ?Sized>() {}
    assert_send_sync::<dyn AppLease>();

    let (host, runtime) = mock_host();
    let (_, analysis) = ready_analysis(&runtime);

    let first = host.lease();
    let second = host.lease();
    assert_eq!(first.lease_id(), LeaseId(1));
    assert_eq!(second.lease_id(), LeaseId(2));
    assert_ne!(first.lease_id(), second.lease_id());

    // Sharing an `Arc<dyn AppLease>` does not create a second lease, and a
    // lease exposes only `lease_id`/`app`/`app_arc` (no minting method).
    let shared = Arc::clone(&first);
    assert_eq!(shared.lease_id(), first.lease_id());
    let direct = first.app_arc();
    let again = first.app_arc();
    assert!(Arc::ptr_eq(&direct, &again));
    assert_eq!(
        first.lease_id(),
        LeaseId(1),
        "app_arc clones the app handle; it does not mint"
    );
    let opened = shared
        .app()
        .show_spectrogram(analysis, &SpectrogramDisplayOptions::default())
        .unwrap();
    assert_eq!(second.app().display(opened).unwrap().id, opened);
    assert_eq!(first.app_arc().info().name, host.info().name);

    drop(first);
    drop(shared);
    // Dropping one lease leaves the app, the sibling lease, and its displays
    // fully usable.
    assert!(second.app().display(opened).is_ok());
    host.close_display(opened).unwrap();
    drop(second);

    let third = host.lease();
    assert_eq!(third.lease_id(), LeaseId(3));
    drop(third);

    host.shutdown();
    let mint = catch_unwind(AssertUnwindSafe(|| host.lease()));
    assert!(mint.is_err(), "shutdown must reject new leases");
}

#[test]
fn backend_ids_are_seeded_from_the_session_and_never_reused() {
    let persisted = SessionDocument {
        displays: vec![JsonSessionDisplay {
            id: SCRIPTED_DISPLAY_ID_BASE + 5,
            parent: None,
            kind: "spectrogram".to_string(),
            name: "persisted".to_string(),
            state: json!({}),
        }],
        ..SessionDocument::default()
    };
    let runtime = Arc::new(MockRuntime::new());
    let host = AppHost::from_runtime(
        runtime.clone(),
        AppHostConfig {
            persisted_session: Some(persisted),
            ..test_config()
        },
    );
    let (_, analysis) = ready_analysis(&runtime);

    let first = host
        .show_spectrogram(analysis, &SpectrogramDisplayOptions::default())
        .unwrap();
    assert_eq!(first, DisplayId(SCRIPTED_DISPLAY_ID_BASE + 6));

    let second = host
        .show_spectrogram(analysis, &SpectrogramDisplayOptions::default())
        .unwrap();
    assert_eq!(second, DisplayId(SCRIPTED_DISPLAY_ID_BASE + 7));

    host.close_display(first).unwrap();
    let third = host
        .show_spectrogram(analysis, &SpectrogramDisplayOptions::default())
        .unwrap();
    assert_eq!(
        third,
        DisplayId(SCRIPTED_DISPLAY_ID_BASE + 8),
        "closed backend ids are not reused"
    );
    assert!(host
        .registry()
        .list()
        .iter()
        .all(|display| display.id >= DisplayId(SCRIPTED_DISPLAY_ID_BASE)));
}

#[test]
fn explicit_user_ids_stay_below_the_backend_range() {
    let (host, runtime) = mock_host();
    let (signal, analysis) = ready_analysis(&runtime);

    let user = host
        .show_spectrogram_with_id(
            DisplayId(42),
            analysis,
            Some(signal),
            &SpectrogramDisplayOptions::default().with_name("user"),
        )
        .unwrap();
    assert_eq!(user, DisplayId(42));
    let record = host.registry().record(user).unwrap();
    assert_eq!(record.owner, DisplayOwner::User);
    assert_eq!(record.info.state["sourceKind"], "user");
    assert_eq!(record.info.state["scripted"], false);
    assert_eq!(record.info.name, "user");

    let backend = host
        .show_spectrogram(analysis, &SpectrogramDisplayOptions::default())
        .unwrap();
    assert_eq!(backend, DisplayId(SCRIPTED_DISPLAY_ID_BASE));
    assert_eq!(
        host.registry().record(backend).unwrap().owner,
        DisplayOwner::Backend
    );

    let duplicate = host
        .show_spectrogram_with_id(user, analysis, None, &SpectrogramDisplayOptions::default())
        .unwrap_err();
    assert_eq!(duplicate.code(), "invalid_input");
    let reserved = host
        .show_spectrogram_with_id(
            DisplayId(SCRIPTED_DISPLAY_ID_BASE),
            analysis,
            None,
            &SpectrogramDisplayOptions::default(),
        )
        .unwrap_err();
    assert_eq!(reserved.code(), "invalid_input");
    let zero = host
        .show_spectrogram_with_id(
            DisplayId(0),
            analysis,
            None,
            &SpectrogramDisplayOptions::default(),
        )
        .unwrap_err();
    assert_eq!(zero.code(), "invalid_input");

    assert_eq!(
        host.displays().unwrap().len(),
        2,
        "rejected explicit ids do not create records"
    );
}

#[test]
fn user_pending_registration_accepts_an_existing_analysis_job() {
    let (host, runtime) = held_host();
    let signal = runtime
        .load_signal(LoadSignalRequest::new(vec![0.0; 16], 8_000.0))
        .unwrap();
    let analysis = runtime
        .analyze(signal, &AnalysisSettings::default())
        .unwrap();

    let user = host
        .register_user_spectrogram_with_id(
            DisplayId(42),
            analysis,
            Some(signal),
            &SpectrogramDisplayOptions::default(),
        )
        .unwrap();
    assert_eq!(user, DisplayId(42));
    let record = host.registry().record(user).unwrap();
    assert_eq!(record.owner, DisplayOwner::User);
    assert_eq!(record.info.name, "Spectrogram");
    assert_eq!(record.info.state["analysisId"], analysis.get());
    assert_eq!(record.info.state["signalId"], signal.get());
    assert_eq!(record.info.state["sourceKind"], "user");

    let missing = host
        .register_user_spectrogram_with_id(
            DisplayId(43),
            AnalysisId(999),
            None,
            &SpectrogramDisplayOptions::default(),
        )
        .unwrap_err();
    assert_eq!(missing.code(), "not_found");
    let busy = host
        .show_spectrogram_with_id(
            DisplayId(43),
            analysis,
            None,
            &SpectrogramDisplayOptions::default(),
        )
        .unwrap_err();
    assert_eq!(busy.code(), "busy");
    assert_eq!(host.displays().unwrap().len(), 1);

    let analysis_job = runtime.job_for(EntityId::Analysis(analysis)).unwrap();
    runtime.complete(analysis_job).unwrap();
    assert!(runtime.spectrogram_data(analysis).is_ok());

    host.close_display(user).unwrap();
    assert_eq!(live(&runtime), (0, 0, 0, 0));
}

#[test]
fn show_update_and_visibility_emit_typed_display_events() {
    let (host, runtime) = mock_host();
    let (signal, analysis) = ready_analysis(&runtime);
    let order = runtime
        .order_cut(analysis, &OrderCutRequest::default())
        .unwrap();
    let line_cut = runtime
        .line_cut(
            analysis,
            &LineCutRequest::new(LineCutSettings::default(), vec![line()]),
        )
        .unwrap();
    let events = capture(&host);

    let spec = host
        .show_spectrogram_with_signal(
            analysis,
            Some(signal),
            &SpectrogramDisplayOptions::default().with_name("spec"),
        )
        .unwrap();
    let info = host.display(spec).unwrap();
    assert_eq!(info.id, spec);
    assert_eq!(info.kind, DisplayKind::Spectrogram);
    assert_eq!(info.parent, None);
    assert!(info.visible);
    assert!(!info.ephemeral);
    assert_eq!(info.name, "spec");
    assert_eq!(info.state["analysisId"], analysis.get());
    assert_eq!(info.state["signalId"], signal.get());
    assert_eq!(info.state["sourceKind"], "backend");
    assert_eq!(info.state["scripted"], true);
    assert!(info.state.get("settings").is_some());

    let updated = host
        .update_display(
            spec,
            json!({ "analysisId": analysis.get(), "zoom": 2.0 }),
            false,
        )
        .unwrap();
    assert!(!updated.visible);
    assert_eq!(updated.state["zoom"], 2.0);
    assert_eq!(host.display(spec).unwrap(), updated);

    let visible = host.set_display_visible(spec, true).unwrap();
    assert!(visible.visible);
    assert_eq!(host.display(spec).unwrap(), visible);

    let order_display = host
        .show_order_cut(
            spec,
            order,
            &CutDisplayOptions::order(OrderCutSettings::default()),
        )
        .unwrap();
    let order_info = host.display(order_display).unwrap();
    assert_eq!(order_info.parent, Some(spec));
    assert_eq!(order_info.kind, DisplayKind::OrderCuts);
    assert_eq!(order_info.state["analysisId"], analysis.get());
    assert_eq!(order_info.state["cutId"], order.get());
    assert_eq!(order_info.state["sourceKind"], "backend");
    assert_eq!(order_info.state["scripted"], true);
    assert!(order_info.state.get("settings").is_some());

    let line_display = host
        .show_line_cut(
            spec,
            line_cut,
            &CutDisplayOptions::line(LineCutSettings::default()),
        )
        .unwrap();
    assert_eq!(
        host.display(line_display).unwrap().kind,
        DisplayKind::LineCuts
    );

    let captured = events.lock().unwrap().clone();
    assert_eq!(captured.len(), 5, "three opens, one update, one visibility");
    assert!(matches!(
        &captured[0],
        AppEvent::DisplayOpened { display } if display.id == spec && display.name == "spec"
    ));
    assert!(matches!(
        &captured[1],
        AppEvent::DisplayUpdated { display }
            if display.id == spec
                && !display.visible
                && display.state["zoom"] == 2.0
    ));
    assert!(matches!(
        &captured[2],
        AppEvent::DisplayUpdated { display } if display.id == spec && display.visible
    ));
    assert!(matches!(
        &captured[3],
        AppEvent::DisplayOpened { display }
            if display.id == order_display && display.parent == Some(spec)
    ));
    assert!(matches!(
        &captured[4],
        AppEvent::DisplayOpened { display } if display.id == line_display
    ));
}

#[test]
fn close_cascades_releases_runtime_and_is_idempotent() {
    let (host, runtime) = mock_host();
    let (signal, analysis) = ready_analysis(&runtime);
    let order = runtime
        .order_cut(analysis, &OrderCutRequest::default())
        .unwrap();
    let line_cut = runtime
        .line_cut(
            analysis,
            &LineCutRequest::new(LineCutSettings::default(), vec![line()]),
        )
        .unwrap();

    let spec = host
        .show_spectrogram_with_signal(
            analysis,
            Some(signal),
            &SpectrogramDisplayOptions::default(),
        )
        .unwrap();
    let order_display = host
        .show_order_cut(
            spec,
            order,
            &CutDisplayOptions::order(OrderCutSettings::default()),
        )
        .unwrap();
    let line_display = host
        .show_line_cut(
            spec,
            line_cut,
            &CutDisplayOptions::line(LineCutSettings::default()),
        )
        .unwrap();
    assert_eq!(live(&runtime), (1, 1, 2, 3));

    let events = capture(&host);
    host.close_display(spec).unwrap();
    assert!(host.displays().unwrap().is_empty());
    assert_eq!(live(&runtime), (0, 0, 0, 0));

    let captured = events.lock().unwrap().clone();
    assert_eq!(captured.len(), 1);
    assert!(matches!(
        &captured[0],
        AppEvent::DisplayClosed { id, children }
            if *id == spec && *children == vec![order_display, line_display]
    ));

    // Re-closing and unknown ids are successful no-ops with no event.
    host.close_display(spec).unwrap();
    host.close_display(DisplayId(9_999_999)).unwrap();
    assert_eq!(events.lock().unwrap().len(), 1);
}

#[test]
fn ephemeral_displays_are_marked_and_filtered_from_the_session() {
    let (host, runtime) = mock_host();
    let (_, analysis) = ready_analysis(&runtime);

    let ephemeral = host
        .show_spectrogram(
            analysis,
            &SpectrogramDisplayOptions::default()
                .with_name("in-memory")
                .with_ephemeral(true),
        )
        .unwrap();
    let info = host.display(ephemeral).unwrap();
    assert!(info.ephemeral);
    assert_eq!(info.name, "in-memory");
    assert_eq!(info.state["ephemeral"], true);
    assert!(
        host.registry().session_displays().is_empty(),
        "ephemeral displays are excluded from session/undo"
    );

    let persistent = host
        .show_spectrogram(
            analysis,
            &SpectrogramDisplayOptions::default().with_name("saved"),
        )
        .unwrap();
    let session = host.registry().session_displays();
    assert_eq!(session.len(), 1);
    assert_eq!(session[0].id, persistent.get());
    assert_eq!(session[0].kind, "spectrogram");
    assert_eq!(host.registry().list().len(), 2);

    host.close_display(ephemeral).unwrap();
    host.close_display(persistent).unwrap();
}

#[test]
fn pending_displays_register_before_data_and_validate_parents_and_kinds() {
    let (host, runtime) = held_host();
    let signal = runtime
        .load_signal(LoadSignalRequest::new(vec![0.0; 16], 8_000.0))
        .unwrap();
    let analysis = runtime
        .analyze(signal, &AnalysisSettings::default())
        .unwrap();

    let busy = host
        .show_spectrogram(analysis, &SpectrogramDisplayOptions::default())
        .unwrap_err();
    assert_eq!(busy.code(), "busy");
    assert!(
        host.displays().unwrap().is_empty(),
        "a busy analysis must not publish a display"
    );

    let spec = host
        .show_spectrogram_pending(
            analysis,
            Some(signal),
            &SpectrogramDisplayOptions::default()
                .with_name("pending")
                .with_ephemeral(true),
        )
        .unwrap();
    let info = host.display(spec).unwrap();
    assert_eq!(info.name, "pending");
    assert!(info.ephemeral);
    assert_eq!(info.state["analysisId"], analysis.get());
    assert_eq!(info.state["signalId"], signal.get());

    let missing = host
        .show_spectrogram_pending(AnalysisId(999), None, &SpectrogramDisplayOptions::default())
        .unwrap_err();
    assert_eq!(missing.code(), "not_found");
    assert_eq!(
        host.displays().unwrap().len(),
        1,
        "a rejected pending open must not allocate an id"
    );

    let analysis_job = runtime.job_for(EntityId::Analysis(analysis)).unwrap();
    runtime.complete(analysis_job).unwrap();

    let order = runtime
        .order_cut(analysis, &OrderCutRequest::default())
        .unwrap();
    let busy_cut = host
        .show_order_cut(
            spec,
            order,
            &CutDisplayOptions::order(OrderCutSettings::default()),
        )
        .unwrap_err();
    assert_eq!(busy_cut.code(), "busy");
    assert_eq!(host.displays().unwrap().len(), 1);

    let order_display = host
        .show_order_cut_pending(
            spec,
            order,
            &CutDisplayOptions::order(OrderCutSettings::default()),
        )
        .unwrap();
    assert_eq!(order_display, DisplayId(SCRIPTED_DISPLAY_ID_BASE + 1));
    assert_eq!(host.display(order_display).unwrap().parent, Some(spec));
    assert_eq!(runtime.cut_data(order).unwrap_err().code(), "busy");

    let line_cut = runtime
        .line_cut(
            analysis,
            &LineCutRequest::new(LineCutSettings::default(), vec![line()]),
        )
        .unwrap();
    let line_display = host
        .show_line_cut_pending(
            spec,
            line_cut,
            &CutDisplayOptions::line(LineCutSettings::default()),
        )
        .unwrap();
    assert_eq!(line_display, DisplayId(SCRIPTED_DISPLAY_ID_BASE + 2));
    assert_eq!(
        host.display(line_display).unwrap().kind,
        DisplayKind::LineCuts
    );

    let missing_cut = host
        .show_line_cut_pending(
            spec,
            fft_app::CutId(99),
            &CutDisplayOptions::line(LineCutSettings::default()),
        )
        .unwrap_err();
    assert_eq!(missing_cut.code(), "not_found");
    let wrong_settings = host
        .show_line_cut_pending(
            spec,
            order,
            &CutDisplayOptions::order(OrderCutSettings::default()),
        )
        .unwrap_err();
    assert_eq!(wrong_settings.code(), "invalid_input");

    let hidden = host.set_display_visible(spec, false).unwrap();
    assert!(!hidden.visible);
    let shown = host.set_display_visible(spec, true).unwrap();
    assert!(shown.visible);

    let cut_job = runtime.job_for(EntityId::Cut(order)).unwrap();
    runtime.complete(cut_job).unwrap();
    assert!(runtime.cut_data(order).is_ok());

    host.close_display(spec).unwrap();
    assert!(host.displays().unwrap().is_empty());
    assert_eq!(live(&runtime), (0, 0, 0, 0));
}

#[test]
fn cut_opens_validate_parent_settings_and_materialized_kind() {
    let (host, runtime) = mock_host();
    let (_, analysis) = ready_analysis(&runtime);
    let order = runtime
        .order_cut(analysis, &OrderCutRequest::default())
        .unwrap();
    let line_cut = runtime
        .line_cut(
            analysis,
            &LineCutRequest::new(LineCutSettings::default(), vec![line()]),
        )
        .unwrap();
    let order_options = CutDisplayOptions::order(OrderCutSettings::default());
    let line_options = CutDisplayOptions::line(LineCutSettings::default());
    let parent = host
        .show_spectrogram(analysis, &SpectrogramDisplayOptions::default())
        .unwrap();

    let missing_parent = host
        .show_order_cut(DisplayId(9_999_999), order, &order_options)
        .unwrap_err();
    assert_eq!(missing_parent.code(), "not_found");

    let wrong_settings = host
        .show_order_cut(parent, order, &line_options)
        .unwrap_err();
    assert_eq!(wrong_settings.code(), "invalid_input");

    let wrong_kind = host
        .show_order_cut(parent, line_cut, &order_options)
        .unwrap_err();
    assert_eq!(wrong_kind.code(), "invalid_input");
    let wrong_line_kind = host
        .show_line_cut(parent, order, &line_options)
        .unwrap_err();
    assert_eq!(wrong_line_kind.code(), "invalid_input");

    let unknown_cut = host
        .show_order_cut(parent, fft_app::CutId(77), &order_options)
        .unwrap_err();
    assert_eq!(unknown_cut.code(), "not_found");

    let child = host.show_line_cut(parent, line_cut, &line_options).unwrap();
    assert_eq!(
        child,
        DisplayId(SCRIPTED_DISPLAY_ID_BASE + 1),
        "failed opens must not consume display ids"
    );

    let nested = host
        .show_order_cut(child, order, &order_options)
        .unwrap_err();
    assert_eq!(nested.code(), "invalid_input");

    let pending_wrong_kind = host
        .show_order_cut_pending(parent, line_cut, &order_options)
        .unwrap_err();
    assert_eq!(pending_wrong_kind.code(), "invalid_input");

    host.close_display(parent).unwrap();
}

#[test]
fn shared_analysis_survives_closing_either_sibling_display() {
    let (host, runtime) = mock_host();
    let (signal, analysis) = ready_analysis(&runtime);
    let user = host
        .show_spectrogram_with_id(
            DisplayId(42),
            analysis,
            Some(signal),
            &SpectrogramDisplayOptions::default(),
        )
        .unwrap();
    let backend = host
        .show_spectrogram(analysis, &SpectrogramDisplayOptions::default())
        .unwrap();

    host.close_display(user).unwrap();
    assert!(host.display(backend).is_ok());
    assert!(runtime.spectrogram_data(analysis).is_ok());
    assert_eq!(runtime.signal_ids(), vec![signal]);
    assert_eq!(runtime.analysis_ids(), vec![analysis]);

    host.close_display(backend).unwrap();
    assert!(runtime.analysis_ids().is_empty());
    assert_eq!(runtime.signal_ids(), vec![signal]);
    runtime.close(EntityId::Signal(signal)).unwrap();
    assert_eq!(live(&runtime), (0, 0, 0, 0));

    // Opposite order: the analysis-only sibling closes first.
    let (signal_b, analysis_b) = ready_analysis(&runtime);
    let user = host
        .show_spectrogram_with_id(
            DisplayId(43),
            analysis_b,
            Some(signal_b),
            &SpectrogramDisplayOptions::default(),
        )
        .unwrap();
    let backend = host
        .show_spectrogram(analysis_b, &SpectrogramDisplayOptions::default())
        .unwrap();
    host.close_display(backend).unwrap();
    assert!(host.display(user).is_ok());
    assert!(runtime.spectrogram_data(analysis_b).is_ok());
    assert_eq!(runtime.signal_ids(), vec![signal_b]);
    host.close_display(user).unwrap();
    assert_eq!(live(&runtime), (0, 0, 0, 0));
}

#[test]
fn shared_cut_survives_until_its_last_display_closes() {
    let (host, runtime) = mock_host();
    let (signal, analysis) = ready_analysis(&runtime);
    let cut = runtime
        .order_cut(analysis, &OrderCutRequest::default())
        .unwrap();
    let first_parent = host
        .show_spectrogram_with_signal(
            analysis,
            Some(signal),
            &SpectrogramDisplayOptions::default(),
        )
        .unwrap();
    let second_parent = host
        .show_spectrogram(analysis, &SpectrogramDisplayOptions::default())
        .unwrap();
    let first_child = host
        .show_order_cut(
            first_parent,
            cut,
            &CutDisplayOptions::order(OrderCutSettings::default()),
        )
        .unwrap();
    let second_child = host
        .show_order_cut(
            second_parent,
            cut,
            &CutDisplayOptions::order(OrderCutSettings::default()),
        )
        .unwrap();
    assert_eq!(live(&runtime), (1, 1, 1, 2));

    host.close_display(first_child).unwrap();
    assert!(host.display(second_child).is_ok());
    assert!(runtime.cut_data(cut).is_ok(), "the shared cut must survive");
    assert_eq!(runtime.cut_ids(), vec![cut]);

    host.close_display(second_parent).unwrap();
    assert!(runtime.cut_ids().is_empty());
    assert!(host.display(first_parent).is_ok());
    assert!(runtime.spectrogram_data(analysis).is_ok());
    assert_eq!(runtime.signal_ids(), vec![signal]);
    assert_eq!(live(&runtime), (1, 1, 0, 1));

    host.close_display(first_parent).unwrap();
    assert!(host.displays().unwrap().is_empty());
    assert_eq!(live(&runtime), (0, 0, 0, 0));
}

#[test]
fn wait_all_resolves_display_jobs_and_reports_busy() {
    let (host, runtime) = held_host();
    let signal = runtime
        .load_signal(LoadSignalRequest::new(vec![0.0; 16], 8_000.0))
        .unwrap();
    let analysis = runtime
        .analyze(signal, &AnalysisSettings::default())
        .unwrap();
    let spec = host
        .show_spectrogram_pending(
            analysis,
            Some(signal),
            &SpectrogramDisplayOptions::default(),
        )
        .unwrap();
    assert_eq!(
        host.display(spec).unwrap().name,
        "signal",
        "pending names fall back to analysis metadata"
    );

    let busy = host.wait_all(Some(Duration::from_millis(5))).unwrap_err();
    assert_eq!(busy.code(), "busy");

    let analysis_job = runtime.job_for(EntityId::Analysis(analysis)).unwrap();
    runtime.complete(analysis_job).unwrap();
    host.wait_all(None).unwrap();
    host.wait_all(Some(Duration::from_secs(1))).unwrap();

    let order = runtime
        .order_cut(analysis, &OrderCutRequest::default())
        .unwrap();
    host.show_order_cut_pending(
        spec,
        order,
        &CutDisplayOptions::order(OrderCutSettings::default()),
    )
    .unwrap();
    let busy = host.wait_all(Some(Duration::from_millis(5))).unwrap_err();
    assert_eq!(busy.code(), "busy");

    let cut_job = runtime.job_for(EntityId::Cut(order)).unwrap();
    runtime.complete(cut_job).unwrap();
    host.wait_all(None).unwrap();

    let (empty_host, _) = mock_host();
    empty_host.wait_all(Some(Duration::ZERO)).unwrap();
}

#[test]
fn show_plot_registers_a_standalone_display_with_path_and_layout() {
    let (host, _runtime) = held_host();
    let id = host
        .show_plot(
            &PlotDisplayOptions::new("/data/series.csv")
                .with_layout(PlotLayout::Pairs)
                .with_name("pairs"),
        )
        .unwrap();
    let info = host.display(id).unwrap();
    assert_eq!(info.kind, DisplayKind::Plot);
    assert_eq!(info.name, "pairs");
    assert!(info.parent.is_none());
    assert_eq!(info.state["path"], "/data/series.csv");
    assert_eq!(info.state["plotLayout"], "pairs");
    assert_eq!(info.state["sourceKind"], "backend");
    assert!(!info.ephemeral);
    assert!(host.registry().record(id).unwrap().analysis.is_none());
}

#[test]
fn shutdown_is_idempotent_clears_sinks_and_keeps_the_runtime_running() {
    let host = AppHost::new(AppHostConfig {
        workers: 1,
        ..test_config()
    });
    let captured = capture(&host);
    assert_eq!(host.registry().subscription_count(), 1);

    host.shutdown();
    host.shutdown();
    assert!(host.is_shutdown());
    assert_eq!(host.registry().subscription_count(), 0);
    assert_eq!(
        host.show_spectrogram(AnalysisId(1), &SpectrogramDisplayOptions::default())
            .unwrap_err()
            .code(),
        "closed"
    );
    assert_eq!(host.wait_all(None).unwrap_err().code(), "closed");
    assert!(captured.lock().unwrap().is_empty());

    // The composition root owns the runtime: app shutdown must not stop it or
    // suppress work submitted directly through the runtime handle.
    let runtime = host.runtime();
    let signal = runtime
        .load_signal(LoadSignalRequest::new(vec![1.0; 8192], 8_000.0))
        .unwrap();
    let analysis = runtime
        .analyze(signal, &AnalysisSettings::default())
        .unwrap();
    let job = runtime.job_for(EntityId::Analysis(analysis)).unwrap();
    runtime.wait(job, Some(Duration::from_secs(5))).unwrap();
    assert!(runtime.spectrogram_data(analysis).is_ok());
    assert!(
        captured.lock().unwrap().is_empty(),
        "cleared sinks receive no post-shutdown job events"
    );
}

#[test]
fn display_events_are_published_outside_the_registry_lock() {
    let (host, runtime) = mock_host();
    let (_, analysis) = ready_analysis(&runtime);
    let observed = Arc::new(StdMutex::new(Vec::new()));
    let inner = Arc::clone(&host);
    let target = Arc::clone(&observed);
    host.subscribe(Arc::new(move |event: AppEvent| {
        if matches!(
            event,
            AppEvent::DisplayOpened { .. } | AppEvent::DisplayClosed { .. }
        ) {
            // Re-entering the registry would deadlock if events were emitted
            // while its lock was held.
            target.lock().unwrap().push((
                inner.displays().unwrap().len(),
                inner.registry().records().len(),
            ));
        }
    }));

    let id = host
        .show_spectrogram(analysis, &SpectrogramDisplayOptions::default())
        .unwrap();
    host.close_display(id).unwrap();
    assert_eq!(*observed.lock().unwrap(), vec![(1, 1), (0, 0)]);
}

#[test]
fn update_and_abandon_are_app_level_commands() {
    let (host, runtime) = mock_host();
    let (_, analysis) = ready_analysis(&runtime);
    let id = host
        .show_spectrogram(analysis, &SpectrogramDisplayOptions::default())
        .unwrap();
    let events = capture(&host);

    let updated = host
        .update_display(
            id,
            json!({ "analysisId": analysis.get(), "zoom": 3.0 }),
            false,
        )
        .unwrap();
    assert!(!updated.visible);
    assert_eq!(updated.state["zoom"], 3.0);
    assert_eq!(host.display(id).unwrap(), updated);

    let removed = host.abandon_display(id).expect("display record");
    assert_eq!(removed.info.id, id);
    assert_eq!(host.display(id).unwrap_err().code(), "not_found");
    assert_eq!(events.lock().unwrap().len(), 1, "abandon_display is silent");
    assert!(matches!(
        &events.lock().unwrap()[0],
        AppEvent::DisplayUpdated { display } if display.id == id
    ));
    assert!(
        runtime.spectrogram_data(analysis).is_ok(),
        "abandon does not close runtime entities; the caller owns cleanup"
    );
    runtime.close(EntityId::Analysis(analysis)).unwrap();
}
