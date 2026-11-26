//! Unit tests for the `fft_tauri_adapter` crate and its command surface.
//!
//! Command cores are exercised with `fft_test::TestApp`/`TestHost`/`TestLease`
//! and a recording event emitter. Adapter lifecycle uses a real `AppHost`
//! around `MockRuntime`; no GUI, `AppHandle`, or window is required.

use super::*;

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};

use fft_app::{
    AppCapabilities, AppEvent, AppEventSink, AppMode, DisplayId, DisplayInfo, DisplayKind,
    DisplayOwner, FftApp, LeaseId, SCRIPTED_DISPLAY_ID_BASE,
};
use fft_app_host::{AppHost, AppHostConfig};
use fft_core::orders::{Cut, LineSpec};
use fft_core::session::SessionDocument;
use fft_core::settings::{
    AnalysisSettings, DisplaySettings, LineCutSettings, OrderCutSettings, Settings,
};
use fft_runtime::{
    AnalysisId, EntityId, FftRuntime, JobId, LoadSignalRequest, MockRuntime, SignalId,
};
use fft_test::{TestApp, TestHost};
use serde_json::{json, Value};
use tempfile::TempDir;

use crate::adapter::TauriAdapter;
#[cfg(not(feature = "python"))]
use crate::adapter::PYTHON_UNAVAILABLE;
#[cfg(feature = "python")]
use crate::adapter::{script_stream_name, ScriptRunner};
use crate::core::{CommandCore, DisplayHostOps};
use crate::events::{spawn_translator, EventEmitter, TransportSink};
use crate::paths::{export_cuts_csv, write_png, FilePaths};

#[derive(Default)]
struct RecordingEmitter {
    events: Mutex<Vec<(String, Value)>>,
}

impl RecordingEmitter {
    fn names(&self) -> Vec<String> {
        self.events
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .map(|(name, _)| name.clone())
            .collect()
    }

    fn find(&self, name: &str) -> Option<Value> {
        self.events
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .find(|(event, _)| event == name)
            .map(|(_, payload)| payload.clone())
    }

    fn find_last(&self, name: &str) -> Option<Value> {
        self.events
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .rev()
            .find(|(event, _)| event == name)
            .map(|(_, payload)| payload.clone())
    }

    fn count(&self, name: &str) -> usize {
        self.events
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .filter(|(event, _)| event == name)
            .count()
    }
}

impl EventEmitter for RecordingEmitter {
    fn emit(&self, name: &str, payload: Value) {
        self.events
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push((name.to_string(), payload));
    }
}

/// Thin host-specific operations adapter for `TestApp`; the app itself remains
/// the shared `fft_test` double.
struct TestOps(Arc<TestApp>);

impl DisplayHostOps for TestOps {
    fn record(&self, id: DisplayId) -> Option<fft_app::DisplayRecord> {
        self.0.registry().record(id)
    }

    fn records(&self) -> Vec<fft_app::DisplayRecord> {
        self.0.registry().records()
    }

    fn register_user_spectrogram_with_id(
        &self,
        id: DisplayId,
        analysis: AnalysisId,
        signal: Option<SignalId>,
        options: &fft_app::SpectrogramDisplayOptions,
    ) -> Result<DisplayId, fft_app::AppError> {
        self.0.registry().insert_spectrogram_with_id(
            id,
            analysis,
            signal,
            options,
            DisplayOwner::User,
            options.ephemeral,
        )
    }

    fn abandon_display(&self, id: DisplayId) -> Option<fft_app::DisplayRecord> {
        self.0.registry().remove_silent(id)
    }
}

struct Rig {
    host: TestHost,
    app: Arc<TestApp>,
    core: Arc<CommandCore>,
    recorder: Arc<RecordingEmitter>,
    dir: TempDir,
}

impl Rig {
    fn new() -> Self {
        Self::with_host(TestHost::new())
    }

    fn held() -> Self {
        Self::with_host(TestHost::held())
    }

    fn with_app(app: TestApp) -> Self {
        Self::with_host(TestHost::with_app(app))
    }

    fn with_host(host: TestHost) -> Self {
        let lease = host.lease();
        let app = host.app_arc();
        let dir = tempfile::tempdir().unwrap();
        let recorder = Arc::new(RecordingEmitter::default());
        let sink = Arc::new(TransportSink::inline(recorder.clone()));
        let core = Arc::new(CommandCore::new(
            lease,
            Arc::new(TestOps(Arc::clone(&app))),
            sink,
            FilePaths::new(dir.path()),
        ));
        core.subscribe();
        Self {
            host,
            app,
            core,
            recorder,
            dir,
        }
    }

    fn dir(&self) -> &Path {
        self.dir.path()
    }
}

#[test]
fn adapter_state_is_send_sync_for_the_tauri_manager() {
    fn assert_send_sync<T: Send + Sync + 'static>() {}
    assert_send_sync::<TauriAdapter>();
}

fn app_host() -> Arc<AppHost> {
    let runtime = Arc::new(MockRuntime::new());
    AppHost::from_runtime(
        runtime,
        AppHostConfig {
            name: "fft_audio-test".to_string(),
            version: "0.1.0".to_string(),
            mode: AppMode::Desktop,
            capabilities: AppCapabilities::for_build(),
            workers: 1,
            persisted_session: None,
        },
    )
}

