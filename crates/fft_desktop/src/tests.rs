//! Tests for `fft_desktop`.
//!
//! All tests for this crate belong in this file. Root-owner tests use a real
//! `fft_app_host::AppHost` over `fft_runtime::MockRuntime` and an internal
//! composition seam that records the leases handed to adapters. No second
//! `FftApp` mock is introduced.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use fft_app::{AppCapabilities, AppLease, AppMode, LeaseId};
use fft_app_host::{AppHost, AppHostConfig};
use fft_runtime::MockRuntime;

use super::{
    parse_args, run, validate_config, CliConfig, CliError, CliErrorKind, CliExit, DesktopApp,
};
use crate::composition::{ScriptComposition, ServeComposition, ServerSession};
use crate::DEFAULT_BIND;

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

// ---------------------------------------------------------------------------
// Parser matrix
// ---------------------------------------------------------------------------

#[test]
fn default_config_is_loopback_and_inert() {
    let config = CliConfig::default();
    assert!(!config.serve);
    assert!(config.python_scripts.is_empty());
    assert!(config.python_delay.is_zero());
    assert!(!config.python_wait);
    assert!(!config.fresh);
    assert!(!config.headless);
    assert!(!config.config_dir_explicit);
    assert_eq!(config.workers, fft_runtime::default_worker_count());
    assert!(config.workers >= 1);
    assert_eq!(config.bind, DEFAULT_BIND);
    assert!(config.bind.ip().is_loopback());
    assert_eq!(config.bind.port(), 8765);
    assert_eq!(config.dist, PathBuf::from("web/dist"));
    assert!(!config.config_dir.as_os_str().is_empty());
    assert!(config.inert());
    assert!(!config.script_only());
}

#[test]
fn parse_headless_flag() {
    let config = parse_args(args(&["--headless"])).unwrap();
    assert!(config.headless);
    assert!(!config.fresh);
}

#[test]
fn parse_fresh_without_config_dir_uses_default_path() {
    let config = parse_args(args(&["--fresh"])).unwrap();
    assert!(config.fresh);
    assert!(!config.config_dir_explicit);
    assert!(!config.config_dir.as_os_str().is_empty());
}

#[test]
fn parse_fresh_with_config_dir_is_explicit() {
    let config = parse_args(args(&["--config-dir", "/tmp/fft-desktop", "--fresh"])).unwrap();
    assert!(config.fresh);
    assert!(config.config_dir_explicit);
    assert_eq!(config.config_dir, PathBuf::from("/tmp/fft-desktop"));
}

#[test]
fn fresh_run_clears_only_app_config_files() {
    let dir = tempfile::tempdir().unwrap();
    let config_dir = dir.path().join("config");
    std::fs::create_dir_all(config_dir.join("themes")).unwrap();
    std::fs::create_dir_all(config_dir.join("headless-uploads")).unwrap();
    std::fs::write(config_dir.join("settings.toml"), "x").unwrap();
    std::fs::write(config_dir.join("session.json"), "x").unwrap();
    std::fs::write(config_dir.join("themes/custom.toml"), "x").unwrap();
    std::fs::write(config_dir.join("keep.txt"), "keep").unwrap();
    std::fs::write(config_dir.join("headless-uploads/file.bin"), "keep").unwrap();

    let mut config = CliConfig::default();
    config.workers = 1;
    config.config_dir = config_dir.clone();
    config.fresh = true;

    assert_eq!(run(config).unwrap(), CliExit::Inert);
    assert!(config_dir.is_dir(), "the config directory must survive");
    assert!(!config_dir.join("settings.toml").exists());
    assert!(!config_dir.join("session.json").exists());
    assert!(!config_dir.join("themes/custom.toml").exists());
    assert!(config_dir.join("themes").is_dir(), "themes/ must survive");
    assert!(config_dir.join("keep.txt").exists());
    assert!(config_dir.join("headless-uploads/file.bin").exists());
}

#[test]
fn fresh_refuses_a_file_target() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("settings.toml");
    std::fs::write(&file, "x").unwrap();

    let mut config = CliConfig::default();
    config.workers = 1;
    config.config_dir = file.clone();
    config.fresh = true;

    let error = run(config).unwrap_err();
    assert_eq!(error.kind(), CliErrorKind::Usage);
    assert!(error.message().contains("directory"));
    assert!(file.exists());
}

#[test]
fn fresh_missing_dir_is_a_noop() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing");

    let mut config = CliConfig::default();
    config.workers = 1;
    config.config_dir = missing.clone();
    config.fresh = true;

    assert_eq!(run(config).unwrap(), CliExit::Inert);
    assert!(!missing.exists());
}

#[cfg(unix)]
#[test]
fn fresh_refuses_a_symlink_config_dir() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target");
    std::fs::create_dir(&target).unwrap();
    let link = dir.path().join("link");
    symlink(&target, &link).unwrap();

    let mut config = CliConfig::default();
    config.workers = 1;
    config.config_dir = link.clone();
    config.fresh = true;

    let error = run(config).unwrap_err();
    assert_eq!(error.kind(), CliErrorKind::Usage);
    assert!(error.message().contains("symlink"));
    assert!(target.exists());
}

