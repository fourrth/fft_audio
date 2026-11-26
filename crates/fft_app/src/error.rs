//! Typed app/display errors with stable transport categories.

use fft_runtime::RuntimeError;

/// Error returned by [`crate::FftApp`] operations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppError {
    InvalidInput {
        message: String,
        context: Option<String>,
    },
    NotFound {
        message: String,
        context: Option<String>,
    },
    Busy {
        message: String,
        context: Option<String>,
    },
    Closed {
        message: String,
        context: Option<String>,
    },
    Unsupported {
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
        impl AppError {
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
    busy => Busy, "busy",
    closed => Closed, "closed",
    unsupported => Unsupported, "unsupported",
    internal => Internal, "internal",
}

impl AppError {
    /// Translate a runtime failure into the app's transport categories.
    pub fn from_runtime(error: RuntimeError) -> Self {
        let message = error.to_string();
        match error.code() {
            "invalid_input" => AppError::invalid_input(error.message()),
            "not_found" => AppError::not_found(error.message()),
            "busy" => AppError::busy(message),
            // Closed/cancelled are lifecycle-valued for transports: a display
            // waiting on such an entity should refresh or close.
            "closed" | "cancelled" => AppError::closed(message),
            "unsupported" => AppError::unsupported(error.message()),
            _ => AppError::internal(message),
        }
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.context() {
            Some(context) => write!(f, "{}: {} ({context})", self.code(), self.message()),
            None => write!(f, "{}: {}", self.code(), self.message()),
        }
    }
}

impl std::error::Error for AppError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_error_categories_are_stable() {
        for (error, code) in [
            (AppError::invalid_input("bad"), "invalid_input"),
            (AppError::not_found("missing"), "not_found"),
            (AppError::busy("later"), "busy"),
            (AppError::closed("gone"), "closed"),
            (AppError::unsupported("no"), "unsupported"),
            (AppError::internal("oops"), "internal"),
        ] {
            assert_eq!(error.code(), code);
            assert_eq!(error.to_string(), format!("{code}: {}", error.message()));
        }
        let error = AppError::not_found("display missing").with_context("display 3");
        assert_eq!(error.to_string(), "not_found: display missing (display 3)");
    }
}
