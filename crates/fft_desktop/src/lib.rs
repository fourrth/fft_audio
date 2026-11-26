//! Unified `fft_audio` product binary and CLI root leaser.
//!
//! The product owns the concrete [`AppHost`], mints every lease, and decides
//! which adapters are started. Adapters receive their own `Arc<dyn AppLease>`
//! for their lifetime and never own the host or mint another lease.
//!
//! Execution model:
//!
//! - a Tauri build with no CLI action launches the GUI; `--headless` suppresses
//!   it and runs the CLI composition;
//! - no GUI, no `--python-script`, and no `--serve`: inert; build and shut down
//!   the app host without starting an adapter;
//! - `--python-script` without `--serve`: script-only, no listener;
//! - `--serve --python-script`: start the server first, then run scripts
//!   sequentially in the same app; keep serving unless `--exit-after-script`;
//! - `--python-delay`/`--python-wait` pause after adapter startup and before the
//!   first script so a UI can attach before scripted displays are emitted.
//!
//! Lease scope: one lease for the HTTP server lifetime and one lease for the
//! Python adapter lifetime. Sequential script runs share that Python lease;
//! the root drops it when the adapter is torn down. Shutdown order is scripts,
//! server, leases, app host, runtime.
//!
//! The `fft_desktop` binary owns the process exit; this library exposes the
//! reusable composition and testable CLI surface.

#[cfg(all(feature = "tauri", feature = "tauri_python"))]
compile_error!(
    "features `tauri` and `tauri_python` are mutually exclusive; enable exactly one GUI frontend"
);

mod macros;

mod args;
mod composition;
mod error;
mod http;
mod scripting;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use fft_app::{AppCapabilities, AppLease, AppMode, FftApp};
use fft_app_host::{AppHost, AppHostConfig};

use fft_core::session::SessionDocument;

use composition::{ScriptComposition, ServeComposition};
use http::HttpComposition;
use scripting::CliScriptComposition;

pub use args::{parse_args, validate_config, CliConfig, DEFAULT_BIND};
pub use error::{CliError, CliErrorKind};

#[cfg(test)]
mod tests;

/// How the CLI run ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CliExit {
    /// No adapter was selected; the app host was built and shut down.
    Inert,
    /// The desktop GUI ran and exited.
    GuiClosed,
    /// A script-only run completed; no listener was started.
    ScriptsCompleted,
    /// A WebUI run stopped (Ctrl-C or server-side end).
    ServeStopped,
    /// A WebUI run stopped because `--exit-after-script` followed the scripts.
    ServeStoppedAfterScripts,
}

impl CliExit {
    /// Product process exit status for a successful run.
    pub const fn code(self) -> i32 {
        0
    }

    /// Alias for [`CliExit::code`], mirroring [`CliError::exit_code`].
    pub const fn exit_code(self) -> i32 {
        self.code()
    }

    /// Whether this run started a WebUI listener.
    pub const fn served(self) -> bool {
        matches!(
            self,
            CliExit::ServeStopped | CliExit::ServeStoppedAfterScripts
        )
    }
}

/// Root owner of the app host and composition entry point.
///
/// [`DesktopApp::new`] validates the mode/features, builds the concrete
/// `AppHost`, and installs the feature-gated adapter composition. Dropping the
/// root shuts the app host down; [`DesktopApp::run`] also does this after the
/// adapter leases are gone.
pub struct DesktopApp {
    config: CliConfig,
    host: Arc<AppHost>,
    serve: Option<Box<dyn ServeComposition>>,
    scripts: Option<Box<dyn ScriptComposition>>,
}

impl DesktopApp {
    /// Validate `config`, build the app host, and prepare composition.
    ///
    /// Feature-off modes fail here before a host, listener, or lease exists.
    pub fn new(config: CliConfig) -> Result<Self, CliError> {
        validate_config(&config)?;
        if config.fresh {
            clear_config_files(&config.config_dir)?;
        }
        let host = build_host(&config, AppMode::Headless, None);
        Ok(Self::with_host(config, host))
    }

    /// Low-level constructor for an already-built host (embedding/tests).
    ///
    /// This does not re-validate `config`; callers that accept arbitrary user
    /// config should call [`validate_config`] first. Feature-gated composition
    /// is still selected by this crate's features.
    pub fn with_host(config: CliConfig, host: Arc<AppHost>) -> Self {
        Self {
            config,
            host,
            serve: Some(Box::new(HttpComposition)),
            scripts: Some(Box::new(CliScriptComposition::new())),
        }
    }

    /// Configuration this product root was built with.
    pub fn config(&self) -> &CliConfig {
        &self.config
    }