#[cfg(unix)]
#[test]
fn fresh_does_not_follow_file_or_theme_symlinks() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let config_dir = dir.path().join("config");
    std::fs::create_dir_all(config_dir.join("themes")).unwrap();
    let settings_target = dir.path().join("outside-settings.toml");
    std::fs::write(&settings_target, "keep").unwrap();
    symlink(&settings_target, config_dir.join("settings.toml")).unwrap();
    let theme_target = dir.path().join("outside-theme.toml");
    std::fs::write(&theme_target, "keep").unwrap();
    symlink(&theme_target, config_dir.join("themes/link.toml")).unwrap();
    std::fs::write(config_dir.join("themes/regular.toml"), "x").unwrap();

    let mut config = CliConfig::default();
    config.workers = 1;
    config.config_dir = config_dir.clone();
    config.fresh = true;

    assert_eq!(run(config).unwrap(), CliExit::Inert);
    assert!(settings_target.exists());
    assert!(theme_target.exists());
    assert!(config_dir.join("settings.toml").exists());
    assert!(config_dir.join("themes/link.toml").exists());
    assert!(!config_dir.join("themes/regular.toml").exists());
}

#[test]
fn gui_is_selected_only_when_no_cli_work_is_requested() {
    let mut config = CliConfig::default();
    let gui_feature = cfg!(any(feature = "tauri", feature = "tauri_python"));
    assert_eq!(config.wants_gui(), gui_feature);

    config.headless = true;
    assert!(!config.wants_gui());
    config.headless = false;
    config.serve = true;
    assert!(!config.wants_gui());
    config.serve = false;
    config.python_scripts.push(PathBuf::from("a.py"));
    assert!(!config.wants_gui());
}

#[test]
fn script_delay_waits_before_running_scripts() {
    let mut config = CliConfig::default();
    config.python_delay = Duration::from_millis(30);

    let started = std::time::Instant::now();
    crate::wait_before_scripts(&config).unwrap();
    assert!(started.elapsed() >= Duration::from_millis(25));
}

#[test]
fn parse_common_workers_and_config_dir() {
    let config = parse_args(args(&[
        "--workers",
        "3",
        "--config-dir",
        "/tmp/fft-cli-config",
    ]))
    .unwrap();
    assert_eq!(config.workers, 3);
    assert_eq!(config.config_dir, PathBuf::from("/tmp/fft-cli-config"));
    assert!(!config.serve);
    assert!(config.python_scripts.is_empty());
    assert!(config.inert());

    let config = parse_args(args(&["--workers=2"])).unwrap();
    assert_eq!(config.workers, 2);
}

#[test]
fn parse_rejects_unknown_arguments_including_batch() {
    let error = parse_args(args(&["--batch"])).unwrap_err();
    assert_eq!(error.code(), "usage");
    assert!(error.message().contains("unexpected argument"));
    assert!(error.message().contains("--batch"));

    let error = parse_args(args(&["positional.py"])).unwrap_err();
    assert_eq!(error.code(), "usage");
    assert!(error.message().contains("unexpected argument"));
}

#[test]
fn parse_rejects_missing_common_values() {
    for flags in [vec!["--workers"], vec!["--config-dir"]] {
        let error = parse_args(args(&flags)).unwrap_err();
        assert_eq!(error.code(), "usage", "flags: {flags:?}");
        assert!(
            error.message().contains(flags[0]),
            "flags: {flags:?}, message: {}",
            error.message()
        );
        assert!(
            error.message().contains("value is required"),
            "flags: {flags:?}, message: {}",
            error.message()
        );
    }
}

#[test]
fn parse_rejects_invalid_workers() {
    let error = parse_args(args(&["--workers", "0"])).unwrap_err();
    assert_eq!(error.code(), "usage");
    assert!(error.message().contains("--workers"));

    let error = parse_args(args(&["--workers", "many"])).unwrap_err();
    assert_eq!(error.code(), "usage");
    assert!(error.message().contains("--workers"));
}

#[test]
fn parse_help_is_generated_and_exit_zero() {
    let error = parse_args(args(&["--help"])).unwrap_err();
    assert!(error.is_help());
    assert!(error.is_rendered());
    assert_eq!(error.code(), "help");
    assert_eq!(error.exit_code(), 0);
    assert!(error.message().contains("Usage:"));
    assert!(error.message().contains("--workers"));
    assert!(!error.message().contains("--batch"));

    #[cfg(feature = "http")]
    assert!(error.message().contains("--serve"));
    #[cfg(not(feature = "http"))]
    assert!(!error.message().contains("--serve"));

    #[cfg(feature = "python")]
    assert!(error.message().contains("--python-script"));
    #[cfg(not(feature = "python"))]
    assert!(!error.message().contains("--python-script"));
}

#[test]
fn parse_version_is_exit_zero() {
    let error = parse_args(args(&["--version"])).unwrap_err();
    assert!(error.is_help());
    assert!(error.is_rendered());
    assert_eq!(error.exit_code(), 0);
    assert!(error.message().contains(env!("CARGO_PKG_VERSION")));
}