fn seed_analysis(app: &TestApp) -> (SignalId, AnalysisId) {
    let runtime = app.runtime_handle();
    let signal = runtime
        .load_signal(LoadSignalRequest::new(vec![0.0; 32], 8_000.0))
        .unwrap();
    let analysis = runtime
        .analyze(signal, &AnalysisSettings::default())
        .unwrap();
    (signal, analysis)
}

fn open_demo(rig: &Rig, display_id: u64) -> crate::dto::OpenResult {
    rig.core
        .open_analysis(
            display_id,
            format!("demo-{display_id}.wav"),
            AnalysisSettings::default(),
        )
        .unwrap()
}

fn last_analysis(rig: &Rig, display_id: u64) -> AnalysisId {
    rig.core
        .get_display(display_id)
        .unwrap()
        .state
        .get("analysisId")
        .and_then(Value::as_u64)
        .map(AnalysisId)
        .expect("display state carries analysisId")
}

#[test]
fn command_and_event_names_match_the_existing_shell_surface() {
    assert_eq!(
        COMMAND_NAMES,
        &[
            "app_info",
            "app_ping",
            "spectrogram_open",
            "spectrogram_get",
            "display_list",
            "display_get",
            "display_close",
            "display_show_spectrogram",
            "spectrogram_export_image",
            "image_save_png",
            "recording_save",
            "order_cuts_compute",
            "line_cuts_compute",
            "cuts_export_csv",
            "plot_load",
            "analysis_cancel",
            "theme_get",
            "theme_list",
            "session_get",
            "session_save",
            "settings_get",
            "settings_save",
            "config_paths",
            "open_path",
            "theme_duplicate",
            "settings_set_spectrogram_defaults",
            "dialog_open_audio",
            "script_run",
            "script_stop",
            "script_stat",
            "script_read_file",
            "script_write_file",
            "script_list_examples",
            "script_create_example",
            "script_pick_file",
        ]
    );
    let unique: HashSet<_> = COMMAND_NAMES.iter().copied().collect();
    assert_eq!(unique.len(), COMMAND_NAMES.len(), "no duplicate commands");

    assert_eq!(
        APP_EVENT_NAMES,
        &[
            "analysis-progress",
            "analysis-done",
            "analysis-error",
            "display-open",
            "display-updated",
            "display-closed",
        ]
    );
    assert_eq!(SCRIPT_EVENT_NAMES, &["script-output", "script-exit"]);
    assert!(EVENT_NAMES.contains(&"notification"));
    for forbidden in ["http", "serve", "listen"] {
        assert!(
            !COMMAND_NAMES.iter().any(|name| name.contains(forbidden)),
            "no {forbidden} command is part of the GUI surface"
        );
    }
}

#[test]
fn app_info_reports_desktop_mode_capabilities_and_conditional_scripting() {
    let app = TestApp::new()
        .with_info(fft_app::AppInfo {
            name: "desktop-test".to_string(),
            version: "9.9.9".to_string(),
            mode: AppMode::Desktop,
        })
        .with_capabilities(AppCapabilities {
            displays: true,
            table_formats: vec![],
        });
    let rig = Rig::with_app(app);

    let info = rig.core.app_info().unwrap();
    assert_eq!(info.name, "desktop-test");
    assert_eq!(info.version, "9.9.9");
    assert_eq!(info.mode, AppMode::Desktop);
    assert!(info.capabilities.displays);
    assert_eq!(info.capabilities.scripting, scripting_capability());
    assert!(info.capabilities.table_formats.is_empty());

    let json = serde_json::to_value(&info).unwrap();
    assert_eq!(json["mode"], "desktop");
    assert_eq!(json["capabilities"]["displays"], true);
    assert!(json["capabilities"]["tableFormats"].is_array());
    if cfg!(feature = "python") {
        assert_eq!(json["capabilities"]["scripting"], "python");
    } else {
        assert_eq!(json["capabilities"]["scripting"], Value::Null);
    }
}

#[test]
fn app_ping_echoes_the_nonce_and_a_timestamp() {
    let ping = PingResult::now(Some("abc".into()));
    assert_eq!(ping.nonce, Some("abc".to_string()));
    assert!(ping.at > 0);

    let ping = PingResult::now(None);
    assert!(ping.nonce.is_none());
    assert_eq!(
        serde_json::to_value(&ping).unwrap(),
        json!({ "nonce": null, "at": ping.at })
    );
}

#[test]
fn commands_share_one_root_lease_and_never_mint() {
    let rig = Rig::new();
    assert_eq!(rig.host.created_lease_ids(), vec![LeaseId(1)]);

    for _ in 0..8 {
        rig.core.app_info().unwrap();
        rig.core.list_displays().unwrap();
        assert_eq!(
            rig.core.get_display(999).unwrap_err(),
            "not_found: display 999 not found"
        );
        rig.core.close_display(999).unwrap();
        assert!(rig.core.spectrogram_payload(999).is_err());
        rig.core.paths().get_settings().unwrap();
        assert!(!rig.core.cancel_analysis(999).cancelled);
    }

    assert_eq!(
        rig.host.created_lease_ids(),
        vec![LeaseId(1)],
        "commands borrow the root lease and never mint another"
    );
    rig.host.assert_active_leases(1);

    rig.core.begin_shutdown();
    rig.core.drop_lease();
    rig.host.assert_all_leases_dropped();
}