    /// The app host owned by this product root. Not public: only the root may
    /// hold the host (and therefore mint leases).
    #[cfg(test)]
    fn host(&self) -> &Arc<AppHost> {
        &self.host
    }

    /// Execute the configured mode, then shut down and drop the host.
    ///
    /// Script and server sessions are stopped before this method returns.
    pub fn run(mut self) -> Result<CliExit, CliError> {
        let serve = self
            .serve
            .take()
            .expect("root composition is initialized exactly once");
        let scripts = self
            .scripts
            .take()
            .expect("root composition is initialized exactly once");
        let result = run_composition(&self.config, &self.host, serve.as_ref(), scripts.as_ref());
        // The composition holds no run/server lease after `run_composition`
        // returns; drop it before the host so engines/tasks see a live app.
        drop(serve);
        drop(scripts);
        // Script and server sessions were stopped and their leases dropped
        // inside `run_composition`; the host goes last.
        self.host.shutdown();
        result
    }

    #[cfg(test)]
    fn with_composition(
        config: CliConfig,
        host: Arc<AppHost>,
        serve: Box<dyn ServeComposition>,
        scripts: Box<dyn ScriptComposition>,
    ) -> Self {
        Self {
            config,
            host,
            serve: Some(serve),
            scripts: Some(scripts),
        }
    }
}

impl Drop for DesktopApp {
    fn drop(&mut self) {
        // Idempotent; also covers a root dropped without `run`.
        self.host.shutdown();
    }
}

/// Build the host, run the configured mode, and clean up.
///
/// This is the composition entry point the `fft_desktop` binary calls after
/// parsing. See [`DesktopApp`] for the ownership/lifecycle details.
pub fn run(config: CliConfig) -> Result<CliExit, CliError> {
    DesktopApp::new(config)?.run()
}

/// Entry point used by the binary: GUI features default to the GUI unless the
/// run is explicitly headless or selects a CLI adapter.
pub fn run_product(config: CliConfig) -> Result<CliExit, CliError> {
    #[cfg(any(feature = "tauri", feature = "tauri_python"))]
    if config.wants_gui() {
        run_gui(config)?;
        return Ok(CliExit::GuiClosed);
    }
    run(config)
}

fn run_composition(
    config: &CliConfig,
    host: &Arc<AppHost>,
    serve: &dyn ServeComposition,
    script_runner: &dyn ScriptComposition,
) -> Result<CliExit, CliError> {
    let has_scripts = !config.python_scripts.is_empty();

    if !config.serve && !has_scripts {
        return Ok(CliExit::Inert);
    }

    if !config.serve {
        wait_before_scripts(config)?;
        run_scripts(host, &config.python_scripts, script_runner)?;
        return Ok(CliExit::ScriptsCompleted);
    }

    // One lease for the whole server lifetime. Request handlers borrow this
    // lease; no request or adapter mints another one.
    let server_lease = host.lease();
    let mut session = serve.start(server_lease, config)?;

    if !has_scripts {
        let wait_result = session.wait();
        let stop_result = session.stop();
        drop(session);
        wait_result?;
        stop_result?;
        return Ok(CliExit::ServeStopped);
    }

    if let Err(error) = wait_before_scripts(config)
        .and_then(|_| run_scripts(host, &config.python_scripts, script_runner))
    {
        // Stop the server and drop its lease before the host is shut down.
        let _ = session.stop();
        drop(session);
        return Err(error);
    }

    if config.exit_after_script {
        let stop_result = session.stop();
        drop(session);
        stop_result?;
        return Ok(CliExit::ServeStoppedAfterScripts);
    }

    let wait_result = session.wait();
    let stop_result = session.stop();
    drop(session);
    wait_result?;
    stop_result?;
    Ok(CliExit::ServeStopped)
}

/// Give the user time to attach a UI before backend displays are emitted.
///
/// HTTP starts first, prints its URL, and only then reaches this
/// wait. This is a no-op unless `--python-delay` or `--python-wait` is set.
fn wait_before_scripts(config: &CliConfig) -> Result<(), CliError> {
    if !config.python_delay.is_zero() {
        std::thread::sleep(config.python_delay);
    }
    if config.python_wait {
        use std::io::Write as _;

        let mut stdout = std::io::stdout();
        let _ = stdout.flush();
        eprintln!("fft_desktop: press Enter to run the Python script(s)");
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).map_err(|error| {
            CliError::io(format!(
                "could not read stdin while waiting for Enter: {error}"
            ))
        })?;
    }
    Ok(())
}