#[cfg(not(feature = "http"))]
#[test]
fn http_flags_are_absent_without_http() {
    for flags in [
        vec!["--serve"],
        vec!["--bind", "127.0.0.1"],
        vec!["--port", "0"],
        vec!["--dist", "web/dist"],
    ] {
        let error = parse_args(args(&flags)).unwrap_err();
        assert_eq!(error.code(), "usage", "flags: {flags:?}");
        assert!(
            error.message().contains("unexpected argument"),
            "flags: {flags:?}, message: {}",
            error.message()
        );
    }
}

#[cfg(not(feature = "python"))]
#[test]
fn python_flags_are_absent_without_python() {
    for flags in [
        vec!["--python-script", "a.py"],
        vec!["--exit-after-script"],
        vec!["--python-delay", "1"],
        vec!["--python-wait"],
    ] {
        let error = parse_args(args(&flags)).unwrap_err();
        assert_eq!(error.code(), "usage", "flags: {flags:?}");
        assert!(
            error.message().contains("unexpected argument"),
            "flags: {flags:?}, message: {}",
            error.message()
        );
    }
}

#[cfg(feature = "http")]
#[test]
fn parse_serve_bind_port_and_dist() {
    let config = parse_args(args(&[
        "--serve",
        "--bind",
        "127.0.0.1:9000",
        "--dist=web/custom",
    ]))
    .unwrap();
    assert!(config.serve);
    assert_eq!(config.bind, "127.0.0.1:9000".parse().unwrap());
    assert_eq!(config.dist, PathBuf::from("web/custom"));
    assert!(!config.inert());
    assert!(!config.script_only());
}

#[cfg(feature = "http")]
#[test]
fn parse_http_bind_port_resolution() {
    let config = parse_args(args(&["--serve", "--bind", "127.0.0.1", "--port", "0"])).unwrap();
    assert_eq!(config.bind, "127.0.0.1:0".parse().unwrap());

    let config = parse_args(args(&["--serve", "--bind=::1"])).unwrap();
    assert!(config.bind.ip().is_loopback());
    assert_eq!(config.bind.port(), 8765);

    let config = parse_args(args(&[
        "--serve",
        "--bind",
        "127.0.0.1:9000",
        "--port",
        "0",
    ]))
    .unwrap();
    assert_eq!(config.bind, "127.0.0.1:0".parse().unwrap());
}

#[cfg(feature = "http")]
#[test]
fn non_loopback_bind_is_allowed() {
    let config = parse_args(args(&["--serve", "--bind", "0.0.0.0", "--port", "9000"])).unwrap();
    assert_eq!(config.bind, "0.0.0.0:9000".parse().unwrap());
    validate_config(&config).expect("the operator chooses the bind address");

    let config = parse_args(args(&["--serve", "--bind", "192.168.1.20:8765"])).unwrap();
    assert_eq!(config.bind, "192.168.1.20:8765".parse().unwrap());
    validate_config(&config).expect("the operator chooses the bind address");
}

#[cfg(feature = "http")]
#[test]
fn http_setup_flags_require_serve() {
    for flags in [
        vec!["--bind", "127.0.0.1"],
        vec!["--port", "0"],
        vec!["--dist", "web/dist"],
    ] {
        let error = parse_args(args(&flags)).unwrap_err();
        assert_eq!(error.code(), "usage", "flags: {flags:?}");
        assert!(
            error.message().contains("--serve"),
            "flags: {flags:?}, message: {}",
            error.message()
        );
    }
}

#[cfg(feature = "http")]
#[test]
fn parse_rejects_invalid_http_values() {
    for flags in [
        vec!["--serve", "--bind"],
        vec!["--serve", "--port"],
        vec!["--serve", "--dist"],
    ] {
        let error = parse_args(args(&flags)).unwrap_err();
        assert_eq!(error.code(), "usage", "flags: {flags:?}");
        assert!(
            error.message().contains(flags[1]),
            "flags: {flags:?}, message: {}",
            error.message()
        );
    }

    let error = parse_args(args(&["--serve=1"])).unwrap_err();
    assert_eq!(error.code(), "usage");
    assert!(error.message().contains("--serve"));

    let error = parse_args(args(&["--serve", "--bind", "not-an-address"])).unwrap_err();
    assert_eq!(error.code(), "usage");
    assert!(error.message().contains("--bind"));

    let error = parse_args(args(&["--serve", "--port", "70000"])).unwrap_err();
    assert_eq!(error.code(), "usage");
    assert!(error.message().contains("70000"));
}

#[cfg(feature = "python")]
#[test]
fn parse_python_scripts_and_exit_after() {
    let config = parse_args(args(&[
        "--python-script",
        "a.py",
        "--python-script",
        "b.py",
        "--exit-after-script",
    ]))
    .unwrap();
    assert!(!config.serve);
    assert!(!config.inert());
    assert!(config.script_only());
    assert_eq!(
        config.python_scripts,
        vec![PathBuf::from("a.py"), PathBuf::from("b.py")]
    );
    assert!(config.exit_after_script);
}