#[test]
fn adapter_borrows_the_root_gui_lease_without_shutting_down_the_host() {
    let host = app_host();
    let recorder = Arc::new(RecordingEmitter::default());
    let dir = tempfile::tempdir().unwrap();
    let adapter = TauriAdapter::for_tests(Arc::clone(&host), recorder, dir.path());

    assert_eq!(adapter.root_lease_id(), Some(LeaseId(1)));
    assert_eq!(adapter.app_info().mode, AppMode::Desktop);
    adapter.core().list_displays().unwrap();
    assert_eq!(host.registry().subscription_count(), 1);

    adapter.shutdown();
    assert!(adapter.is_shutdown());
    assert!(!host.is_shutdown());
    assert_eq!(adapter.root_lease_id(), None);
    // The root owns the host; its shutdown clears the adapter subscription.
    assert_eq!(host.registry().subscription_count(), 1);

    // Idempotent.
    adapter.shutdown();
    assert!(adapter.is_shutdown());

    host.shutdown();
    assert_eq!(host.registry().subscription_count(), 0);
}

#[test]
fn open_get_close_and_unknown_display_behavior() {
    let rig = Rig::new();

    let err = rig
        .core
        .open_analysis(0, "bad.wav".into(), AnalysisSettings::default())
        .unwrap_err();
    assert!(err.contains("must be between"), "{err}");
    let err = rig
        .core
        .open_analysis(
            SCRIPTED_DISPLAY_ID_BASE,
            "bad.wav".into(),
            AnalysisSettings::default(),
        )
        .unwrap_err();
    assert!(err.contains("must be between"), "{err}");
    let invalid = AnalysisSettings {
        frame: 0,
        ..AnalysisSettings::default()
    };
    let err = rig
        .core
        .open_analysis(7, "bad.wav".into(), invalid)
        .unwrap_err();
    assert!(err.contains("invalid analysis settings"), "{err}");
    assert!(rig.core.list_displays().unwrap().is_empty());
    assert!(rig.app.runtime_handle().signal_ids().is_empty());

    let opened = open_demo(&rig, 7);
    assert_eq!(opened.display_id, 7);
    assert_eq!(opened.analysis_id, 1);
    let analysis = AnalysisId(opened.analysis_id);

    let displays = rig.core.list_displays().unwrap();
    assert_eq!(displays.len(), 1);
    assert_eq!(displays[0].id, DisplayId(7));
    assert_eq!(displays[0].name, "demo-7");
    assert_eq!(displays[0].kind, DisplayKind::Spectrogram);

    let display = rig.core.get_display(7).unwrap();
    assert_eq!(display.id, DisplayId(7));
    assert_eq!(last_analysis(&rig, 7), analysis);

    let payload = rig.core.spectrogram_payload(7).unwrap();
    assert_eq!(payload.meta.sample_rate, 1.0);
    assert_eq!(payload.meta.frames, 0);
    assert_eq!(payload.meta.analysis_settings.frame, 2048);
    assert!(payload.data.is_empty());

    assert_eq!(
        rig.core.get_display(999).unwrap_err(),
        "not_found: display 999 not found"
    );
    assert!(rig
        .core
        .spectrogram_payload(999)
        .unwrap_err()
        .contains("not ready"));
    let err = rig
        .core
        .render_spectrogram(999)
        .err()
        .expect("unknown display has no spectrogram to render");
    assert!(err.contains("no spectrogram to export"));

    // Unknown closes are successful no-ops.
    rig.core.close_display(999).unwrap();

    assert!(rig.recorder.count("display-open") >= 1);
    assert_eq!(rig.recorder.find("analysis-done").unwrap()["displayId"], 7);

    rig.core.close_display(7).unwrap();
    assert!(rig.core.list_displays().unwrap().is_empty());
    assert!(rig.app.runtime_handle().analysis_info(analysis).is_none());
    assert!(rig.app.runtime_handle().signal_info(SignalId(1)).is_none());
    assert_eq!(
        rig.recorder.find("display-closed").unwrap(),
        json!({ "id": 7, "children": [] })
    );
}

#[test]
fn backend_spectrogram_show_allocates_reserved_ids_and_rejects_unknown() {
    let rig = Rig::new();
    let (_, analysis) = seed_analysis(&rig.app);

    let shown = rig.core.show_backend_spectrogram(analysis, true).unwrap();
    assert_eq!(shown.kind, DisplayKind::Spectrogram);
    assert!(shown.ephemeral);
    assert!(
        shown.id.get() >= SCRIPTED_DISPLAY_ID_BASE,
        "backend/script displays stay in the reserved range"
    );
    assert_eq!(rig.core.list_displays().unwrap().len(), 1);

    let before = rig.core.list_displays().unwrap();
    let err = rig
        .core
        .show_backend_spectrogram(AnalysisId(999), true)
        .unwrap_err();
    assert!(err.contains("not_found"), "{err}");
    assert_eq!(rig.core.list_displays().unwrap(), before);
}