fn run_scripts(
    host: &Arc<AppHost>,
    scripts: &[std::path::PathBuf],
    runner: &dyn ScriptComposition,
) -> Result<(), CliError> {
    if scripts.is_empty() {
        return Ok(());
    }

    // One lease for the Python adapter lifetime. Sequential runs share the
    // same scoped handle; only the root mints it, and only once per adapter.
    let lease: Arc<dyn AppLease> = host.lease();
    for path in scripts {
        runner.run(Arc::clone(&lease), path.as_path())?;
    }
    Ok(())
}

/// Delete only app-owned config files in an explicitly supplied directory.
///
/// `--fresh` never removes a directory and never recurses. It targets:
/// `settings.toml`, `session.json`, and direct `themes/*.toml` files.
/// Symlinks are never followed or removed.
fn clear_config_files(path: &Path) -> Result<(), CliError> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            eprintln!(
                "fft_desktop: config directory {} does not exist; nothing to clear",
                path.display()
            );
            return Ok(());
        }
        Err(error) => {
            return Err(CliError::io(format!(
                "could not inspect config directory {}: {error}",
                path.display()
            )))
        }
    };
    if metadata.file_type().is_symlink() {
        return Err(CliError::usage(format!(
            "--fresh refuses to use symlink {}",
            path.display()
        )));
    }
    if !metadata.is_dir() {
        return Err(CliError::usage(format!(
            "--fresh requires a directory, got {}",
            path.display()
        )));
    }

    let canonical = path.canonicalize().map_err(|error| {
        CliError::io(format!(
            "could not resolve config directory {}: {error}",
            path.display()
        ))
    })?;
    if let Some(reason) = unsafe_fresh_target(&canonical) {
        return Err(CliError::usage(format!(
            "--fresh refuses to use {}: {reason}",
            path.display()
        )));
    }

    let mut files = Vec::new();
    for name in ["settings.toml", "session.json"] {
        if let Some(file) = regular_file(&canonical.join(name))? {
            files.push(file);
        }
    }

    let themes = canonical.join("themes");
    match std::fs::symlink_metadata(&themes) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            eprintln!(
                "fft_desktop: skipping symlinked themes directory {}",
                themes.display()
            );
        }
        Ok(metadata) if metadata.is_dir() => {
            let entries = std::fs::read_dir(&themes).map_err(|error| {
                CliError::io(format!(
                    "could not read themes directory {}: {error}",
                    themes.display()
                ))
            })?;
            for entry in entries {
                let entry = entry.map_err(|error| {
                    CliError::io(format!(
                        "could not read themes directory {}: {error}",
                        themes.display()
                    ))
                })?;
                let file = entry.path();
                if file.extension().and_then(|extension| extension.to_str()) != Some("toml") {
                    continue;
                }
                if let Some(file) = regular_file(&file)? {
                    files.push(file);
                }
            }
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(CliError::io(format!(
                "could not inspect themes directory {}: {error}",
                themes.display()
            )))
        }
    }

    for file in &files {
        std::fs::remove_file(file).map_err(|error| {
            CliError::io(format!(
                "could not remove config file {}: {error}",
                file.display()
            ))
        })?;
    }

    if files.is_empty() {
        eprintln!(
            "fft_desktop: no config files to clear in {}",
            path.display()
        );
    } else {
        eprintln!(
            "fft_desktop: cleared {} config file(s) in {}",
            files.len(),
            path.display()
        );
    }
    Ok(())
}

/// Return a regular file candidate, or skip anything else.
///
/// Directories, symlinks, and missing paths are not config files this command
/// may remove.
fn regular_file(path: &Path) -> Result<Option<PathBuf>, CliError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            eprintln!("fft_desktop: skipping symlink {}", path.display());
            Ok(None)
        }
        Ok(metadata) if metadata.is_file() => Ok(Some(path.to_path_buf())),
        Ok(_) => Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(CliError::io(format!(
            "could not inspect config file {}: {error}",
            path.display()
        ))),
    }
}

fn unsafe_fresh_target(path: &Path) -> Option<&'static str> {
    if path.parent().is_none() || path.ancestors().count() < 3 {
        return Some("top-level directories are not allowed");
    }
    if let Some(home) = home_dir() {
        if path == home {
            return Some("the home directory is not allowed");
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        if let Ok(cwd) = cwd.canonicalize() {
            if cwd == path || cwd.starts_with(path) {
                return Some("the current working directory is inside this path");
            }
        }
    }
    None
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .and_then(|path| path.canonicalize().ok())
}

#[cfg(any(feature = "tauri", feature = "tauri_python"))]
struct RootHost {
    host: Arc<AppHost>,
}

