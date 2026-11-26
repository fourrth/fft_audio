//! Typed runtime errors with stable transport categories.
//!
//! The category (`code`) is part of the contract; messages and context are for
//! humans and may change. Adapters should branch on [`RuntimeError::code`].

/// Error returned by [`crate::FftRuntime`] operations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeError {
    /// A request was malformed or failed validation before any state changed.
    InvalidInput {
        message: String,
        context: Option<String>,
    },
    /// The requested signal/analysis/cut/job does not exist.
    NotFound {
        message: String,
        context: Option<String>,
    },
    /// The entity or runtime was already closed.
    Closed {
        message: String,
        context: Option<String>,
    },
    /// The result exists but is not materialized yet, or the runtime is busy.
    Busy {
        message: String,
        context: Option<String>,
    },
    /// Analysis or cut computation failed.
    AnalysisFailed {
        message: String,
        context: Option<String>,
    },
    /// Cooperative cancellation stopped the job.
    Cancelled {
        message: String,
        context: Option<String>,
    },
    /// Reading/writing an export failed.
    Export {
        message: String,
        context: Option<String>,
    },
    /// The operation or format is not available in this build.
    Unsupported {
        message: String,
        context: Option<String>,
    },
    /// Unexpected internal failure (poisoned state, broken invariant, ...).
    Internal {
        message: String,
        context: Option<String>,
    },
}

macro_rules! constructors {
    ($( $method:ident => $variant:ident, $code:literal ),+ $(,)?) => {
        impl RuntimeError {
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

            /// Human-readable message without the context suffix.
            pub fn message(&self) -> &str {
                match self {
                    $( Self::$variant { message, .. } => message, )+
                }
            }

            /// Optional context (entity id, path, feature name, ...).
            pub fn context(&self) -> Option<&str> {
                match self {
                    $( Self::$variant { context, .. } => context.as_deref(), )+
                }
            }

            /// Attach context without changing the category.
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
    analysis_failed => AnalysisFailed, "analysis_failed",
    cancelled => Cancelled, "cancelled",
    export => Export, "export",
    unsupported => Unsupported, "unsupported",
    internal => Internal, "internal",
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.context() {
            Some(context) => write!(f, "{}: {} ({context})", self.code(), self.message()),
            None => write!(f, "{}: {}", self.code(), self.message()),
        }
    }
}

impl std::error::Error for RuntimeError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_and_context_are_stable() {
        let error = RuntimeError::analysis_failed("FFT failed").with_context("analysis 4");
        assert_eq!(error.code(), "analysis_failed");
        assert_eq!(error.message(), "FFT failed");
        assert_eq!(error.context(), Some("analysis 4"));
        assert_eq!(
            error.to_string(),
            "analysis_failed: FFT failed (analysis 4)"
        );
        assert!(std::error::Error::source(&error).is_none());
    }

    #[test]
    fn every_category_has_its_expected_code() {
        let cases = [
            (RuntimeError::invalid_input("x"), "invalid_input"),
            (RuntimeError::not_found("x"), "not_found"),
            (RuntimeError::closed("x"), "closed"),
            (RuntimeError::busy("x"), "busy"),
            (RuntimeError::analysis_failed("x"), "analysis_failed"),
            (RuntimeError::cancelled("x"), "cancelled"),
            (RuntimeError::export("x"), "export"),
            (RuntimeError::unsupported("x"), "unsupported"),
            (RuntimeError::internal("x"), "internal"),
        ];
        for (error, code) in cases {
            assert_eq!(error.code(), code);
            assert_eq!(error.context(), None);
        }
    }
}