#[test]
fn closing_one_of_two_displays_keeps_the_shared_analysis() {
    let rig = Rig::new();
    let (signal, analysis) = seed_analysis(&rig.app);
    let options = fft_app::SpectrogramDisplayOptions::default();
    let first = rig
        .app
        .registry()
        .insert_spectrogram(
            analysis,
            Some(signal),
            &options,
            DisplayOwner::Backend,
            false,
        )
        .unwrap();
    let second = rig
        .app
        .registry()
        .insert_spectrogram(
            analysis,
            Some(signal),
            &options,
            DisplayOwner::Backend,
            false,
        )
        .unwrap();
    assert_eq!(rig.core.list_displays().unwrap().len(), 2);

    rig.core.close_display(first.get()).unwrap();
    assert!(rig.core.get_display(second.get()).is_ok());
    assert!(rig.app.runtime_handle().analysis_info(analysis).is_some());
    assert!(rig.app.runtime_handle().signal_info(signal).is_some());

    rig.core.close_display(second.get()).unwrap();
    assert!(rig.core.list_displays().unwrap().is_empty());
    assert!(rig.app.runtime_handle().analysis_info(analysis).is_none());
    assert!(rig.app.runtime_handle().signal_info(signal).is_none());
}

#[test]
fn spectrogram_export_renders_full_resolution_png() {
    let rig = Rig::new();
    open_demo(&rig, 7);

    let image = rig.core.render_spectrogram(7).unwrap();
    assert_eq!(image.width, 1600);
    assert_eq!(image.height, 900);
    assert_eq!(image.pixels.len(), 1600 * 900);

    let path = rig.dir().join("spectrogram.png");
    write_png(&path, &image).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert!(bytes.len() > 100);
}

#[test]
fn cut_commands_return_metadata_and_reject_unready_parents() {
    let rig = Rig::new();
    open_demo(&rig, 7);

    assert_eq!(
        rig.core
            .order_cuts(999, OrderCutSettings::default(), None)
            .unwrap_err(),
        "the parent spectrogram is not ready yet"
    );
    assert_eq!(
        rig.core
            .line_cuts(999, LineCutSettings::default(), vec![])
            .unwrap_err(),
        "the parent spectrogram is not ready yet"
    );

    let order = rig
        .core
        .order_cuts(7, OrderCutSettings::default(), None)
        .unwrap();
    assert_eq!(order.cuts.len(), 3, "default orders 1, 2, 4");
    assert!(order
        .cuts
        .iter()
        .all(|cut| matches!(cut.kind, fft_core::orders::CutKind::Order(_))));
    let json = serde_json::to_value(&order).unwrap();
    assert!(json["cuts"][0]["kind"]["type"].is_string());
    assert!(json["durationS"].is_number());
    assert!(json["nyquist"].is_number());
    assert!(json["binHz"].is_number());
    assert!(json["hopS"].is_number());

    let line = LineSpec {
        t0: 0.0,
        f0: 50.0,
        t1: 0.3,
        f1: 150.0,
        log_f: false,
    };
    let lines = rig
        .core
        .line_cuts(7, LineCutSettings::default(), vec![line])
        .unwrap();
    assert_eq!(lines.cuts.len(), 1);
    assert!(matches!(
        lines.cuts[0].kind,
        fft_core::orders::CutKind::Line(_)
    ));

    assert!(
        rig.app.runtime_handle().cut_ids().is_empty(),
        "transient cuts are closed after computing payloads"
    );

    let csv = rig.dir().join("cuts.csv");
    export_cuts_csv(&csv, &order.cuts).unwrap();
    let text = std::fs::read_to_string(&csv).unwrap();
    assert_eq!(
        text.lines().next(),
        Some("kind,value,time_s,rpm,freq_hz,db")
    );
}

#[test]
fn analysis_cancel_reports_active_jobs_and_emits_transport_error() {
    let rig = Rig::held();
    open_demo(&rig, 7);
    let analysis = last_analysis(&rig, 7);
    let job = rig
        .app
        .runtime_handle()
        .job_for(EntityId::Analysis(analysis))
        .unwrap();

    let cancelled = rig.core.cancel_analysis(7);
    assert!(cancelled.cancelled);
    let error = rig.recorder.find("analysis-error").unwrap();
    assert_eq!(error["displayId"], 7);
    assert_eq!(error["message"], "analysis cancelled");
    assert!(!rig.core.cancel_analysis(7).cancelled);
    assert!(rig.app.runtime_handle().job_status(job).is_terminal());
}

#[test]
fn failed_analysis_cleanup_abandons_the_display() {
    let rig = Rig::new();
    rig.app.runtime_handle().fail_next_analysis("boom");
    open_demo(&rig, 9);
    let analysis = last_analysis(&rig, 9);
    let job = rig
        .app
        .runtime_handle()
        .job_for(EntityId::Analysis(analysis))
        .unwrap();

    let error = rig.recorder.find("analysis-error").unwrap();
    assert_eq!(error["displayId"], 9);
    assert_eq!(error["message"], "boom");

    rig.core.cleanup_failed_display(9, job);
    assert!(rig.core.list_displays().unwrap().is_empty());
    assert!(rig.app.runtime_handle().analysis_info(analysis).is_none());
    assert!(rig.app.runtime_handle().signal_info(SignalId(1)).is_none());
}

