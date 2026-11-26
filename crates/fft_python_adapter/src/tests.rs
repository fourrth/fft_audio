//! Tests for `fft_python_adapter`.
//!
//! The embedded interpreter is process-global, so Python tests serialize on
//! [`PY_TEST_LOCK`]. Lease/lifecycle tests use `fft_test`; the real
//! `fft_app_host`/`CoreRuntime` is used only for data/array integration.

#![cfg(feature = "python")]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use fft_app::{AppEvent, AppLease, FftApp};
use fft_app_host::{AppHost, AppHostConfig};
use fft_script::{
    CapturingScriptEvents, FftScript, ScriptError, ScriptEvent, ScriptEventSink, ScriptRunId,
    ScriptSource, ScriptStream,
};
use fft_test::TestHost;

use super::PythonScript;

/// The embedded interpreter is process-global; Rust tests run in parallel, so
/// serialize the Python tests rather than relying on Busy.
static PY_TEST_LOCK: Mutex<()> = Mutex::new(());

fn test_serial() -> std::sync::MutexGuard<'static, ()> {
    PY_TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

fn wait_for_exit(events: &CapturingScriptEvents, timeout: Duration) -> ScriptEvent {
    let deadline = Instant::now() + timeout;
    loop {
        let snapshot = events.events();
        if let Some(event) = snapshot
            .iter()
            .rev()
            .find(|event| matches!(event, ScriptEvent::Exited { .. }))
        {
            return event.clone();
        }
        if Instant::now() >= deadline {
            panic!("script did not exit in time; events: {snapshot:#?}");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn wait_for_line(events: &CapturingScriptEvents, line: &str, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        if output_lines(events)
            .iter()
            .any(|candidate| candidate == line)
        {
            return;
        }
        if Instant::now() >= deadline {
            panic!(
                "script did not emit {line:?} in time; events: {:#?}",
                events.events()
            );
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn output_lines(events: &CapturingScriptEvents) -> Vec<String> {
    events
        .events()
        .into_iter()
        .filter_map(|event| match event {
            ScriptEvent::Output { line, .. } => Some(line),
            ScriptEvent::Exited { .. } => None,
        })
        .collect()
}

fn output_lines_for(events: &CapturingScriptEvents, stream: ScriptStream) -> Vec<String> {
    events
        .events()
        .into_iter()
        .filter_map(|event| match event {
            ScriptEvent::Output {
                stream: actual,
                line,
                ..
            } if actual == stream => Some(line),
            _ => None,
        })
        .collect()
}

fn run_script(
    script: &PythonScript,
    lease: Arc<dyn AppLease>,
    source: &str,
) -> (ScriptRunId, Arc<CapturingScriptEvents>) {
    let events = Arc::new(CapturingScriptEvents::new());
    let sink: Arc<dyn ScriptEventSink> = events.clone();
    let run = script
        .run(
            lease,
            ScriptSource::Inline {
                name: "test_script.py".into(),
                source: source.into(),
            },
            sink,
        )
        .unwrap();
    (run, events)
}

fn assert_exited(
    run: ScriptRunId,
    events: &CapturingScriptEvents,
    success: bool,
    code: Option<i32>,
) {
    assert_eq!(
        wait_for_exit(events, Duration::from_secs(20)),
        ScriptEvent::Exited { run, success, code },
        "lines: {:#?}",
        output_lines(events)
    );
}

fn core_host() -> (Arc<AppHost>, Arc<dyn AppLease>) {
    let host = AppHost::new(AppHostConfig {
        name: "fft-python-adapter-test".to_string(),
        workers: 2,
        ..AppHostConfig::default()
    });
    let lease = host.lease();
    (host, lease)
}

fn write_test_wav(path: &Path) {
    let sample_rate = 8_000u32;
    let samples: Vec<i16> = (0..800)
        .map(|index| ((index as f64 * 0.1).sin() * 10_000.0) as i16)
        .collect();
    let data_len = (samples.len() * 2) as u32;
    let mut bytes = Vec::with_capacity(44 + samples.len() * 2);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    std::fs::write(path, bytes).expect("write test wav");
}

fn temp_path(label: &str) -> PathBuf {
    let base = Path::new("/tmp/opencode");
    std::fs::create_dir_all(base).expect("create /tmp/opencode");
    base.join(format!("fft_python_adapter_{}_{label}", std::process::id()))
}

#[test]
fn python_script_is_a_lease_aware_fft_script() {
    let _serial = test_serial();

    fn assert_send_sync<T: FftScript + Send + Sync + 'static>() {}
    fn assert_object_safe(_: &dyn FftScript) {}

    assert_send_sync::<PythonScript>();
    let script = PythonScript::new();
    assert_object_safe(&script);
    assert!(!script.is_running());
    assert_eq!(script.active_run(), None);
    let boxed: Arc<dyn FftScript> = Arc::new(script);
    assert!(Arc::strong_count(&boxed) == 1);
}

#[test]
fn typed_module_api_has_no_public_dicts_and_assets_are_present() {
    let _serial = test_serial();
    let host = TestHost::new();
    let script = PythonScript::new();
    let (run, events) = run_script(
        &script,
        host.lease(),
        r#"
import sys
import fft_audio as fa

assert __name__ == "__main__"
assert __file__ == "test_script.py"
assert fa.Window.HANN.value == "hann"
assert fa.Window.HANN.name == "HANN"
assert fa.Window.HANN == fa.Window("hann") == "hann"
assert str(fa.CutAxis.RPM) == "rpm"
assert fa.CutAxis("time") == fa.CutAxis.TIME
assert str(fa.JobStatus.PENDING) == "pending"
assert fa.JobStatus("ready") == fa.JobStatus.READY
assert fa.DisplayKind.SPECTROGRAM.value == "spectrogram"
assert fa.DisplayKind("order_cuts") == fa.DisplayKind.ORDER_CUTS
assert fa.DisplayKind("plot") == fa.DisplayKind.PLOT
assert fa.DisplayKind.PLOT.name == "PLOT"
assert issubclass(fa.DataDetachedError, RuntimeError)
assert issubclass(fa.DataBorrowedError, RuntimeError)
assert issubclass(fa.CancelledError, RuntimeError)
assert issubclass(fa.JobFailedError, RuntimeError)

app = fa.app()
assert isinstance(app, fa.App)
assert isinstance(fa.App(), fa.App)
info = app.info()
assert isinstance(info, fa.AppInfo)
assert isinstance(info.name, str)
assert isinstance(info.version, str)
assert info.mode in ("desktop", "headless")
assert not hasattr(info, "__dict__")
assert not getattr(info, "__annotations__", None)
assert app.version() == info.version
assert app.runtime_name() == info.name
print("module-ok")
print("module-err", file=sys.stderr)
"#,
    );
    assert_exited(run, &events, true, Some(0));
    assert!(output_lines_for(&events, ScriptStream::Stdout)
        .iter()
        .any(|line| line == "module-ok"));
    assert!(output_lines_for(&events, ScriptStream::Stderr)
        .iter()
        .any(|line| line == "module-err"));

    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let stub = manifest.join("src/fft_audio.pyi");
    let typed = manifest.join("src/py.typed");
    assert!(stub.is_file(), "missing {stub:?}");
    assert!(typed.is_file(), "missing {typed:?}");
    let stub_text = std::fs::read_to_string(stub).unwrap();
    for name in [
        "class App",
        "class Signal",
        "class SpectrogramHandle",
        "class OrderCutHandle",
        "class LineCutHandle",
        "class Display",
    ] {
        assert!(stub_text.contains(name), "stub is missing {name}");
    }
    assert!(manifest.join("src/fft_audio.py").is_file());
}

#[test]
fn the_lease_is_held_during_the_run_and_dropped_after_success() {
    let _serial = test_serial();
    let host = TestHost::new();
    let lease = host.lease();
    let lease_id = lease.lease_id();
    let script = PythonScript::new();
    let gate = temp_path("lease_gate");
    let _ = std::fs::remove_file(&gate);
    let source = format!(
        r#"
import os
import time
import fft_audio as fa

app = fa.app()
assert app.info().name == "fft_audio-test"
print("started")
deadline = time.monotonic() + 5.0
while not os.path.exists(r"{gate}") and time.monotonic() < deadline:
    time.sleep(0.005)
print("finished")
"#,
        gate = gate.display()
    );
    let (run, events) = run_script(&script, lease, &source);

    wait_for_line(&events, "started", Duration::from_secs(5));
    assert!(
        host.is_lease_active(lease_id),
        "the run lease must stay active while the script executes"
    );
    assert!(script.is_running());
    assert_eq!(script.active_run(), Some(run));

    std::fs::write(&gate, b"go").expect("release script gate");
    assert_exited(run, &events, true, Some(0));
    let _ = std::fs::remove_file(&gate);
    host.assert_lease_dropped(lease_id);
    host.assert_all_leases_dropped();
    assert!(!script.is_running());
    assert_eq!(script.active_run(), None);
    assert!(output_lines(&events).iter().any(|line| line == "finished"));
}

#[test]
fn the_lease_is_dropped_after_a_python_exception() {
    let _serial = test_serial();
    let host = TestHost::new();
    let lease = host.lease();
    let lease_id = lease.lease_id();
    let script = PythonScript::new();
    let (run, events) = run_script(
        &script,
        lease,
        "def boom():\n    raise RuntimeError('kaboom')\nboom()\n",
    );
    assert_exited(run, &events, false, Some(1));
    host.assert_lease_dropped(lease_id);
    host.assert_all_leases_dropped();
}

#[test]
fn the_lease_is_dropped_after_system_exit_and_the_host_survives() {
    let _serial = test_serial();
    let host = TestHost::new();
    let script = PythonScript::new();

    for (source, success, code) in [
        ("print('before-none')\nraise SystemExit\n", true, Some(0)),
        ("print('before-zero')\nraise SystemExit(0)\n", true, Some(0)),
        (
            "print('before-five')\nraise SystemExit(5)\n",
            false,
            Some(5),
        ),
        ("raise SystemExit('non-integer exit')\n", false, Some(1)),
    ] {
        let lease = host.lease();
        let lease_id = lease.lease_id();
        let (run, events) = run_script(&script, lease, source);
        assert_exited(run, &events, success, code);
        assert!(
            !output_lines(&events)
                .iter()
                .any(|line| line.contains("Traceback")),
            "SystemExit must not print a traceback: {:#?}",
            output_lines(&events)
        );
        host.assert_lease_dropped(lease_id);
    }
    host.assert_all_leases_dropped();

    // A SystemExit run did not terminate the interpreter: a later run works.
    let (run, events) = run_script(&script, host.lease(), "print('still-alive')\n");
    assert_exited(run, &events, true, Some(0));
    assert!(output_lines(&events)
        .iter()
        .any(|line| line == "still-alive"));
}

#[test]
fn a_retained_python_handle_cannot_reach_the_finished_runs_app() {
    let _serial = test_serial();
    let host = TestHost::new();
    let script = PythonScript::new();

    let first = host.lease();
    let first_id = first.lease_id();
    let (first_run, first_events) = run_script(
        &script,
        first,
        r#"
import builtins
import fft_audio as fa

builtins._n5_saved_app = fa.app()
print("saved")
"#,
    );
    assert_exited(first_run, &first_events, true, Some(0));
    host.assert_lease_dropped(first_id);

    let second = host.lease();
    let second_id = second.lease_id();
    let (second_run, second_events) = run_script(
        &script,
        second,
        r#"
import builtins

saved = builtins._n5_saved_app
try:
    saved.info()
    raise AssertionError("a retained App must not reach a finished run")
except RuntimeError as error:
    assert "ended" in str(error), str(error)
finally:
    builtins.__dict__.pop("_n5_saved_app", None)
print("inert")
"#,
    );
    assert_exited(second_run, &second_events, true, Some(0));
    host.assert_lease_dropped(second_id);
    assert!(output_lines(&second_events)
        .iter()
        .any(|line| line == "inert"));
}

#[test]
fn a_second_concurrent_run_is_busy_and_drops_its_lease() {
    let _serial = test_serial();
    let host = TestHost::new();
    let script = PythonScript::new();

    let gate = temp_path("busy_gate");
    let _ = std::fs::remove_file(&gate);
    let source = format!(
        r#"
import os
import time

print("first")
deadline = time.monotonic() + 5.0
while not os.path.exists(r"{gate}") and time.monotonic() < deadline:
    time.sleep(0.005)
print("finished")
"#,
        gate = gate.display()
    );
    let first = host.lease();
    let first_id = first.lease_id();
    let (run, events) = run_script(&script, first, &source);
    wait_for_line(&events, "first", Duration::from_secs(5));
    assert_eq!(script.active_run(), Some(run));

    let second = host.lease();
    let second_id = second.lease_id();
    let second_events = Arc::new(CapturingScriptEvents::new());
    let sink: Arc<dyn ScriptEventSink> = second_events.clone();
    let busy = script.run(
        second,
        ScriptSource::Inline {
            name: "second.py".into(),
            source: "print('second')\n".into(),
        },
        sink,
    );
    assert!(matches!(busy, Err(ScriptError::Busy { .. })));
    assert_eq!(host.active_leases(), 1);
    assert!(host.dropped_lease_ids().contains(&second_id));
    assert!(second_events.events().is_empty());

    std::fs::write(&gate, b"go").expect("release script gate");
    assert_exited(run, &events, true, Some(0));
    let _ = std::fs::remove_file(&gate);
    host.assert_lease_dropped(first_id);
    host.assert_all_leases_dropped();
}

#[test]
fn cancellation_is_cooperative_and_drops_the_lease() {
    let _serial = test_serial();
    let host = TestHost::new();
    let script = PythonScript::new();
    let lease = host.lease();
    let lease_id = lease.lease_id();
    let (run, events) = run_script(
        &script,
        lease,
        r#"
import time
import fft_audio as fa

app = fa.app()
for _ in range(2000):
    app.displays()
    time.sleep(0.005)
print("should-not-finish")
"#,
    );
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(script.active_run(), Some(run));
    script.cancel(run);
    let exit = wait_for_exit(&events, Duration::from_secs(5));
    assert!(
        matches!(exit, ScriptEvent::Exited { success: false, .. }),
        "unexpected exit: {exit:?}"
    );
    assert!(output_lines(&events)
        .iter()
        .any(|line| line.contains("cancelled")));
    assert!(!script.is_running());
    host.assert_lease_dropped(lease_id);
    host.assert_all_leases_dropped();
}

#[test]
fn lazy_handles_status_wait_and_display_lifecycle_are_typed() {
    let _serial = test_serial();
    let host = TestHost::new();
    let script = PythonScript::new();
    let (run, events) = run_script(
        &script,
        host.lease(),
        r#"
import fft_audio as fa

app = fa.app()
sig = app.load_signal([0.0] * 256, sample_rate=8000.0, name="tone")
assert sig.name == "tone"
assert sig.sample_count == 256
assert abs(sig.duration_s - 0.032) < 1e-9
spec = sig.spectrogram(fft_size=128, frame=128, hop=64, window="hann", show=True)
assert isinstance(spec.status, fa.JobStatus)
app.wait_all()
assert spec.status == fa.JobStatus.READY
assert spec.info.frames == 0  # the mock runtime has empty data
oc = spec.order_cut(fundamental_hz=100.0, orders=[1.0, 2.0], axis="rpm", show_graph=True)
lc = spec.line_cut(start=(0.0, 10.0), end=(0.01, 200.0), num_points=8, show_graph=False)
app.wait_all()
spec.wait()
spec.wait(timeout_s=0.0)
oc.wait()
lc.wait()
assert spec.as_numpy().shape == (0, 0)
assert oc.as_numpy().shape == (0, 2)
assert lc.to_numpy().shape == (0, 2)

parent = spec.display
child = oc.display
assert parent is not None and child is not None
assert child.parent_id == parent.id
assert child.kind == fa.DisplayKind.ORDER_CUTS
assert parent.kind == fa.DisplayKind.SPECTROGRAM
parent.hide()
assert not parent.visible
parent.show()
assert parent.visible
assert len(app.displays()) == 2
app.close_all()
assert len(app.displays()) == 0
print("lazy-ok")
"#,
    );
    assert_exited(run, &events, true, Some(0));
    assert!(output_lines(&events).iter().any(|line| line == "lazy-ok"));
}

#[test]
fn failed_jobs_expose_nonblocking_status_and_error() {
    let _serial = test_serial();
    let host = TestHost::new();
    host.app().fail_next_analysis("synthetic python failure");
    let script = PythonScript::new();
    let (run, events) = run_script(
        &script,
        host.lease(),
        r#"
import fft_audio as fa

app = fa.app()
sig = app.load_signal([0.0] * 64, sample_rate=8000.0)
spec = sig.spectrogram(fft_size=64, show=False)
assert str(spec.status) == "failed"
err = spec.error
assert isinstance(err, Exception)
assert "synthetic python failure" in str(err)
print("status-ok")
"#,
    );
    assert_exited(run, &events, true, Some(0));
    assert!(output_lines(&events).iter().any(|line| line == "status-ok"));
}

#[test]
fn stdout_stderr_and_tracebacks_are_captured_as_script_events() {
    let _serial = test_serial();
    let host = TestHost::new();
    let script = PythonScript::new();
    let (run, events) = run_script(
        &script,
        host.lease(),
        "import sys\nprint('out-line')\nprint('err-line', file=sys.stderr)\n",
    );
    assert_exited(run, &events, true, Some(0));
    assert!(output_lines_for(&events, ScriptStream::Stdout)
        .iter()
        .any(|line| line == "out-line"));
    assert!(output_lines_for(&events, ScriptStream::Stderr)
        .iter()
        .any(|line| line == "err-line"));

    let (run, events) = run_script(
        &script,
        host.lease(),
        "def boom():\n    raise ValueError('kaboom')\nboom()\n",
    );
    assert_exited(run, &events, false, Some(1));
    let stderr = output_lines_for(&events, ScriptStream::Stderr);
    assert!(
        stderr.iter().any(|line| line.contains("Traceback")),
        "stderr: {stderr:#?}"
    );
    assert!(
        stderr.iter().any(|line| line.contains("ValueError")),
        "stderr: {stderr:#?}"
    );
    assert!(
        stderr.iter().any(|line| line.contains("kaboom")),
        "stderr: {stderr:#?}"
    );
}

#[test]
fn waiting_releases_the_gil_for_other_python_threads() {
    let _serial = test_serial();
    let host = TestHost::held();
    let script = PythonScript::new();
    let (run, events) = run_script(
        &script,
        host.lease(),
        r#"
import threading
import time
import fft_audio as fa

app = fa.app()
sig = app.load_signal([0.0] * 128, sample_rate=8000.0)
spec = sig.spectrogram(fft_size=64, show=False)
assert str(spec.status) == "pending"

ticked = []
def worker():
    time.sleep(0.05)
    ticked.append(time.monotonic())

thread = threading.Thread(target=worker)
thread.start()
start = time.monotonic()
try:
    spec.wait(timeout_s=0.8)
    raise AssertionError("a pending job must time out")
except TimeoutError:
    pass
thread.join()
assert ticked, "the worker never ran while wait held the GIL"
assert ticked[0] - start < 0.6, ticked[0] - start
print("gil-ok")
"#,
    );
    assert_exited(run, &events, true, Some(0));
    assert!(output_lines(&events).iter().any(|line| line == "gil-ok"));
}

#[test]
fn app_events_never_contain_script_output_or_exit() {
    let _serial = test_serial();
    let host = TestHost::new();
    let script = PythonScript::new();
    let (run, events) = run_script(
        &script,
        host.lease(),
        r#"
import fft_audio as fa

app = fa.app()
print("script-output")
sig = app.load_signal([0.0] * 128, sample_rate=8000.0)
spec = sig.spectrogram(fft_size=64, show=True)
app.wait_all()
assert spec.status == fa.JobStatus.READY
"#,
    );
    assert_exited(run, &events, true, Some(0));
    let app_events = host.app().events();
    assert!(app_events.iter().all(|event| matches!(
        event,
        AppEvent::JobProgress { .. }
            | AppEvent::JobDone { .. }
            | AppEvent::JobFailed { .. }
            | AppEvent::DisplayOpened { .. }
            | AppEvent::DisplayUpdated { .. }
            | AppEvent::DisplayClosed { .. }
    )));
    let app_json = serde_json::to_string(&app_events).unwrap();
    assert!(!app_json.contains("script_output"));
    assert!(!app_json.contains("script_exited"));
    assert!(events
        .events()
        .iter()
        .any(|event| matches!(event, ScriptEvent::Output { line, .. } if line == "script-output")));
    assert!(matches!(
        events.events().last(),
        Some(ScriptEvent::Exited { .. })
    ));
}

#[test]
fn numpy_read_views_copies_and_displayed_spectrogram_are_read_only() {
    let _serial = test_serial();
    let (_host, lease) = core_host();
    let script = PythonScript::new();
    let (run, events) = run_script(
        &script,
        lease,
        r#"
import numpy as np
import fft_audio as fa

app = fa.app()
t = np.arange(4096, dtype=np.float64) / 8000.0
samples = np.sin(2.0 * np.pi * 100.0 * t)
sig = app.load_signal(samples, sample_rate=8000.0, name="sine")
spec = sig.spectrogram(fft_size=256, hop=64, show=True)
oc = spec.order_cut(fundamental_hz=100.0, orders=[1.0, 2.0], bandwidth_orders=0.2, show_graph=True)
lc = spec.line_cut(start=(0.0, 100.0), end=(0.2, 400.0), num_points=32, show_graph=False)
app.wait_all()

s = spec.as_numpy()
assert s.dtype == np.float32
assert s.shape == (spec.info.frames, spec.info.bins)
assert s.shape[0] > 0 and s.shape[1] > 0
assert s.base is spec
assert not s.flags.writeable
s2 = spec.as_numpy()
assert np.shares_memory(s, s2)
for method in ("as_numpy_mut", "into_numpy"):
    try:
        getattr(spec, method)()
        raise AssertionError(method + " must reject canonical spectrogram data")
    except fa.DataBorrowedError:
        pass
cp = spec.to_numpy()
assert cp.flags.writeable and not np.shares_memory(s, cp)

m = oc.as_numpy()
m2 = oc.as_numpy()
assert m.dtype == np.float32 and m.shape == (oc.info.frames, oc.info.order_count)
assert m.shape[0] > 0 and m.shape[1] == 2
assert not m.flags.writeable
assert m.base is oc
assert np.shares_memory(m, m2)
mc = oc.to_numpy()
assert mc.flags.writeable and not np.shares_memory(m, mc)

times, db = lc.as_arrays()
times2, db2 = lc.as_arrays()
assert times.dtype == np.float64 and db.dtype == np.float32
assert len(times) == 32 and len(db) == 32
assert not times.flags.writeable and not db.flags.writeable
assert np.shares_memory(times, times2) and np.shares_memory(db, db2)
n2 = lc.to_numpy()
assert n2.shape == (32, 2) and n2.dtype == np.float64
ti = oc.info.times
assert ti.dtype == np.float64 and len(ti) == oc.info.frames
assert not ti.flags.writeable

ocv = spec.order_cut(fundamental_hz=np.array([100.0, 120.0]), orders=[1.0], show_graph=False)
v = ocv.as_numpy()
assert v.shape[1] == 1 and v.shape[0] > 0
print("arrays-ok")
"#,
    );
    assert_exited(run, &events, true, Some(0));
    assert!(output_lines(&events).iter().any(|line| line == "arrays-ok"));
}

#[test]
fn ownership_transfer_mutable_lease_and_strided_input_are_typed() {
    let _serial = test_serial();
    let (_host, lease) = core_host();
    let script = PythonScript::new();
    let (run, events) = run_script(
        &script,
        lease,
        r#"
import numpy as np
import fft_audio as fa

app = fa.app()
base = np.arange(2048, dtype=np.float32)
strided = base[::2]
assert not strided.flags.c_contiguous
sig = app.load_signal(strided, sample_rate=8000.0, name="strided")
assert sig.sample_count == 1024

sig2 = app.load_signal(np.sin(np.arange(2048) * 0.1), sample_rate=8000.0, name="sine")
spec = sig2.spectrogram(fft_size=256, hop=64, show=False)
oc = spec.order_cut(fundamental_hz=100.0, orders=[1.0, 2.0], show_graph=False)
arr = oc.into_numpy()
assert arr.flags.writeable
assert arr.dtype == np.float32 and arr.shape[1] == 2
try:
    oc.as_numpy()
    raise AssertionError("detached handle must fail")
except fa.DataDetachedError:
    pass

oc_read = spec.order_cut(fundamental_hz=100.0, orders=[1.0], show_graph=False)
view = oc_read.as_numpy()
assert view.shape[1] == 1
try:
    oc_read.into_numpy()
    raise AssertionError("a read view must block ownership transfer")
except fa.DataBorrowedError:
    pass
oc_info = spec.order_cut(fundamental_hz=100.0, orders=[1.0], show_graph=False)
_ = oc_info.info.times
try:
    oc_info.into_numpy()
    raise AssertionError("an info snapshot must block ownership transfer")
except fa.DataBorrowedError:
    pass

oc2 = spec.order_cut(fundamental_hz=100.0, orders=[1.0], show_graph=False)
leased = oc2.as_numpy_mut()
assert leased.flags.writeable
leased[0, 0] = -123.0
assert float(leased[0, 0]) == -123.0
try:
    oc2.as_numpy_mut()
    raise AssertionError("a second mutable lease must be rejected")
except fa.DataBorrowedError:
    pass
try:
    oc2.as_numpy()
    raise AssertionError("read access during a mutable lease must be rejected")
except fa.DataBorrowedError:
    pass
oc2.close()
assert not leased.flags.writeable, "close must clear the writable lease"
try:
    leased[0, 0] = 1.0
    raise AssertionError("a closed lease array must be read-only")
except ValueError:
    pass

shown = spec.order_cut(fundamental_hz=100.0, orders=[1.0], show_graph=True)
try:
    shown.as_numpy_mut()
    raise AssertionError("displayed cut data must reject a mutable lease")
except fa.DataBorrowedError:
    pass
try:
    shown.into_numpy()
    raise AssertionError("displayed cut data must reject ownership transfer")
except fa.DataBorrowedError:
    pass
shown.close()

lc = spec.line_cut(start=(0.0, 100.0), end=(0.1, 300.0), num_points=16, show_graph=False)
lc_view = lc.to_numpy()
assert lc_view.shape == (16, 2)
assert lc_view.flags.writeable
lc_times, _ = lc.as_arrays()
assert len(lc_times) == 16
lc.close()

lc2 = spec.line_cut(start=(0.0, 100.0), end=(0.1, 300.0), num_points=16, show_graph=False)
lc_arr = lc2.into_numpy()
assert lc_arr.shape == (16, 2) and lc_arr.flags.writeable
try:
    lc2.as_arrays()
    raise AssertionError("detached line handle must fail")
except fa.DataDetachedError:
    pass
print("ownership-ok")
"#,
    );
    assert_exited(run, &events, true, Some(0));
    assert!(output_lines(&events)
        .iter()
        .any(|line| line == "ownership-ok"));
}

#[test]
fn acceptance_style_order_line_export_and_open_file_flow() {
    let _serial = test_serial();
    let (_host, lease) = core_host();
    let script = PythonScript::new();
    let wav = temp_path("tone.wav");
    let line_csv = temp_path("line.csv");
    let order_csv = temp_path("order.csv");
    let image = temp_path("spectrogram.png");
    let _ = std::fs::remove_file(&wav);
    let _ = std::fs::remove_file(&line_csv);
    let _ = std::fs::remove_file(&order_csv);
    let _ = std::fs::remove_file(&image);
    write_test_wav(&wav);

    let source = format!(
        r#"
import pathlib
import fft_audio as fa

app = fa.app()
sig = app.open_file(r"{wav}", name="wav-tone")
assert sig.name == "wav-tone"
assert sig.sample_rate == 8000.0
assert sig.sample_count == 800
spec = sig.spectrogram(fft_size=128, hop=32, show=True)
spec.wait()
info = spec.info
assert isinstance(info.path, pathlib.Path)
assert info.path.name == "{filename}"
spec.save_image(r"{image}", width=320, height=180)
assert pathlib.Path(r"{image}").exists()

oc = spec.order_cut(fundamental_hz=100.0, orders=[1.0, 2.0], show_graph=True)
oc.wait()
order_path = oc.export_csv(r"{order_csv}")
assert isinstance(order_path, pathlib.Path)
assert order_path.exists()

lc = spec.line_cut(start=(0.0, 100.0), end=(0.1, 300.0), num_points=16, show_graph=True)
first_display = lc.display
assert first_display is not None
lc.set_line(start=(0.0, 100.0), end=(0.08, 500.0), num_points=32)
second_display = lc.display
assert second_display is not None
assert second_display.id != first_display.id
lc.wait()
assert lc.to_numpy().shape == (32, 2)
line_path = lc.export_csv(r"{line_csv}")
assert isinstance(line_path, pathlib.Path)
assert line_path.exists()

assert len(app.displays()) == 3
app.close_all()
assert len(app.displays()) == 0
print("acceptance-ok")
"#,
        wav = wav.display(),
        filename = wav.file_name().unwrap().to_string_lossy(),
        image = image.display(),
        order_csv = order_csv.display(),
        line_csv = line_csv.display(),
    );
    let (run, events) = run_script(&script, lease, &source);
    assert_exited(run, &events, true, Some(0));
    assert!(output_lines(&events)
        .iter()
        .any(|line| line == "acceptance-ok"));
    assert!(wav.is_file());
    assert!(line_csv.is_file());
    assert!(order_csv.is_file());
    assert!(image.is_file());

    let _ = std::fs::remove_file(&wav);
    let _ = std::fs::remove_file(&line_csv);
    let _ = std::fs::remove_file(&order_csv);
    let _ = std::fs::remove_file(&image);
}

#[test]
fn variable_fundamental_records_points_in_display_state() {
    let _serial = test_serial();
    let (host, lease) = core_host();
    let script = PythonScript::new();
    let (run, events) = run_script(
        &script,
        lease,
        r#"
import numpy as np
import fft_audio as fa

app = fa.app()
sig = app.load_signal(np.sin(np.arange(4096) * 0.1), sample_rate=8000.0, name="sine")
spec = sig.spectrogram(fft_size=256, hop=64, show=True)
oc = spec.order_cut(
    fundamental_hz=np.array([100.0, 120.0, 140.0]),
    orders=[1.0],
    show_graph=True,
)
oc.wait()
assert oc.as_numpy().shape[1] == 1
print("rpm-display-ok")
"#,
    );
    assert_exited(run, &events, true, Some(0));
    let displays = host.displays().expect("displays");
    let order = displays
        .iter()
        .find(|display| display.kind == fft_app::DisplayKind::OrderCuts)
        .expect("order-cut display");
    assert_eq!(order.state["rpm"]["kind"], "points");
    let points = order.state["rpm"]["value"].as_array().unwrap();
    assert_eq!(points.len(), 3);
    assert_eq!(points[0]["timeS"], 0.0);
    assert_eq!(points[1]["rpm"], 120.0 * 60.0);
    assert_eq!(points[2]["rpm"], 140.0 * 60.0);
}

#[test]
fn file_source_runs_and_missing_file_reports_a_script_error() {
    let _serial = test_serial();
    let host = TestHost::new();
    let script = PythonScript::new();
    let path = temp_path("source.py");
    std::fs::write(&path, "print('file-ok')\n").expect("write script");

    let events = Arc::new(CapturingScriptEvents::new());
    let sink: Arc<dyn ScriptEventSink> = events.clone();
    let run = script
        .run(host.lease(), ScriptSource::File(path.clone()), sink)
        .unwrap();
    assert_exited(run, &events, true, Some(0));
    assert!(output_lines(&events).iter().any(|line| line == "file-ok"));
    let _ = std::fs::remove_file(&path);

    let missing = temp_path("missing.py");
    let _ = std::fs::remove_file(&missing);
    let lease = host.lease();
    let lease_id = lease.lease_id();
    let events = Arc::new(CapturingScriptEvents::new());
    let sink: Arc<dyn ScriptEventSink> = events.clone();
    let run = script
        .run(lease, ScriptSource::File(missing), sink)
        .unwrap();
    assert_exited(run, &events, false, Some(1));
    assert!(output_lines_for(&events, ScriptStream::Stderr)
        .iter()
        .any(|line| line.contains("failed to read script")));
    host.assert_lease_dropped(lease_id);
}
