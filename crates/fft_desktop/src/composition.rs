//! Private composition seams between the CLI root owner and optional adapters.
//!
//! The CLI owns the app host and mints every lease. Adapters only receive an
//! `Arc<dyn AppLease>`; these traits intentionally expose no host, runtime, or
//! minting capability. The concrete feature-gated implementations live in
//! [`crate::http`] and [`crate::scripting`].

use std::path::Path;
use std::sync::Arc;

use fft_app::AppLease;

use crate::{CliConfig, CliError};

/// A started server. Holds its lease until stopped.
pub(crate) trait ServerSession {
    /// Block until the server ends (Ctrl-C, listener failure, ...).
    fn wait(&mut self) -> Result<(), CliError>;

    /// Stop accepting work and drop the server-lifetime lease.
    fn stop(&mut self) -> Result<(), CliError>;
}

/// Starts the optional WebUI adapter with one server-lifetime lease.
pub(crate) trait ServeComposition: Send + Sync {
    fn start(
        &self,
        lease: Arc<dyn AppLease>,
        config: &CliConfig,
    ) -> Result<Box<dyn ServerSession>, CliError>;
}

/// Runs one script on the Python adapter's lease.
///
/// The root mints one lease for the adapter and passes a clone to each
/// sequential run. Implementations must return only after the run completed
/// (or failed) so the CLI can run scripts sequentially and stop on the first
/// failure. They never mint or drop the lease themselves.
pub(crate) trait ScriptComposition: Send + Sync {
    fn run(&self, lease: Arc<dyn AppLease>, path: &Path) -> Result<(), CliError>;
}