#[test]
fn event_translation_uses_existing_names_and_camel_case_payloads() {
    let rig = Rig::new();
    let (_, analysis) = seed_analysis(&rig.app);
    let display = rig.core.show_backend_spectrogram(analysis, false).unwrap();
    let display_id = display.id.get();

    let opened = rig.recorder.find("display-open").unwrap();
    assert_eq!(opened["display"]["id"], display_id);
    assert_eq!(opened["display"]["kind"], "spectrogram");

    let sink = rig.core.transport().clone();
    sink.register(JobId(77), display_id);
    sink.emit(AppEvent::JobProgress {
        job: JobId(77),
        frames_done: 10,
        frames_total: Some(40),
        eta_s: Some(1.5),
    });
    assert_eq!(
        rig.recorder.find("analysis-progress").unwrap(),
        json!({
            "displayId": display_id,
            "framesDone": 10,
            "framesTotal": 40,
            "etaSeconds": 1.5
        })
    );

    // Unmapped events are buffered until their job is registered.
    sink.emit(AppEvent::JobProgress {
        job: JobId(88),
        frames_done: 1,
        frames_total: Some(2),
        eta_s: None,
    });
    assert_eq!(rig.recorder.count("analysis-progress"), 1);
    sink.register(JobId(88), 42);
    assert_eq!(rig.recorder.count("analysis-progress"), 2);
    assert_eq!(
        rig.recorder.find_last("analysis-progress").unwrap()["displayId"],
        42
    );

    sink.register(JobId(77), display_id);
    sink.emit(AppEvent::JobDone { job: JobId(77) });
    assert_eq!(
        rig.recorder.find("analysis-done").unwrap(),
        json!({ "displayId": display_id })
    );

    sink.register(JobId(99), display_id);
    sink.emit(AppEvent::JobFailed {
        job: JobId(99),
        message: "bad".into(),
    });
    assert_eq!(
        rig.recorder.find("analysis-error").unwrap(),
        json!({ "displayId": display_id, "message": "bad" })
    );

    // Ignored jobs (loads/transient cuts) never reach the transport.
    sink.register(JobId(100), display_id);
    sink.ignore(JobId(100));
    let before = rig.recorder.names().len();
    sink.emit(AppEvent::JobProgress {
        job: JobId(100),
        frames_done: 3,
        frames_total: None,
        eta_s: None,
    });
    assert_eq!(rig.recorder.names().len(), before);

    let info = DisplayInfo {
        id: DisplayId(5),
        parent: None,
        kind: DisplayKind::OrderCuts,
        name: "orders".into(),
        state: json!({ "cutId": 3 }),
        visible: true,
        ephemeral: false,
    };
    sink.emit(AppEvent::DisplayUpdated {
        display: info.clone(),
    });
    assert_eq!(
        rig.recorder.find("display-updated").unwrap()["display"]["id"],
        5
    );
    sink.emit(AppEvent::DisplayClosed {
        id: DisplayId(5),
        children: vec![DisplayId(6)],
    });
    assert_eq!(
        rig.recorder.find("display-closed").unwrap(),
        json!({ "id": 5, "children": [6] })
    );
}

#[test]
fn channel_translator_flushes_queued_events_before_stopping() {
    let recorder = Arc::new(RecordingEmitter::default());
    let (sender, receiver) = std::sync::mpsc::channel();
    let sink = Arc::new(TransportSink::channel(sender));
    let translator = spawn_translator(recorder.clone(), receiver);

    sink.emit(AppEvent::DisplayClosed {
        id: DisplayId(7),
        children: vec![],
    });
    sink.shutdown();
    translator.join().unwrap();

    assert_eq!(
        recorder.find("display-closed").unwrap(),
        json!({ "id": 7, "children": [] })
    );
}

