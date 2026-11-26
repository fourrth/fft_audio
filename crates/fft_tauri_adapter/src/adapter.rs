//! Tauri IPC adapter for a root-owned app host.
//!
//! The product root owns the concrete `AppHost` and mints every lease. This
//! adapter receives one GUI lifetime lease plus an optional, separate scripting
//! lease and engine injected by the root. It never mints a lease or shuts down
//! the host.
//!
//! No `AppHandle` is stored in the managed state. The event translator thread
//! owns the only long-lived handle and is explicitly stopped and joined before
//! the GUI lease is dropped.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::thread::JoinHandle;

use fft_app::AppLease;
#[cfg(test)]
use fft_app_host::AppHost;
use fft_runtime::ScriptRunId;

use crate::core::{app_info_from, CommandCore, DisplayHostOps};
use crate::dto::{AppCapabilitiesInfo, AppInfo};
use crate::events::{spawn_translator, EventEmitter, TauriEmitter, TransportSink};
use crate::paths::FilePaths;

pub(crate) const PYTHON_UNAVAILABLE: &str = "Python scripting is not included in this build";

/// A scripting engine and the root-minted lease granted for its lifetime.
pub struct TauriScripting {
    pub lease: Arc<dyn AppLease>,
    pub engine: Arc<dyn fft_script::FftScript>,
}

/// Construction options for the GUI adapter.
#[derive(Clone, Debug)]
pub struct TauriOptions {
    /// Desktop configuration directory (`settings.toml`, `session.json`,
    /// `themes/`, and script examples live below it).
    pub config_dir: PathBuf,
}

impl TauriOptions {
    pub fn new(config_dir: impl Into<PathBuf>) -> Self {
        Self {
            config_dir: config_dir.into(),
        }
    }

    /// Resolve the standard Tauri app-config directory.
    pub fn from_app(app: &tauri::AppHandle) -> Result<Self, String> {
        use tauri::Manager as _;
        app.path()
            .app_config_dir()
            .map(Self::new)
            .map_err(|error| format!("could not locate the config directory: {error}"))
    }
}

/// Tauri IPC adapter over root-minted GUI/scripting leases.
pub struct TauriAdapter {
    core: Arc<CommandCore>,
    scripts: Arc<ScriptRunner>,
    sink: Arc<TransportSink>,
    /// The translator owns the only strong `AppHandle`; this handle stops and
    /// joins it so the managed state cannot form a manager reference cycle.
    translator: Mutex<Option<JoinHandle<()>>>,
    closed: AtomicBool,
    info: AppInfo,
    capabilities: AppCapabilitiesInfo,
    script_lease: Mutex<Option<Arc<dyn AppLease>>>,
}

impl TauriAdapter {
    /// Build the GUI adapter with leases minted by the product root.
    ///
    /// `gui_lease` is borrowed by commands for the adapter lifetime. When
    /// scripting is enabled, `TauriScripting` supplies a separate lease and the
    /// engine that owns it; this adapter never mints or replaces either.
    pub fn new(
        app: tauri::AppHandle,
        gui_lease: Arc<dyn AppLease>,
        ops: Arc<dyn DisplayHostOps>,
        options: TauriOptions,
        scripting: Option<TauriScripting>,
    ) -> Self {
        let (sender, receiver) = std::sync::mpsc::channel();
        let sink = Arc::new(TransportSink::channel(sender));
        let translator = Some(spawn_translator(Arc::new(TauriEmitter::new(app)), receiver));
        let engine = scripting
            .as_ref()
            .map(|scripting| Arc::clone(&scripting.engine));
        let script_lease = scripting.map(|scripting| scripting.lease);
        Self::build(
            gui_lease,
            ops,
            options,
            sink,
            translator,
            ScriptRunner::new(engine),
            script_lease,
            true,
        )
    }

    #[allow(clippy::too_many_arguments)] // test/root composition seam
    fn build(
        gui_lease: Arc<dyn AppLease>,
        ops: Arc<dyn DisplayHostOps>,
        options: TauriOptions,
        sink: Arc<TransportSink>,
        translator: Option<JoinHandle<()>>,
        scripts: ScriptRunner,
        script_lease: Option<Arc<dyn AppLease>>,
        attach_cleanup: bool,
    ) -> Self {
        let info = app_info_from(gui_lease.app().info(), gui_lease.app().capabilities());
        let capabilities = AppCapabilitiesInfo::from_capabilities(gui_lease.app().capabilities());
        let core = Arc::new(CommandCore::new(
            gui_lease,
            ops,
            Arc::clone(&sink),
            FilePaths::new(options.config_dir.clone()),
        ));
        if attach_cleanup {
            sink.attach_cleanup(Arc::downgrade(&core));
        }
        core.subscribe();
        Self {
            core,
            scripts: Arc::new(scripts),
            sink,
            translator: Mutex::new(translator),
            closed: AtomicBool::new(false),
            info,
            capabilities,
            script_lease: Mutex::new(script_lease),
        }
    }