#[cfg(feature = "python")]
#[test]
fn parse_python_wait_flags() {
    let config = parse_args(args(&[
        "--python-script",
        "a.py",
        "--python-delay",
        "1.5",
        "--python-wait",
    ]))
    .unwrap();
    assert_eq!(config.python_delay, Duration::from_millis(1500));
    assert!(config.python_wait);
}

#[cfg(feature = "python")]
#[test]
fn python_wait_flags_require_a_script() {
    for flags in [vec!["--python-delay", "1"], vec!["--python-wait"]] {
        let error = parse_args(args(&flags)).unwrap_err();
        assert_eq!(error.code(), "usage", "flags: {flags:?}");
        assert!(
            error.message().contains("--python-script"),
            "flags: {flags:?}, message: {}",
            error.message()
        );
    }
}

#[cfg(feature = "python")]
#[test]
fn parse_rejects_invalid_python_delays() {
    for delay in ["abc", "nan", "inf", "-1"] {
        let error = parse_args(args(&[
            "--python-script",
            "a.py",
            &format!("--python-delay={delay}"),
        ]))
        .unwrap_err();
        assert_eq!(error.code(), "usage", "delay: {delay}");
        assert!(
            error.message().contains("--python-delay"),
            "delay: {delay}, message: {}",
            error.message()
        );
    }
}

#[cfg(feature = "python")]
#[test]
fn parse_exit_after_script_requires_a_script() {
    let error = parse_args(args(&["--exit-after-script"])).unwrap_err();
    assert_eq!(error.kind(), CliErrorKind::Usage);
    assert_eq!(error.code(), "usage");
    assert_eq!(error.exit_code(), 2);
    assert!(error.message().contains("--exit-after-script"));
    assert!(error.message().contains("--python-script"));
}

#[cfg(feature = "python")]
#[test]
fn parse_rejects_missing_python_values() {
    let error = parse_args(args(&["--python-script"])).unwrap_err();
    assert_eq!(error.code(), "usage");
    assert!(error.message().contains("--python-script"));

    let error = parse_args(args(&["--python-script="])).unwrap_err();
    assert_eq!(error.code(), "usage");
    assert!(error.message().contains("--python-script"));
}

#[cfg(all(feature = "http", feature = "python"))]
#[test]
fn parse_serve_with_repeatable_scripts() {
    let config = parse_args(args(&[
        "--serve",
        "--python-script",
        "a.py",
        "--python-script",
        "b.py",
        "--exit-after-script",
        "--workers",
        "3",
    ]))
    .unwrap();

    assert!(config.serve);
    assert_eq!(
        config.python_scripts,
        vec![PathBuf::from("a.py"), PathBuf::from("b.py")]
    );
    assert!(config.exit_after_script);
    assert_eq!(config.workers, 3);
    assert!(!config.inert());
    assert!(!config.script_only());
}

// ---------------------------------------------------------------------------
// Feature-off errors and validation
// ---------------------------------------------------------------------------

#[test]
fn validate_rejects_bad_programmatic_configs() {
    let mut config = CliConfig::default();
    config.exit_after_script = true;
    let error = validate_config(&config).unwrap_err();
    assert_eq!(error.code(), "usage");
    assert!(error.message().contains("--exit-after-script"));

    let mut config = CliConfig::default();
    config.workers = 0;
    let error = validate_config(&config).unwrap_err();
    assert_eq!(error.code(), "usage");
    assert!(error.message().contains("--workers"));

    let mut config = CliConfig::default();
    config.python_scripts.push(PathBuf::new());
    let error = validate_config(&config).unwrap_err();
    assert_eq!(error.code(), "usage");
    assert!(error.message().contains("--python-script"));
}

#[cfg(not(feature = "http"))]
#[test]
fn serve_without_http_feature_is_a_rebuild_error() {
    let mut config = CliConfig::default();
    config.serve = true;
    let error = validate_config(&config).unwrap_err();
    assert_eq!(error.kind(), CliErrorKind::UnsupportedFeature);
    assert_eq!(error.code(), "unsupported_feature");
    assert_eq!(error.feature(), Some("http"));
    assert_eq!(error.exit_code(), 2);
    assert!(error.message().contains("--features http"));
}

#[cfg(not(feature = "http"))]
#[test]
fn default_run_without_http_feature_is_inert() {
    let mut config = CliConfig::default();
    config.workers = 1;
    assert_eq!(run(config).unwrap(), CliExit::Inert);
}

#[cfg(not(feature = "python"))]
#[test]
fn python_script_without_python_feature_is_a_rebuild_error() {
    let mut config = CliConfig::default();
    config.python_scripts.push(PathBuf::from("a.py"));
    let error = validate_config(&config).unwrap_err();
    assert_eq!(error.kind(), CliErrorKind::UnsupportedFeature);
    assert_eq!(error.code(), "unsupported_feature");
    assert_eq!(error.feature(), Some("python"));
    assert_eq!(error.exit_code(), 2);
    assert!(error.message().contains("--features python"));

    let error = run(config).unwrap_err();
    assert_eq!(error.feature(), Some("python"));
}