#[test]
fn local_settings_session_themes_and_script_files_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let paths = FilePaths::new(dir.path());

    let config = paths.config_paths().unwrap();
    assert!(Path::new(&config.config_dir).is_dir());
    assert!(Path::new(&config.themes_dir).is_dir());
    assert!(config.settings_path.ends_with("settings.toml"));
    assert!(config.session_path.ends_with("session.json"));

    assert_eq!(paths.get_settings().unwrap(), Settings::default());
    let mut settings = paths.get_settings().unwrap();
    settings.analysis.frame = 1024;
    settings.appearance.theme = "light".into();
    paths.save_settings(&settings).unwrap();
    assert_eq!(paths.get_settings().unwrap(), settings);

    paths
        .set_spectrogram_defaults(
            AnalysisSettings {
                frame: 512,
                ..AnalysisSettings::default()
            },
            DisplaySettings {
                range_db: 55.0,
                ..DisplaySettings::default()
            },
        )
        .unwrap();
    let settings = paths.get_settings().unwrap();
    assert_eq!(settings.analysis.frame, 512);
    assert_eq!(settings.display.range_db, 55.0);
    assert_eq!(settings.appearance.theme, "light", "other fields stay");

    assert!(paths.get_session().unwrap().is_none());
    let session = SessionDocument {
        next_id: 7,
        ..SessionDocument::default()
    };
    paths.save_session(&session).unwrap();
    assert_eq!(paths.get_session().unwrap(), Some(session));

    let dark = paths.get_theme(Some("dark")).unwrap();
    assert_eq!(dark.name, "Dark");
    let themes = paths.list_themes().unwrap();
    assert!(themes.iter().any(|theme| theme.id == "dark"));
    assert!(themes.iter().any(|theme| theme.id == "light"));

    let id = paths.duplicate_theme("dark", "My Theme".into()).unwrap();
    assert_eq!(id, "my-theme");
    assert!(paths
        .list_themes()
        .unwrap()
        .iter()
        .any(|theme| theme.id == "my-theme"));
    assert!(paths.duplicate_theme("dark", "My Theme".into()).is_err());

    let examples = paths.list_script_examples();
    assert!(!examples.is_empty(), "the desktop build embeds examples");
    let unique: HashSet<_> = examples.iter().map(|example| &example.filename).collect();
    assert_eq!(unique.len(), examples.len(), "flattened names stay unique");
    for example in &examples {
        assert!(example.filename.ends_with(".py"));
        assert!(!example.filename.contains('/'));
        assert!(!example.filename.contains('\\'));
        let created = paths.create_script_example(&example.filename).unwrap();
        assert!(created.path.ends_with(&example.filename));
        let source = crate::paths::read_script_file(&created.path).unwrap();
        assert!(
            source.contains("fft_audio"),
            "example source was not copied into the config directory"
        );
    }
    let created = paths.create_script_example(&examples[0].filename).unwrap();
    assert!(created.path.ends_with(&examples[0].filename));
    let stat = crate::paths::script_stat(&created.path);
    assert!(stat.exists);
    assert!(stat.mtime_ms.is_some());
    crate::paths::write_script_file(&created.path, "print('bye')\n").unwrap();
    assert_eq!(
        crate::paths::read_script_file(&created.path).unwrap(),
        "print('bye')\n"
    );
    assert!(paths.create_script_example("../escape.py").is_err());
    assert!(paths.create_script_example("missing.py").is_err());

    let missing = dir.path().join("missing.py");
    let stat = crate::paths::script_stat(missing.to_str().unwrap());
    assert!(!stat.exists);
    assert!(stat.mtime_ms.is_none());
}

#[test]
fn command_payloads_keep_camel_case_shapes() {
    let info = AppInfo {
        name: "fft_audio".into(),
        version: "1.2.3".into(),
        mode: AppMode::Desktop,
        capabilities: AppCapabilitiesInfo {
            displays: true,
            scripting: None,
            table_formats: vec![],
        },
    };
    assert_eq!(
        serde_json::to_value(&info).unwrap(),
        json!({
            "name": "fft_audio",
            "version": "1.2.3",
            "mode": "desktop",
            "capabilities": { "displays": true, "scripting": null, "tableFormats": [] }
        })
    );

    assert_eq!(
        serde_json::to_value(OpenResult {
            display_id: 7,
            analysis_id: 3,
        })
        .unwrap(),
        json!({ "displayId": 7, "analysisId": 3 })
    );
    assert_eq!(
        serde_json::to_value(CancelResult { cancelled: true }).unwrap(),
        json!({ "cancelled": true })
    );
    assert_eq!(
        serde_json::to_value(AnalysisProgressEvent {
            display_id: 7,
            frames_done: 10,
            frames_total: Some(40),
            eta_seconds: Some(1.5),
        })
        .unwrap(),
        json!({
            "displayId": 7,
            "framesDone": 10,
            "framesTotal": 40,
            "etaSeconds": 1.5
        })
    );
    assert_eq!(
        serde_json::to_value(AnalysisDoneEvent { display_id: 7 }).unwrap(),
        json!({ "displayId": 7 })
    );
    assert_eq!(
        serde_json::to_value(AnalysisErrorEvent {
            display_id: 7,
            message: "bad".into(),
        })
        .unwrap(),
        json!({ "displayId": 7, "message": "bad" })
    );
    assert_eq!(
        serde_json::to_value(ScriptExampleInfo {
            filename: "harmonics_chirp_example.py".into(),
        })
        .unwrap(),
        json!({ "filename": "harmonics_chirp_example.py" })
    );

    let display = DisplayInfo {
        id: DisplayId(7),
        parent: Some(DisplayId(1)),
        kind: DisplayKind::OrderCuts,
        name: "orders".into(),
        state: json!({ "cutId": 4 }),
        visible: false,
        ephemeral: true,
    };
    assert_eq!(
        serde_json::to_value(DisplayEventPayload {
            display: display.clone()
        })
        .unwrap(),
        json!({
            "display": {
                "id": 7,
                "parent": 1,
                "kind": "order_cuts",
                "name": "orders",
                "state": { "cutId": 4 },
                "visible": false,
                "ephemeral": true
            }
        })
    );
    assert_eq!(
        serde_json::to_value(DisplayClosedEvent {
            id: 7,
            children: vec![8, 9],
        })
        .unwrap(),
        json!({ "id": 7, "children": [8, 9] })
    );

    let payload = SpectrogramPayload {
        meta: SpectrogramMeta {
            frames: 1,
            bins: 2,
            bin_hz: 3.0,
            frame_s: 0.1,
            duration_s: 0.2,
            nyquist: 4.0,
            max_db: 0.0,
            min_db: -80.0,
            sample_rate: 8_000.0,
            analysis_settings: AnalysisSettings::default(),
            table: None,
        },
        data: "AA==".into(),
    };
    let json = serde_json::to_value(&payload).unwrap();
    assert!(json["meta"]["binHz"].is_number());
    assert!(json["meta"]["frameS"].is_number());
    assert!(json["meta"]["analysisSettings"]["frame"].is_number());
    assert_eq!(json["data"], "AA==");

    let cuts = CutsPayload {
        cuts: vec![Cut {
            kind: fft_core::orders::CutKind::Order(2.0),
            points: vec![],
        }],
        duration_s: 1.0,
        nyquist: 2.0,
        bin_hz: 3.0,
        hop_s: 0.5,
    };
    let json = serde_json::to_value(&cuts).unwrap();
    assert_eq!(json["durationS"], 1.0);
    assert_eq!(json["nyquist"], 2.0);
    assert_eq!(json["binHz"], 3.0);
    assert_eq!(json["hopS"], 0.5);
    assert!(json["cuts"][0]["kind"]["type"].is_string());

    assert_eq!(
        serde_json::to_value(ConfigPaths {
            config_dir: "/c".into(),
            settings_path: "/c/settings.toml".into(),
            session_path: "/c/session.json".into(),
            themes_dir: "/c/themes".into(),
        })
        .unwrap(),
        json!({
            "configDir": "/c",
            "settingsPath": "/c/settings.toml",
            "sessionPath": "/c/session.json",
            "themesDir": "/c/themes"
        })
    );
    assert_eq!(
        serde_json::to_value(ScriptOutputEvent {
            display_id: 7,
            stream: "system",
            line: "log".into(),
        })
        .unwrap(),
        json!({ "displayId": 7, "stream": "system", "line": "log" })
    );
    assert_eq!(
        serde_json::to_value(ScriptExitEvent {
            display_id: 7,
            code: Some(2),
            success: false,
        })
        .unwrap(),
        json!({ "displayId": 7, "code": 2, "success": false })
    );
    assert_eq!(
        serde_json::to_value(FileStat {
            exists: true,
            mtime_ms: Some(123),
        })
        .unwrap(),
        json!({ "exists": true, "mtimeMs": 123 })
    );
    assert_eq!(
        serde_json::to_value(OpenFileResult {
            path: "/tmp/x.wav".into(),
        })
        .unwrap(),
        json!({ "path": "/tmp/x.wav" })
    );
    assert_eq!(
        serde_json::to_value(ScriptPathResult {
            path: "/tmp/x.py".into(),
        })
        .unwrap(),
        json!({ "path": "/tmp/x.py" })
    );
}