    /// GUI-free constructor used by adapter lifecycle tests. The test acts as
    /// the product root and mints the GUI/scripting leases.
    #[cfg(test)]
    pub(crate) fn for_tests(
        host: Arc<AppHost>,
        emitter: Arc<dyn EventEmitter>,
        config_dir: impl Into<PathBuf>,
    ) -> Self {
        Self::for_tests_inner(host, emitter, config_dir, None)
    }

    #[cfg(all(test, feature = "python"))]
    pub(crate) fn for_tests_with_scripting(
        host: Arc<AppHost>,
        emitter: Arc<dyn EventEmitter>,
        config_dir: impl Into<PathBuf>,
        engine: Arc<dyn fft_script::FftScript>,
    ) -> Self {
        Self::for_tests_inner(host, emitter, config_dir, Some(engine))
    }

    #[cfg(test)]
    fn for_tests_inner(
        host: Arc<AppHost>,
        emitter: Arc<dyn EventEmitter>,
        config_dir: impl Into<PathBuf>,
        engine: Option<Arc<dyn fft_script::FftScript>>,
    ) -> Self {
        let gui_lease = host.lease();
        let script_lease = engine.as_ref().map(|_| host.lease());
        let ops: Arc<dyn DisplayHostOps> = Arc::clone(&host) as Arc<dyn DisplayHostOps>;
        Self::build(
            gui_lease,
            ops,
            TauriOptions::new(config_dir),
            Arc::new(TransportSink::inline(emitter)),
            None,
            ScriptRunner::new(engine),
            script_lease,
            false,
        )
    }

    /// Install the adapter as managed Tauri state.
    pub fn manage(self, app: &tauri::AppHandle) -> bool {
        use tauri::Manager as _;
        app.manage(self)
    }

    pub(crate) fn core(&self) -> &Arc<CommandCore> {
        &self.core
    }

    pub fn app_info(&self) -> AppInfo {
        self.info.clone()
    }

    pub fn app_capabilities(&self) -> AppCapabilitiesInfo {
        self.capabilities.clone()
    }

    /// The one GUI-lifetime lease, or `None` after shutdown.
    pub fn root_lease_id(&self) -> Option<fft_app::LeaseId> {
        self.core.lease().ok().map(|lease| lease.lease_id())
    }

    pub fn is_shutdown(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// Stop scripts, stop the translator, and drop both adapter leases. The
    /// product root shuts down the app host/runtime after this returns.
    pub fn shutdown(&self) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        self.scripts.shutdown();
        // Reject new commands while queued events are still drained.
        self.core.begin_shutdown();
        self.sink.shutdown();
        if let Some(translator) = self
            .translator
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
        {
            let _ = translator.join();
        }
        self.core.drop_lease();
        self.script_lease
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
    }

    /// Start one embedded script run for `display_id`.
    ///
    /// The runner receives a clone of the root-minted scripting lease; the
    /// engine drops its clone when the run exits.
    pub(crate) fn script_run(
        &self,
        display_id: u64,
        path: String,
        emitter: Arc<dyn EventEmitter>,
    ) -> Result<(), String> {
        if self.is_shutdown() {
            return Err("app has been shut down".to_string());
        }
        let lease = self
            .script_lease
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
            .ok_or_else(|| PYTHON_UNAVAILABLE.to_string())?;
        let sink = Arc::new(RunSink::new(
            Arc::downgrade(&self.scripts),
            emitter,
            display_id,
        ));
        self.scripts.start(lease, display_id, path, sink)
    }

    pub(crate) fn script_stop(&self, display_id: u64) -> Result<(), String> {
        if self
            .script_lease
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .is_none()
        {
            return Err(PYTHON_UNAVAILABLE.to_string());
        }
        self.scripts.stop(display_id);
        Ok(())
    }
}

impl Drop for TauriAdapter {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Per-GUI-adapter script bookkeeping.
///
/// The engine is language-neutral; Python is only the current injected engine.
pub(crate) struct ScriptRunner {
    runs: Mutex<HashMap<u64, ScriptRunId>>,
    stopping: AtomicBool,
    engine: Mutex<Option<Arc<dyn fft_script::FftScript>>>,
}

impl ScriptRunner {
    fn new(engine: Option<Arc<dyn fft_script::FftScript>>) -> Self {
        Self {
            runs: Mutex::new(HashMap::new()),
            stopping: AtomicBool::new(false),
            engine: Mutex::new(engine),
        }
    }