#[cfg(all(feature = "http", not(feature = "python")))]
#[test]
fn serve_plus_script_without_python_feature_reports_python() {
    let mut config = CliConfig::default();
    config.serve = true;
    config.python_scripts.push(PathBuf::from("a.py"));
    let error = validate_config(&config).unwrap_err();
    assert_eq!(error.kind(), CliErrorKind::UnsupportedFeature);
    assert_eq!(error.feature(), Some("python"));
}

#[cfg(feature = "http")]
#[test]
fn http_composition_starts_and_stops_with_one_server_lease() {
    let host = test_host();
    let lease = host.lease();
    let lease_ref = Arc::downgrade(&lease);
    let config_dir = tempfile::tempdir().unwrap();

    let mut config = CliConfig::default();
    config.workers = 1;
    config.serve = true;
    config.bind = "127.0.0.1:0".parse().unwrap();
    config.config_dir = config_dir.path().to_path_buf();

    let mut session = crate::http::HttpComposition
        .start(lease, &config)
        .expect("HTTP composition should start");
    assert!(
        lease_ref.strong_count() > 0,
        "the server owns the lease while it is running"
    );

    session.stop().expect("HTTP composition should stop");
    assert_eq!(
        lease_ref.strong_count(),
        0,
        "the server-lifetime lease is released on stop"
    );

    fft_app::FftApp::shutdown(host.as_ref());
    assert!(host.is_shutdown());
}

#[cfg(feature = "python")]
#[test]
fn python_composition_waits_for_mock_exit() {
    let host = test_host();
    let lease = host.lease();
    let lease_id = lease.lease_id();
    let engine = Arc::new(fft_script::MockScript::new());
    let engine_ref: Arc<dyn fft_script::FftScript> = engine.clone();
    let composition = crate::scripting::CliScriptComposition::with_engine(engine_ref);

    composition.run(lease, Path::new("ok.py")).unwrap();

    let record = engine.last_record().expect("mock script record");
    assert_eq!(record.lease_id, lease_id);
    assert!(record.success);

    fft_app::FftApp::shutdown(host.as_ref());
}

#[cfg(feature = "python")]
#[test]
fn python_composition_maps_a_failed_exit() {
    struct FailingScript;

    impl fft_script::FftScript for FailingScript {
        fn run(
            &self,
            _lease: Arc<dyn AppLease>,
            _source: fft_script::ScriptSource,
            events: Arc<dyn fft_script::ScriptEventSink>,
        ) -> Result<fft_script::ScriptRunId, fft_script::ScriptError> {
            let run = fft_script::ScriptRunId(7);
            events.output(run, fft_script::ScriptStream::Stderr, "boom".to_string());
            events.exited(run, false, Some(2));
            Ok(run)
        }

        fn cancel(&self, _run: fft_script::ScriptRunId) {}
    }

    let host = test_host();
    let composition = crate::scripting::CliScriptComposition::with_engine(Arc::new(FailingScript));
    let error = composition
        .run(host.lease(), Path::new("fail.py"))
        .unwrap_err();
    assert_eq!(error.kind(), CliErrorKind::App);
    assert!(error.message().contains("exit code 2"));

    fft_app::FftApp::shutdown(host.as_ref());
}

#[cfg(feature = "python")]
#[test]
fn python_composition_runs_real_scripts_and_reports_system_exit() {
    let dir = tempfile::tempdir().unwrap();
    let ok = dir.path().join("ok.py");
    std::fs::write(&ok, "print('fft-desktop-python-ok')\n").unwrap();
    let fail = dir.path().join("fail.py");
    std::fs::write(&fail, "import sys\nsys.exit(3)\n").unwrap();

    let host = test_host();
    let composition = crate::scripting::CliScriptComposition::new();
    let lease = host.lease();
    composition.run(Arc::clone(&lease), &ok).unwrap();
    let error = composition.run(lease, &fail).unwrap_err();
    assert_eq!(error.kind(), CliErrorKind::App);
    assert!(error.message().contains("exit code 3"));

    fft_app::FftApp::shutdown(host.as_ref());
}

// ---------------------------------------------------------------------------
// Root owner: leases, ordering, and shutdown
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
enum RecordedEvent {
    ServeStarted {
        lease: LeaseId,
    },
    ServeWaited,
    ServeStopped,
    ScriptStarted {
        lease: LeaseId,
        path: PathBuf,
        active_server_leases: usize,
        active_script_leases: usize,
    },
    ScriptFailed {
        path: PathBuf,
    },
}

#[derive(Default)]
struct Recorder {
    events: Vec<RecordedEvent>,
    server_leases: Vec<Weak<dyn AppLease>>,
    script_leases: Vec<(LeaseId, Weak<dyn AppLease>)>,
    fail_scripts: HashSet<PathBuf>,
    wait_error: Option<&'static str>,
    stop_error: Option<&'static str>,
}

type SharedRecorder = Arc<Mutex<Recorder>>;

fn recorder() -> SharedRecorder {
    Arc::new(Mutex::new(Recorder::default()))
}

fn with_recorder<R>(state: &SharedRecorder, action: impl FnOnce(&mut Recorder) -> R) -> R {
    let mut guard = state.lock().unwrap_or_else(|error| error.into_inner());
    action(&mut guard)
}

fn events(state: &SharedRecorder) -> Vec<RecordedEvent> {
    with_recorder(state, |recorder| recorder.events.clone())
}