#[test]
fn no_http_dependency_or_listener_is_part_of_gui_mode() {
    let manifest = include_str!("../Cargo.toml");
    for forbidden in [
        "fft_http_adapter",
        "tauri-plugin-http",
        "axum",
        "warp",
        "hyper",
        "httparse",
    ] {
        assert!(
            !manifest.contains(forbidden),
            "GUI adapter must not depend on {forbidden}"
        );
    }

    for source in [include_str!("commands.rs"), include_str!("adapter.rs")] {
        for forbidden in ["TcpListener", "TcpStream", "0.0.0.0", "http://"] {
            assert!(
                !source.contains(forbidden),
                "GUI adapter must not contain an HTTP listener surface: {forbidden}"
            );
        }
    }
}

#[cfg(not(feature = "python"))]
#[test]
fn scripts_require_the_python_feature() {
    let host = app_host();
    let recorder = Arc::new(RecordingEmitter::default());
    let dir = tempfile::tempdir().unwrap();
    let adapter = TauriAdapter::for_tests(Arc::clone(&host), recorder.clone(), dir.path());

    assert_eq!(
        adapter
            .script_run(7, "demo.py".into(), recorder)
            .unwrap_err(),
        PYTHON_UNAVAILABLE
    );
    assert_eq!(adapter.script_stop(7).unwrap_err(), PYTHON_UNAVAILABLE);
    assert!(adapter.root_lease_id().is_some());
    assert_eq!(scripting_capability(), None);
    adapter.shutdown();
    host.shutdown();
}

#[cfg(feature = "python")]
#[test]
fn script_run_uses_the_root_minted_scripting_lease_and_emits_events() {
    use fft_script::testing::MockScript;

    let host = app_host();
    let recorder = Arc::new(RecordingEmitter::default());
    let dir = tempfile::tempdir().unwrap();
    let script = Arc::new(MockScript::new());
    let engine: Arc<dyn fft_script::FftScript> = script.clone();
    let adapter = TauriAdapter::for_tests_with_scripting(
        Arc::clone(&host),
        recorder.clone(),
        dir.path(),
        engine,
    );

    adapter
        .script_run(7, "demo.py".into(), recorder.clone())
        .unwrap();
    assert_eq!(
        script.last_record().unwrap().lease_id,
        LeaseId(2),
        "the run receives the root-minted scripting lease"
    );
    assert!(script.active_runs().is_empty(), "the run exits");
    assert_eq!(script_stream_name(fft_script::ScriptStream::Log), "system");
    assert!(recorder.count("script-output") >= 1);
    assert_eq!(recorder.find("script-exit").unwrap()["displayId"], 7);
    assert_eq!(recorder.find("script-exit").unwrap()["success"], true);

    adapter.script_stop(7).unwrap();
    adapter.shutdown();
    assert!(!host.is_shutdown());
    host.shutdown();
}

