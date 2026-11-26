//! Typed scripting errors with stable transport categories.

use fft_app::AppError;
use fft_runtime::RuntimeError;

/// Error returned by [`crate::FftScript`] setup/execution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScriptError {
    InvalidInput {
        message: String,
        context: Option<String>,
    },
    NotFound {
        message: String,
        context: Option<String>,
    },
    Closed {
        message: String,
        context: Option<String>,
    },
    /// The engine is still executing another run.
    Busy {
        message: String,
        context: Option<String>,
    },
    Unsupported {
        message: String,
        context: Option<String>,
    },
    Cancelled {
        message: String,
        context: Option<String>,
    },
    /// Failure reported by the app/runtime while the script was executing.
    Runtime {
        message: String,
        context: Option<String>,
    },
    Internal {
        message: String,
        context: Option<String>,
    },
}

macro_rules! constructors {
    ($( $method:ident => $variant:ident, $code:literal ),+ $(,)?) => {
        impl ScriptError {
            $(
                #[doc = concat!("Create an error with code `", $code, "`.")]
                pub fn $method(message: impl Into<String>) -> Self {
                    Self::$variant {
                        message: message.into(),
                        context: None,
                    }
                }
            )+

            /// Stable, transport-facing category code.
            pub fn code(&self) -> &'static str {
                match self {
                    $( Self::$variant { .. } => $code, )+
                }
            }

            pub fn message(&self) -> &str {
                match self {
                    $( Self::$variant { message, .. } => message, )+
                }
            }

            pub fn context(&self) -> Option<&str> {
                match self {
                    $( Self::$variant { context, .. } => context.as_deref(), )+
                }
            }

            pub fn with_context(mut self, value: impl Into<String>) -> Self {
                let context = Some(value.into());
                match &mut self {
                    $( Self::$variant { context: slot, .. } => *slot = context, )+
                }
                self
            }
        }
    };
}

constructors! {
    invalid_input => InvalidInput, "invalid_input",
    not_found => NotFound, "not_found",
    closed => Closed, "closed",
    busy => Busy, "busy",
    unsupported => Unsupported, "unsupported",
    cancelled => Cancelled, "cancelled",
    runtime => Runtime, "runtime",
    internal => Internal, "internal",
}

impl From<AppError> for ScriptError {
    fn from(error: AppError) -> Self {
        let context = error.context().map(str::to_string);
        let mapped = match error.code() {
            "invalid_input" => ScriptError::invalid_input(error.message()),
            "not_found" => ScriptError::not_found(error.message()),
            "closed" => ScriptError::closed(error.message()),
            "busy" => ScriptError::busy(error.message()),
            "unsupported" => ScriptError::unsupported(error.message()),
            _ => ScriptError::internal(error.message()),
        };
        match context {
            Some(context) => mapped.with_context(context),
            None => mapped,
        }
    }
}

impl From<RuntimeError> for ScriptError {
    fn from(error: RuntimeError) -> Self {
        let context = error.context().map(str::to_string);
        let mapped = ScriptError::runtime(error.to_string());
        match context {
            Some(context) => mapped.with_context(context),
            None => mapped,
        }
    }
}

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.context() {
            Some(context) => write!(f, "{}: {} ({context})", self.code(), self.message()),
            None => write!(f, "{}: {}", self.code(), self.message()),
        }
    }
}

impl std::error::Error for ScriptError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_error_categories_are_stable() {
        for (error, code) in [
            (ScriptError::invalid_input("x"), "invalid_input"),
            (ScriptError::not_found("x"), "not_found"),
            (ScriptError::closed("x"), "closed"),
            (ScriptError::busy("x"), "busy"),
            (ScriptError::unsupported("x"), "unsupported"),
            (ScriptError::cancelled("x"), "cancelled"),
            (ScriptError::runtime("x"), "runtime"),
            (ScriptError::internal("x"), "internal"),
        ] {
            assert_eq!(error.code(), code);
        }
    }

    #[test]
    fn app_and_runtime_errors_map_into_script_categories() {
        let app =
            ScriptError::from(AppError::not_found("display missing").with_context("display 2"));
        assert_eq!(app.code(), "not_found");
        assert_eq!(app.context(), Some("display 2"));

        let runtime = ScriptError::from(RuntimeError::busy("not ready"));
        assert_eq!(runtime.code(), "runtime");
        assert!(runtime.message().contains("not ready"));
    }
}
