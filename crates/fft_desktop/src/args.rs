//! CLI configuration, clap argument definitions, and mode validation.
//!
//! The product is described once in [`CliArgs`]. Adapter arguments are compiled
//! only when their feature is enabled, so `--help` and parsing always match the
//! build. Feature-off adapter arguments are unknown arguments; programmatic
//! `CliConfig` values are still checked by [`validate_config`].
//!
//! With a GUI feature enabled, no CLI action launches the GUI. `--headless`
//! suppresses the GUI and runs the CLI composition; `--serve` and
//! `--python-script` select CLI adapters explicitly. Without any GUI feature or
//! adapter action the product is inert.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;

use crate::error::{CliError, HTTP_DISABLED_SERVE, PYTHON_DISABLED};

/// Default bind address used when `--bind`/`--port` are absent. The WebUI is
/// loopback-only by default; users opt in to wider exposure with `--bind`.
pub const DEFAULT_BIND: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8765);

crate::feature_cli_args! {
    #[command(
        name = "fft_desktop",
        version,
        about = "fft_audio desktop application and adapter root",
        long_about = None,
    )]
    common {
        /// Runtime worker threads.
        #[arg(
            long,
            value_name = "N",
            default_value_t = fft_runtime::default_worker_count(),
            value_parser = parse_workers,
        )]
        workers: usize,

        /// Configuration/settings/session directory.
        #[arg(long, value_name = "PATH")]
        config_dir: Option<PathBuf>,

        /// Do not launch the GUI; run the CLI composition.
        #[arg(long)]
        headless: bool,

        /// Reset settings, session, and user themes in the config directory.
        #[arg(long)]
        fresh: bool,
    }
    features(http: "http") {
        /// Serve the WebUI until Ctrl-C.
        #[arg(long)]
        serve: bool,

        /// IP or IP:PORT to bind (default 127.0.0.1).
        #[arg(long, value_name = "ADDR", requires = "serve")]
        bind: Option<String>,

        /// Override the WebUI port.
        #[arg(long, value_name = "PORT", requires = "serve")]
        port: Option<u16>,

        /// Built frontend directory.
        #[arg(long, value_name = "PATH", requires = "serve")]
        dist: Option<PathBuf>,
    }
    features(python: "python") {
        /// Local Python script to run; repeat for sequential runs.
        #[arg(long = "python-script", value_name = "PATH")]
        python_scripts: Vec<PathBuf>,

        /// Stop the WebUI after the last script.
        #[arg(long, requires = "python_scripts")]
        exit_after_script: bool,

        /// Sleep this many seconds before the first script.
        #[arg(
            long = "python-delay",
            value_name = "SECONDS",
            requires = "python_scripts",
            value_parser = parse_delay,
        )]
        python_delay: Option<f64>,

        /// Wait for Enter before running the first script.
        #[arg(long = "python-wait", requires = "python_scripts")]
        python_wait: bool,
    }
}

impl CliArgs {
    fn into_config(self) -> Result<CliConfig, CliError> {
        let mut config = CliConfig {
            workers: self.common.workers,
            ..CliConfig::default()
        };

        if let Some(config_dir) = self.common.config_dir {
            config.config_dir = config_dir;
            config.config_dir_explicit = true;
        }
        config.headless = self.common.headless;
        config.fresh = self.common.fresh;

        #[cfg(feature = "http")]
        {
            config.serve = self.http.serve;
            if let Some(dist) = self.http.dist {
                config.dist = dist;
            }

            let port = self.http.port.unwrap_or(config.bind.port());
            let mut bind = match self.http.bind {
                Some(value) => parse_bind(&value, port)?,
                None => config.bind,
            };
            if let Some(port) = self.http.port {
                bind.set_port(port);
            }
            config.bind = bind;
        }

        #[cfg(feature = "python")]
        {
            config.python_scripts = self.python.python_scripts;
            config.exit_after_script = self.python.exit_after_script;
            if let Some(seconds) = self.python.python_delay {
                config.python_delay = Duration::from_secs_f64(seconds);
            }
            config.python_wait = self.python.python_wait;
        }

        Ok(config)
    }
}

/// Configuration for one product run.
///
/// The product is the root leaser: it creates the app host, then mints one
/// lease for each adapter (the WebUI server and the Python adapter).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CliConfig {
    /// Compute workers passed to the app host runtime.
    pub workers: usize,
    /// Explicitly serve the WebUI.
    pub serve: bool,
    /// Local Python scripts, run sequentially in one app.
    pub python_scripts: Vec<PathBuf>,
    /// Stop a started WebUI after the last script.
    pub exit_after_script: bool,
    /// Delay after adapter startup and before the first script.
    pub python_delay: Duration,
    /// Wait for Enter after adapter startup and before the first script.
    pub python_wait: bool,
    /// WebUI bind address. The default is loopback; any local address is
    /// accepted so the operator decides where the server is reachable.
    pub bind: SocketAddr,
    /// Built frontend directory.
    pub dist: PathBuf,
    /// Configuration/settings/session directory.
    pub config_dir: PathBuf,
    /// Whether `--config-dir` was supplied explicitly.
    pub config_dir_explicit: bool,
    /// Do not launch the GUI; run the CLI composition.
    pub headless: bool,
    /// Delete app-owned config files in the config directory.
    pub fresh: bool,
}