    #[cfg(all(test, feature = "python"))]
    pub(crate) fn with_engine(engine: Arc<dyn fft_script::FftScript>) -> Arc<Self> {
        Arc::new(Self::new(Some(engine)))
    }

    fn lock_runs(&self) -> std::sync::MutexGuard<'_, HashMap<u64, ScriptRunId>> {
        self.runs.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// Remove the mapping only when it still refers to `run`; a newer run for
    /// the same display must never be forgotten by a stale exit event.
    fn finish(&self, display_id: u64, run: ScriptRunId) {
        let mut runs = self.lock_runs();
        if runs.get(&display_id) == Some(&run) {
            runs.remove(&display_id);
        }
    }

    pub(crate) fn start(
        &self,
        lease: Arc<dyn AppLease>,
        display_id: u64,
        path: String,
        sink: Arc<RunSink>,
    ) -> Result<(), String> {
        let engine = {
            let engine = self
                .engine
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            engine
                .as_ref()
                .cloned()
                .ok_or_else(|| PYTHON_UNAVAILABLE.to_string())?
        };
        if self.stopping.load(Ordering::SeqCst) {
            return Err("the app is shutting down".to_string());
        }

        let events: Arc<dyn fft_script::ScriptEventSink> = sink.clone();
        let run = engine
            .run(
                lease,
                fft_script::ScriptSource::File(PathBuf::from(path)),
                events,
            )
            .map_err(|error| error.to_string())?;

        {
            let mut runs = self.lock_runs();
            if self.stopping.load(Ordering::SeqCst) {
                drop(runs);
                engine.cancel(run);
                return Err("the app is shutting down".to_string());
            }
            runs.insert(display_id, run);
        }
        // A trivial script can finish before `run` returns. Its exit callback
        // may already have cleared the map, so do not recreate a stale entry.
        if sink.completed() {
            self.finish(display_id, run);
        }
        Ok(())
    }

    fn stop(&self, display_id: u64) {
        let run = self.lock_runs().remove(&display_id);
        if let Some(run) = run {
            if let Some(engine) = self
                .engine
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .as_ref()
                .cloned()
            {
                engine.cancel(run);
            }
        }
    }

    /// Cancel every active run and drop the engine. Cancellation is
    /// cooperative; the engine releases the per-run lease on exit.
    pub(crate) fn shutdown(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        let runs: Vec<ScriptRunId> = self.lock_runs().drain().map(|(_, run)| run).collect();
        let engine = self
            .engine
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        if let Some(engine) = engine {
            for run in runs {
                engine.cancel(run);
            }
        }
    }
}

pub(crate) struct RunSink {
    runner: Weak<ScriptRunner>,
    emitter: Arc<dyn EventEmitter>,
    display_id: u64,
    completed: AtomicBool,
}

impl RunSink {
    pub(crate) fn new(
        runner: Weak<ScriptRunner>,
        emitter: Arc<dyn EventEmitter>,
        display_id: u64,
    ) -> Self {
        Self {
            runner,
            emitter,
            display_id,
            completed: AtomicBool::new(false),
        }
    }

    fn completed(&self) -> bool {
        self.completed.load(Ordering::SeqCst)
    }
}

pub(crate) fn script_stream_name(stream: fft_script::ScriptStream) -> &'static str {
    match stream {
        fft_script::ScriptStream::Stdout => "stdout",
        fft_script::ScriptStream::Stderr => "stderr",
        fft_script::ScriptStream::Log => "system",
    }
}

impl fft_script::ScriptEventSink for RunSink {
    fn output(&self, _run: ScriptRunId, stream: fft_script::ScriptStream, line: String) {
        let event = crate::dto::ScriptOutputEvent {
            display_id: self.display_id,
            stream: script_stream_name(stream),
            line,
        };
        if let Ok(payload) = serde_json::to_value(event) {
            self.emitter.emit("script-output", payload);
        }
    }

    fn exited(&self, run: ScriptRunId, success: bool, code: Option<i32>) {
        let event = crate::dto::ScriptExitEvent {
            display_id: self.display_id,
            code,
            success,
        };
        if let Ok(payload) = serde_json::to_value(event) {
            self.emitter.emit("script-exit", payload);
        }
        self.completed.store(true, Ordering::SeqCst);
        if let Some(runner) = self.runner.upgrade() {
            runner.finish(self.display_id, run);
        }
    }
}