fn event_kinds(state: &SharedRecorder) -> Vec<&'static str> {
    events(state)
        .iter()
        .map(|event| match event {
            RecordedEvent::ServeStarted { .. } => "serve_start",
            RecordedEvent::ServeWaited => "serve_wait",
            RecordedEvent::ServeStopped => "serve_stop",
            RecordedEvent::ScriptStarted { .. } => "script",
            RecordedEvent::ScriptFailed { .. } => "script_fail",
        })
        .collect()
}

fn script_paths(state: &SharedRecorder) -> Vec<PathBuf> {
    events(state)
        .into_iter()
        .filter_map(|event| match event {
            RecordedEvent::ScriptStarted { path, .. } => Some(path),
            _ => None,
        })
        .collect()
}

fn assert_all_leases_dropped(state: &SharedRecorder) {
    with_recorder(state, |recorder| {
        for weak in &recorder.server_leases {
            assert_eq!(
                weak.strong_count(),
                0,
                "server-lifetime lease survived the run"
            );
        }
        for (id, weak) in &recorder.script_leases {
            assert_eq!(
                weak.strong_count(),
                0,
                "script lease {id:?} survived its run"
            );
        }
    });
}

struct RecordingServe {
    state: SharedRecorder,
}

impl ServeComposition for RecordingServe {
    fn start(
        &self,
        lease: Arc<dyn AppLease>,
        _config: &CliConfig,
    ) -> Result<Box<dyn ServerSession>, CliError> {
        let id = lease.lease_id();
        with_recorder(&self.state, |recorder| {
            recorder
                .events
                .push(RecordedEvent::ServeStarted { lease: id });
            recorder.server_leases.push(Arc::downgrade(&lease));
        });
        Ok(Box::new(RecordingSession {
            state: Arc::clone(&self.state),
            lease: Some(lease),
        }))
    }
}

struct RecordingSession {
    state: SharedRecorder,
    lease: Option<Arc<dyn AppLease>>,
}

impl ServerSession for RecordingSession {
    fn wait(&mut self) -> Result<(), CliError> {
        assert!(
            self.lease.is_some(),
            "the server-lifetime lease must be alive while serving"
        );
        let error = with_recorder(&self.state, |recorder| {
            recorder.events.push(RecordedEvent::ServeWaited);
            recorder.wait_error
        });
        match error {
            Some(message) => Err(CliError::app(message)),
            None => Ok(()),
        }
    }

    fn stop(&mut self) -> Result<(), CliError> {
        let error = with_recorder(&self.state, |recorder| {
            recorder.events.push(RecordedEvent::ServeStopped);
            recorder.stop_error
        });
        self.lease.take();
        match error {
            Some(message) => Err(CliError::app(message)),
            None => Ok(()),
        }
    }
}

impl Drop for RecordingSession {
    fn drop(&mut self) {
        self.lease.take();
    }
}

struct RecordingScripts {
    state: SharedRecorder,
}

impl ScriptComposition for RecordingScripts {
    fn run(&self, lease: Arc<dyn AppLease>, path: &Path) -> Result<(), CliError> {
        let id = lease.lease_id();
        let fail = with_recorder(&self.state, |recorder| {
            let active_server_leases = recorder
                .server_leases
                .iter()
                .filter(|weak| weak.strong_count() > 0)
                .count();
            let active_script_leases = recorder
                .script_leases
                .iter()
                .filter(|(_, weak)| weak.strong_count() > 0)
                .count();
            recorder.events.push(RecordedEvent::ScriptStarted {
                lease: id,
                path: path.to_path_buf(),
                active_server_leases,
                active_script_leases,
            });
            recorder.script_leases.push((id, Arc::downgrade(&lease)));
            recorder.fail_scripts.contains(path)
        });

        // The per-run lease is dropped here, before the next script starts.
        if fail {
            with_recorder(&self.state, |recorder| {
                recorder.events.push(RecordedEvent::ScriptFailed {
                    path: path.to_path_buf(),
                });
            });
            return Err(CliError::app(format!("script {} failed", path.display())));
        }
        Ok(())
    }
}

fn test_host() -> Arc<AppHost> {
    AppHost::from_runtime(
        Arc::new(MockRuntime::new()),
        AppHostConfig {
            name: "fft_audio-cli-test".to_string(),
            version: "0.1.0".to_string(),
            mode: AppMode::Headless,
            capabilities: AppCapabilities::for_build(),
            workers: 1,
            persisted_session: None,
        },
    )
}

fn root_with(config: CliConfig, state: SharedRecorder) -> (DesktopApp, Arc<AppHost>) {
    let host = test_host();
    let adapter = DesktopApp::with_composition(
        config,
        Arc::clone(&host),
        Box::new(RecordingServe {
            state: Arc::clone(&state),
        }),
        Box::new(RecordingScripts { state }),
    );
    (adapter, host)
}

fn script_config(scripts: &[&str]) -> CliConfig {
    let mut config = CliConfig::default();
    config.workers = 1;
    config.python_scripts = scripts
        .iter()
        .map(|script| PathBuf::from(*script))
        .collect();
    config
}

