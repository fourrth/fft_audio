//! Typed CLI errors and process exit codes.
//!
//! The library never calls `std::process::exit`; the `fft_desktop` binary
//! prints [`CliError`] and exits with [`CliError::exit_code`]. Usage/feature
//! errors use exit code `2`; adapter and app failures use `1`; help is not a
//! failure (`0`).

use std::fmt;

/// Category of a CLI failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CliErrorKind {
    /// Invalid command line or programmatic configuration.
    Usage,
    /// The requested mode needs a feature that is not compiled in.
    UnsupportedFeature,
    /// The optional adapter is compiled in but its composition is not wired yet.
    AdapterNotComposed,
    /// App/script/server failure.
    App,
    /// Filesystem or other I/O failure.
    Io,
    /// `--help`/`--version` was requested; this is not a failure.
    Help,
}

impl CliErrorKind {
    /// Stable, machine-readable category.
    pub const fn code(self) -> &'static str {
        match self {
            CliErrorKind::Usage => "usage",
            CliErrorKind::UnsupportedFeature => "unsupported_feature",
            CliErrorKind::AdapterNotComposed => "adapter_not_composed",
            CliErrorKind::App => "app",
            CliErrorKind::Io => "io",
            CliErrorKind::Help => "help",
        }
    }
}

/// A CLI parse/validation/run error with a stable code and exit status.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CliError {
    kind: CliErrorKind,
    code: &'static str,
    message: String,
    exit_code: i32,
    feature: Option<&'static str>,
    /// The message is already a complete clap rendering (including `error:` or
    /// the full help text) and should not be prefixed again by the binary.
    rendered: bool,
}

impl CliError {
    fn new(
        kind: CliErrorKind,
        code: &'static str,
        message: impl Into<String>,
        exit_code: i32,
    ) -> Self {
        Self {
            kind,
            code,
            message: message.into(),
            exit_code,
            feature: None,
            rendered: false,
        }
    }

    /// Map a clap parse error to the product error type.
    ///
    /// Clap renders help/version and usage text itself, so the message is kept
    /// verbatim and marked as rendered. Help/version exit `0`; parser errors
    /// exit `2`.
    pub(crate) fn from_clap(error: clap::Error) -> Self {
        let (kind, code, exit_code) = match error.kind() {
            clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion => {
                (CliErrorKind::Help, "help", 0)
            }
            _ => (CliErrorKind::Usage, "usage", 2),
        };
        let mut mapped = Self::new(kind, code, error.to_string(), exit_code);
        mapped.rendered = true;
        mapped
    }

    /// Invalid command line or configuration (exit code `2`).
    pub fn usage(message: impl Into<String>) -> Self {
        Self::new(CliErrorKind::Usage, "usage", message, 2)
    }

    /// A mode requires a feature that is not compiled in (exit code `2`).
    pub fn unsupported_feature(feature: &'static str, message: impl Into<String>) -> Self {
        let mut error = Self::new(
            CliErrorKind::UnsupportedFeature,
            "unsupported_feature",
            message,
            2,
        );
        error.feature = Some(feature);
        error
    }

    /// The adapter is compiled in but its real composition is not available yet.
    pub fn adapter_not_composed(adapter: impl AsRef<str>) -> Self {
        Self::new(
            CliErrorKind::AdapterNotComposed,
            "adapter_not_composed",
            format!("adapter not composed yet: {}", adapter.as_ref()),
            1,
        )
    }

    /// App/script/server failure (exit code `1`).
    pub fn app(message: impl Into<String>) -> Self {
        Self::new(CliErrorKind::App, "app", message, 1)
    }

    /// Filesystem or other I/O failure (exit code `1`).
    pub fn io(message: impl Into<String>) -> Self {
        Self::new(CliErrorKind::Io, "io", message, 1)
    }

    pub fn kind(&self) -> CliErrorKind {
        self.kind
    }

    /// Stable error code (`usage`, `unsupported_feature`, ...).
    pub fn code(&self) -> &'static str {
        self.code
    }

    /// Feature named by an [`CliErrorKind::UnsupportedFeature`] error.
    pub fn feature(&self) -> Option<&'static str> {
        self.feature
    }

    /// Human-readable message (the full help text for `--help`).
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Whether the message is a complete clap rendering that must be printed
    /// verbatim rather than prefixed with the binary name.
    pub fn is_rendered(&self) -> bool {
        self.rendered
    }

    /// Process exit status the product wrapper should use.
    pub fn exit_code(&self) -> i32 {
        self.exit_code
    }

    pub fn is_help(&self) -> bool {
        self.kind == CliErrorKind::Help
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for CliError {}

#[cfg(not(feature = "http"))]
pub(crate) const HTTP_DISABLED: &str =
    "the WebUI server requires the `http` feature; rebuild with `--features http`";

pub(crate) const HTTP_DISABLED_SERVE: &str =
    "--serve requires the `http` feature; rebuild with `--features http`";

pub(crate) const PYTHON_DISABLED: &str =
    "--python-script requires the `python` feature; rebuild with `--features python`";
