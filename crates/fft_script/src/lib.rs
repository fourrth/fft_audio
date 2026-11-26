//! Language-neutral scripting contract over [`fft_app::FftApp`].
//!
//! A script engine is a service hosted by an app; it receives a scoped
//! [`AppLease`] for each run and uses the leased app's
//! runtime for data/compute. Products may share one adapter-lifetime lease
//! across sequential runs. The engine must not own a second scheduler, display
//! tree, or HTTP server.
//!
//! Concrete engines live in their adapter crates; this crate is the contract,
//! event types, and shared test doubles.

pub mod error;
pub mod events;
pub mod testing;

use std::path::PathBuf;
use std::sync::Arc;

use fft_app::AppLease;

pub use error::ScriptError;
pub use events::{ScriptEvent, ScriptEventSink, ScriptStream};
pub use fft_runtime::ScriptRunId;
pub use testing::{CapturingScriptEvents, MockScript, MockScriptRecord};

#[cfg(test)]
mod tests;

/// Where a script comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScriptSource {
    File(PathBuf),
    Inline {
        /// Display/file name used in logs and errors.
        name: String,
        source: String,
    },
}

impl ScriptSource {
    pub fn name(&self) -> &str {
        match self {
            ScriptSource::File(path) => path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("script"),
            ScriptSource::Inline { name, .. } => name,
        }
    }

    /// Inline text, when available. File sources are read by the concrete
    /// language engine so it can report IO errors in its own category.
    pub fn inline_source(&self) -> Option<&str> {
        match self {
            ScriptSource::File(_) => None,
            ScriptSource::Inline { source, .. } => Some(source),
        }
    }
}

/// One scripting language binding.
///
/// Rules:
/// - implementors are services, not `FftApp`s;
/// - the app may own the engine, so hold only a per-run `Arc`/`Weak` to avoid
///   a cycle;
/// - `run` returns immediately with a typed run id; output/exit flow through
///   the supplied sink;
/// - no HTTP execution path exists.
pub trait FftScript: Send + Sync {
    fn run(
        &self,
        lease: Arc<dyn AppLease>,
        source: ScriptSource,
        events: Arc<dyn ScriptEventSink>,
    ) -> std::result::Result<ScriptRunId, ScriptError>;

    fn cancel(&self, run: ScriptRunId);
}