impl Default for CliConfig {
    fn default() -> Self {
        Self {
            workers: fft_runtime::default_worker_count(),
            serve: false,
            python_scripts: Vec::new(),
            exit_after_script: false,
            python_delay: Duration::ZERO,
            python_wait: false,
            bind: DEFAULT_BIND,
            dist: PathBuf::from("web/dist"),
            config_dir: default_config_dir(),
            config_dir_explicit: false,
            headless: false,
            fresh: false,
        }
    }
}

impl CliConfig {
    /// Whether this run selects no adapter at all.
    ///
    /// An inert run still owns and shuts down the app host, but opens no
    /// listener and executes no script.
    pub fn inert(&self) -> bool {
        !self.serve && self.python_scripts.is_empty()
    }

    /// Whether this run only executes local scripts (no listener).
    pub fn script_only(&self) -> bool {
        !self.serve && !self.python_scripts.is_empty()
    }

    /// Whether this run should launch a GUI when the build has a GUI feature.
    ///
    /// Explicit CLI work (`--serve`, scripts, or `--headless`) selects the CLI
    /// composition; otherwise a Tauri build starts the GUI.
    pub fn wants_gui(&self) -> bool {
        cfg!(any(feature = "tauri", feature = "tauri_python"))
            && !self.headless
            && !self.serve
            && self.python_scripts.is_empty()
    }
}

/// Parse command-line flags into the product configuration.
///
/// `args` must contain flags/values only (typically `std::env::args().skip(1)`).
/// `--help`/`--version` are reported as [`CliError`] with exit code `0`; parser
/// errors are usage errors with exit code `2`. Feature availability is
/// intentionally checked by [`validate_config`] before an adapter starts.
/// Arguments belonging to a disabled adapter feature are not compiled in and
/// are therefore rejected by clap as unknown.
pub fn parse_args(args: impl IntoIterator<Item = String>) -> Result<CliConfig, CliError> {
    let parsed = CliArgs::try_parse_from(std::iter::once("fft_desktop".to_string()).chain(args))
        .map_err(CliError::from_clap)?;
    parsed.into_config()
}

/// Validate a configuration before the app host is built.
///
/// This rejects structurally invalid configs (including programmatic ones) and
/// reports a clear rebuild-with-feature error when the requested mode needs a
/// feature that is not compiled in. No host, listener, or lease exists yet.
pub fn validate_config(config: &CliConfig) -> Result<(), CliError> {
    if config.workers == 0 {
        return Err(CliError::usage("--workers must be at least 1"));
    }
    if config.exit_after_script && config.python_scripts.is_empty() {
        return Err(CliError::usage(
            "--exit-after-script requires at least one --python-script",
        ));
    }
    if config
        .python_scripts
        .iter()
        .any(|path| path.as_os_str().is_empty())
    {
        return Err(CliError::usage("--python-script needs a non-empty path"));
    }

    if config.serve && !cfg!(feature = "http") {
        return Err(CliError::unsupported_feature("http", HTTP_DISABLED_SERVE));
    }
    if !config.python_scripts.is_empty() && !cfg!(feature = "python") {
        return Err(CliError::unsupported_feature("python", PYTHON_DISABLED));
    }

    Ok(())
}

#[cfg(feature = "python")]
fn parse_delay(value: &str) -> Result<f64, String> {
    let seconds = value
        .parse::<f64>()
        .map_err(|_| format!("--python-delay needs seconds, got `{value}`"))?;
    if !seconds.is_finite() || seconds < 0.0 {
        return Err("--python-delay must be a non-negative number of seconds".to_string());
    }
    Ok(seconds)
}

fn parse_workers(value: &str) -> Result<usize, String> {
    let workers = value
        .parse::<usize>()
        .map_err(|_| format!("--workers needs a whole number, got `{value}`"))?;
    if workers == 0 {
        return Err("--workers must be at least 1".to_string());
    }
    Ok(workers)
}

#[cfg(feature = "http")]
fn parse_bind(value: &str, current_port: u16) -> Result<SocketAddr, CliError> {
    if let Ok(address) = value.parse::<SocketAddr>() {
        return Ok(address);
    }
    if let Ok(ip) = value.parse::<IpAddr>() {
        return Ok(SocketAddr::new(ip, current_port));
    }
    Err(CliError::usage(format!(
        "--bind needs an IP or `IP:PORT` address, got `{value}`"
    )))
}

fn default_config_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("FFT_AUDIO_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(dir).join("fft_audio");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".config").join("fft_audio");
    }
    PathBuf::from(".fft_audio")
}