#[test]
fn inert_run_shuts_down_the_host_without_starting_an_adapter() {
    let mut config = CliConfig::default();
    config.workers = 1;
    let state = recorder();
    let (adapter, host) = root_with(config, Arc::clone(&state));

    assert_eq!(adapter.run().unwrap(), CliExit::Inert);
    assert!(host.is_shutdown());
    assert!(events(&state).is_empty());
    assert_all_leases_dropped(&state);
}

#[test]
fn script_only_mints_one_scripting_lease_and_never_starts_a_server() {
    let state = recorder();
    let (adapter, host) = root_with(script_config(&["a.py", "b.py", "c.py"]), Arc::clone(&state));

    assert_eq!(adapter.run().unwrap(), CliExit::ScriptsCompleted);
    assert!(host.is_shutdown());
    assert_all_leases_dropped(&state);

    assert_eq!(
        script_paths(&state),
        vec![
            PathBuf::from("a.py"),
            PathBuf::from("b.py"),
            PathBuf::from("c.py")
        ]
    );
    assert_eq!(
        event_kinds(&state),
        vec!["script", "script", "script"],
        "script-only mode must not start or wait on a listener"
    );

    let starts: Vec<(LeaseId, usize, usize)> = events(&state)
        .into_iter()
        .filter_map(|event| match event {
            RecordedEvent::ScriptStarted {
                lease,
                active_server_leases,
                active_script_leases,
                ..
            } => Some((lease, active_server_leases, active_script_leases)),
            _ => None,
        })
        .collect();
    let ids: HashSet<LeaseId> = starts.iter().map(|(id, ..)| *id).collect();
    assert_eq!(
        ids.len(),
        1,
        "the Python adapter gets one lease for its lifetime"
    );
    for (_, active_server, _) in starts {
        assert_eq!(active_server, 0, "script-only mode holds no HTTP lease");
    }
}

#[test]
fn script_only_exit_after_script_is_redundant_but_allowed() {
    let mut config = script_config(&["a.py"]);
    config.exit_after_script = true;
    let state = recorder();
    let (adapter, host) = root_with(config, Arc::clone(&state));

    assert_eq!(adapter.run().unwrap(), CliExit::ScriptsCompleted);
    assert!(host.is_shutdown());
    assert_eq!(event_kinds(&state), vec!["script"]);
    assert_all_leases_dropped(&state);
}

#[test]
fn script_failure_stops_later_scripts_and_shuts_down_the_host() {
    let state = recorder();
    with_recorder(&state, |recorder| {
        recorder.fail_scripts.insert(PathBuf::from("b.py"));
    });
    let (adapter, host) = root_with(script_config(&["a.py", "b.py", "c.py"]), Arc::clone(&state));

    let error = adapter.run().unwrap_err();
    assert_eq!(error.kind(), CliErrorKind::App);
    assert!(error.message().contains("b.py"));
    assert!(host.is_shutdown());
    assert_all_leases_dropped(&state);
    assert_eq!(
        script_paths(&state),
        vec![PathBuf::from("a.py"), PathBuf::from("b.py")],
        "c.py must not run after the first failure"
    );
    assert_eq!(event_kinds(&state), vec!["script", "script", "script_fail"]);
}

#[test]
fn serve_mode_mints_exactly_one_server_lifetime_lease() {
    let mut config = CliConfig::default();
    config.workers = 1;
    config.serve = true;
    assert!(!config.inert());

    let state = recorder();
    let (adapter, host) = root_with(config, Arc::clone(&state));

    assert_eq!(adapter.run().unwrap(), CliExit::ServeStopped);
    assert!(host.is_shutdown());
    assert_all_leases_dropped(&state);
    assert_eq!(
        event_kinds(&state),
        vec!["serve_start", "serve_wait", "serve_stop"]
    );
    with_recorder(&state, |recorder| {
        assert_eq!(recorder.server_leases.len(), 1);
        assert!(recorder.script_leases.is_empty());
    });
}

#[test]
fn serve_with_exit_after_script_runs_scripts_sequentially_then_stops() {
    let mut config = script_config(&["a.py", "b.py"]);
    config.serve = true;
    config.exit_after_script = true;

    let state = recorder();
    let (adapter, host) = root_with(config, Arc::clone(&state));

    assert_eq!(adapter.run().unwrap(), CliExit::ServeStoppedAfterScripts);
    assert!(host.is_shutdown());
    assert_all_leases_dropped(&state);
    assert_eq!(
        event_kinds(&state),
        vec!["serve_start", "script", "script", "serve_stop"],
        "exit-after-script must not wait for a second server lifetime"
    );
    assert_eq!(
        script_paths(&state),
        vec![PathBuf::from("a.py"), PathBuf::from("b.py")]
    );

    with_recorder(&state, |recorder| {
        assert_eq!(recorder.server_leases.len(), 1);
        assert_eq!(recorder.script_leases.len(), 2);
    });
    let lease_ids: HashSet<LeaseId> = events(&state)
        .iter()
        .filter_map(|event| match event {
            RecordedEvent::ServeStarted { lease } => Some(*lease),
            RecordedEvent::ScriptStarted { lease, .. } => Some(*lease),
            _ => None,
        })
        .collect();
    assert_eq!(
        lease_ids.len(),
        2,
        "the server gets one lease and the Python adapter gets one lease"
    );
    let script_states: Vec<(usize, usize)> = events(&state)
        .into_iter()
        .filter_map(|event| match event {
            RecordedEvent::ScriptStarted {
                active_server_leases,
                active_script_leases,
                ..
            } => Some((active_server_leases, active_script_leases)),
            _ => None,
        })
        .collect();
    assert_eq!(
        script_states,
        vec![(1, 0), (1, 1)],
        "the server lease stays alive and sequential scripts share the Python lease"
    );
}