#[cfg(feature = "python")]
#[test]
fn script_engine_receives_one_lease_and_drops_it_after_the_run() {
    use fft_script::testing::MockScript;

    let host = TestHost::new();
    let root = host.lease();
    let recorder = Arc::new(RecordingEmitter::default());
    let runner = ScriptRunner::with_engine(Arc::new(MockScript::new()));
    let sink = Arc::new(crate::adapter::RunSink::new(
        Arc::downgrade(&runner),
        recorder.clone(),
        7,
    ));

    runner
        .start(host.lease(), 7, "demo.py".into(), sink)
        .unwrap();
    assert_eq!(host.created_lease_ids(), vec![LeaseId(1), LeaseId(2)]);
    assert_eq!(
        host.active_lease_ids(),
        vec![LeaseId(1)],
        "the per-run lease is dropped when the script exits"
    );
    assert_eq!(recorder.find("script-exit").unwrap()["displayId"], 7);

    runner.shutdown();
    drop(root);
    host.assert_all_leases_dropped();
}

#[cfg(feature = "python")]
struct PendingScript {
    next: std::sync::atomic::AtomicU64,
    cancelled: Mutex<Vec<fft_runtime::ScriptRunId>>,
}

#[cfg(feature = "python")]
impl fft_script::FftScript for PendingScript {
    fn run(
        &self,
        _lease: Arc<dyn fft_app::AppLease>,
        _source: fft_script::ScriptSource,
        _events: Arc<dyn fft_script::ScriptEventSink>,
    ) -> Result<fft_runtime::ScriptRunId, fft_script::ScriptError> {
        Ok(fft_runtime::ScriptRunId(
            self.next.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
        ))
    }

    fn cancel(&self, run: fft_runtime::ScriptRunId) {
        self.cancelled
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(run);
    }
}

#[cfg(feature = "python")]
#[test]
fn shutdown_cancels_active_script_runs() {
    let host = TestHost::new();
    let root = host.lease();
    let recorder = Arc::new(RecordingEmitter::default());
    let engine = Arc::new(PendingScript {
        next: std::sync::atomic::AtomicU64::new(1),
        cancelled: Mutex::new(Vec::new()),
    });
    let runner = ScriptRunner::with_engine(Arc::clone(&engine) as Arc<dyn fft_script::FftScript>);
    let sink = Arc::new(crate::adapter::RunSink::new(
        Arc::downgrade(&runner),
        recorder,
        7,
    ));

    runner
        .start(host.lease(), 7, "demo.py".into(), sink)
        .unwrap();
    runner.shutdown();
    let cancelled = engine
        .cancelled
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    assert_eq!(cancelled, vec![fft_runtime::ScriptRunId(1)]);
    drop(root);
    host.assert_all_leases_dropped();
}

#[cfg(feature = "python")]
#[test]
fn script_streams_keep_the_transport_names() {
    assert_eq!(
        script_stream_name(fft_script::ScriptStream::Stdout),
        "stdout"
    );
    assert_eq!(
        script_stream_name(fft_script::ScriptStream::Stderr),
        "stderr"
    );
    assert_eq!(script_stream_name(fft_script::ScriptStream::Log), "system");
}

#[cfg(feature = "python")]
#[test]
fn python_capability_is_advertised_when_the_feature_is_enabled() {
    assert_eq!(scripting_capability(), Some("python"));
    let info = AppCapabilitiesInfo::from_capabilities(AppCapabilities {
        displays: true,
        table_formats: vec![],
    });
    assert_eq!(info.scripting, Some("python"));
}

#[cfg(not(feature = "python"))]
#[test]
fn scripting_ui_is_not_advertised_without_the_feature() {
    assert_eq!(scripting_capability(), None);
    let info = AppCapabilitiesInfo::from_capabilities(AppCapabilities {
        displays: true,
        table_formats: vec![],
    });
    assert_eq!(info.scripting, None);
}

#[test]
fn recordings_are_written_into_the_config_folder_with_safe_names() {
    let dir = tempfile::tempdir().unwrap();
    let paths = FilePaths::new(dir.path());
    let recording = paths.save_recording("../take-01.wav", &[1, 2, 3]).unwrap();
    assert_eq!(
        recording.parent(),
        Some(dir.path().join("recordings").as_path())
    );
    assert_eq!(recording.file_name().unwrap(), "take-01.wav");
    assert_eq!(std::fs::read(&recording).unwrap(), vec![1, 2, 3]);

    // A second take with the same name must not overwrite the first.
    let second = paths.save_recording("take-01.wav", &[4]).unwrap();
    assert_ne!(second, recording);
    assert_eq!(std::fs::read(&second).unwrap(), vec![4]);

    assert!(paths.save_recording("empty.wav", &[]).is_err());
}