#[cfg(any(feature = "tauri", feature = "tauri_python"))]
impl RootHost {
    fn shutdown(&self) {
        self.host.shutdown();
    }
}

#[cfg(any(feature = "tauri", feature = "tauri_python"))]
fn setup_error(message: impl Into<String>) -> Box<dyn std::error::Error> {
    Box::new(std::io::Error::new(
        std::io::ErrorKind::Other,
        message.into(),
    ))
}

/// Run the Tauri GUI. The root owns the host and mints both the GUI and the
/// optional scripting lease; the adapter only borrows them.
#[cfg(any(feature = "tauri", feature = "tauri_python"))]
pub fn run_gui(config: CliConfig) -> Result<(), CliError> {
    use fft_tauri_adapter::{DisplayHostOps, TauriAdapter, TauriOptions};
    use tauri::Manager as _;

    #[cfg(feature = "tauri_python")]
    use fft_tauri_adapter::TauriScripting;

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init())
        .setup(move |app| {
            let config_dir = if config.config_dir_explicit {
                config.config_dir.clone()
            } else {
                app.path().app_config_dir().map_err(|error| {
                    setup_error(format!(
                        "could not locate the desktop config directory: {error}"
                    ))
                })?
            };

            if config.fresh {
                clear_config_files(&config_dir).map_err(|error| setup_error(error.to_string()))?;
            }

            let persisted = SessionDocument::load_json(&config_dir.join("session.json"))
                .ok()
                .flatten();
            let host = build_host(&config, AppMode::Desktop, persisted);

            let gui_lease = host.lease();
            let ops: Arc<dyn DisplayHostOps> = Arc::clone(&host) as Arc<dyn DisplayHostOps>;

            #[cfg(feature = "tauri_python")]
            let scripting = {
                let lease = host.lease();
                let engine: Arc<dyn fft_script::FftScript> =
                    Arc::new(fft_python_adapter::PythonScript::new());
                Some(TauriScripting { lease, engine })
            };
            #[cfg(not(feature = "tauri_python"))]
            let scripting = None;

            let adapter = TauriAdapter::new(
                app.handle().clone(),
                gui_lease,
                ops,
                TauriOptions::new(config_dir),
                scripting,
            );
            adapter.manage(app.handle());
            app.manage(RootHost { host });
            #[cfg(target_os = "linux")]
            enable_linux_microphone(app.handle());
            Ok(())
        });

    let app = fft_tauri_adapter::install_commands(builder)
        .build(tauri::generate_context!())
        .map_err(|error| CliError::app(format!("could not build the desktop app: {error}")))?;

    app.run(|handle, event| {
        if matches!(event, tauri::RunEvent::Exit) {
            if let Some(adapter) = handle.try_state::<TauriAdapter>() {
                adapter.shutdown();
            }
            if let Some(root) = handle.try_state::<RootHost>() {
                root.shutdown();
            }
        }
    });
    Ok(())
}

/// Enable microphone capture in the Linux WebView.
///
/// WebKitGTK disables media streams and denies `getUserMedia` requests unless
/// the webview enables them and a permission handler allows the audio device.
/// Without this the Recorder gets `NotAllowedError` without any prompt.
#[cfg(all(target_os = "linux", any(feature = "tauri", feature = "tauri_python")))]
fn enable_linux_microphone(app: &tauri::AppHandle) {
    use tauri::Manager as _;
    use webkit2gtk::glib::object::Cast as _;
    use webkit2gtk::{
        PermissionRequestExt, SettingsExt, UserMediaPermissionRequest,
        UserMediaPermissionRequestExt, WebViewExt,
    };

    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    if let Err(error) = window.with_webview(|webview| {
        let webview = webview.inner();
        if let Some(settings) = webview.settings() {
            settings.set_enable_media_stream(true);
        }
        let _ = webview.connect_permission_request(|_, request| {
            let Some(media) = request.downcast_ref::<UserMediaPermissionRequest>() else {
                return false;
            };
            if media.is_for_audio_device() && !media.is_for_video_device() {
                media.allow();
                return true;
            }
            false
        });
    }) {
        eprintln!("could not configure the desktop microphone: {error}");
    }
}

const CLI_APP_NAME: &str = "fft_desktop";

fn build_host(
    config: &CliConfig,
    mode: AppMode,
    persisted_session: Option<SessionDocument>,
) -> Arc<AppHost> {
    AppHost::new(AppHostConfig {
        name: CLI_APP_NAME.to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        mode,
        capabilities: AppCapabilities::for_build(),
        workers: config.workers,
        persisted_session,
    })
}