#[test]
fn serve_with_scripts_keeps_serving_without_exit_after() {
    let mut config = script_config(&["a.py"]);
    config.serve = true;

    let state = recorder();
    let (adapter, host) = root_with(config, Arc::clone(&state));

    assert_eq!(adapter.run().unwrap(), CliExit::ServeStopped);
    assert!(host.is_shutdown());
    assert_all_leases_dropped(&state);
    assert_eq!(
        event_kinds(&state),
        vec!["serve_start", "script", "serve_wait", "serve_stop"]
    );
}

#[test]
fn script_failure_in_serve_mode_stops_the_server_and_skips_remaining_scripts() {
    let mut config = script_config(&["a.py", "b.py", "c.py"]);
    config.serve = true;
    config.exit_after_script = true;

    let state = recorder();
    with_recorder(&state, |recorder| {
        recorder.fail_scripts.insert(PathBuf::from("b.py"));
    });
    let (adapter, host) = root_with(config, Arc::clone(&state));

    let error = adapter.run().unwrap_err();
    assert_eq!(error.kind(), CliErrorKind::App);
    assert!(error.message().contains("b.py"));
    assert!(host.is_shutdown());
    assert_all_leases_dropped(&state);
    assert_eq!(
        script_paths(&state),
        vec![PathBuf::from("a.py"), PathBuf::from("b.py")]
    );
    assert_eq!(
        event_kinds(&state),
        vec![
            "serve_start",
            "script",
            "script",
            "script_fail",
            "serve_stop"
        ]
    );
}

#[test]
fn server_wait_error_still_stops_the_server_and_shuts_down() {
    let mut config = CliConfig::default();
    config.workers = 1;
    config.serve = true;
    let state = recorder();
    with_recorder(&state, |recorder| {
        recorder.wait_error = Some("server failed");
    });
    let (adapter, host) = root_with(config, Arc::clone(&state));

    let error = adapter.run().unwrap_err();
    assert_eq!(error.kind(), CliErrorKind::App);
    assert!(error.message().contains("server failed"));
    assert!(host.is_shutdown());
    assert_all_leases_dropped(&state);
    assert_eq!(
        event_kinds(&state),
        vec!["serve_start", "serve_wait", "serve_stop"]
    );
}

#[test]
fn server_stop_error_is_reported_after_cleanup() {
    let mut config = CliConfig::default();
    config.workers = 1;
    config.serve = true;
    let state = recorder();
    with_recorder(&state, |recorder| {
        recorder.stop_error = Some("stop failed");
    });
    let (adapter, host) = root_with(config, Arc::clone(&state));

    let error = adapter.run().unwrap_err();
    assert_eq!(error.kind(), CliErrorKind::App);
    assert!(error.message().contains("stop failed"));
    assert!(host.is_shutdown());
    assert_all_leases_dropped(&state);
}

#[test]
fn dropping_an_unrun_root_shuts_down_the_host() {
    let mut config = CliConfig::default();
    config.workers = 1;
    let host = test_host();
    assert!(!host.is_shutdown());

    let adapter = DesktopApp::with_host(config, Arc::clone(&host));
    assert!(Arc::ptr_eq(adapter.host(), &host));
    drop(adapter);
    assert!(host.is_shutdown());
}

#[test]
fn cli_error_exposes_stable_codes_and_exit_status() {
    let error = CliError::app("boom");
    assert_eq!(error.kind(), CliErrorKind::App);
    assert_eq!(error.code(), "app");
    assert_eq!(error.exit_code(), 1);
    assert_eq!(error.to_string(), "boom");

    let error = CliError::usage("bad flags");
    assert_eq!(error.code(), "usage");
    assert_eq!(error.exit_code(), 2);
}

#[test]
fn cli_exit_codes_and_served_flags() {
    assert_eq!(CliExit::Inert.code(), 0);
    assert_eq!(CliExit::GuiClosed.code(), 0);
    assert_eq!(CliExit::ScriptsCompleted.code(), 0);
    assert_eq!(CliExit::ScriptsCompleted.exit_code(), 0);
    assert_eq!(CliExit::ServeStopped.code(), 0);
    assert_eq!(CliExit::ServeStoppedAfterScripts.code(), 0);
    assert!(!CliExit::Inert.served());
    assert!(!CliExit::GuiClosed.served());
    assert!(!CliExit::ScriptsCompleted.served());
    assert!(CliExit::ServeStopped.served());
    assert!(CliExit::ServeStoppedAfterScripts.served());
}
