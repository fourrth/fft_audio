//! Optional WebUI composition (`http` feature).
//!
//! The product mints exactly one lease for the server lifetime and hands it to
//! the adapter. This module only bridges the synchronous CLI composition to the
//! adapter's async server lifecycle; it never owns the app host or mints another
//! lease.

use std::sync::Arc;

use fft_app::AppLease;

#[cfg(feature = "http")]
use std::time::Duration;

#[cfg(feature = "http")]
use fft_http_adapter::{HttpAdapter, HttpAdapterConfig, HttpServerHandle};

use crate::composition::{ServeComposition, ServerSession};
use crate::{CliConfig, CliError};

/// Feature-gated WebUI composition used by [`crate::DesktopApp`].
pub(crate) struct HttpComposition;

impl ServeComposition for HttpComposition {
    fn start(
        &self,
        lease: Arc<dyn AppLease>,
        config: &CliConfig,
    ) -> Result<Box<dyn ServerSession>, CliError> {
        start(lease, config)
    }
}

#[cfg(feature = "http")]
fn start(lease: Arc<dyn AppLease>, config: &CliConfig) -> Result<Box<dyn ServerSession>, CliError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| CliError::io(format!("could not start the async runtime: {error}")))?;

    let adapter_config =
        HttpAdapterConfig::new(config.bind, config.dist.clone(), config.config_dir.clone());
    let adapter = HttpAdapter::new(adapter_config, lease)
        .map_err(|error| CliError::io(format!("could not initialize the WebUI: {error}")))?;
    let handle = runtime
        .block_on(adapter.serve())
        .map_err(|error| CliError::io(format!("could not start the WebUI: {error}")))?;

    let addr = handle.addr();
    if addr.ip().is_unspecified() {
        println!("fft_desktop WebUI: http://{addr} (all interfaces)");
    } else {
        println!("fft_desktop WebUI: http://{addr}");
    }
    if !addr.ip().is_loopback() {
        eprintln!(
            "warning: the WebUI has no authentication; binding beyond loopback exposes it to the network"
        );
    }

    Ok(Box::new(HttpServerSession {
        runtime: Some(runtime),
        handle: Some(handle),
    }))
}

#[cfg(not(feature = "http"))]
fn start(
    _lease: Arc<dyn AppLease>,
    _config: &CliConfig,
) -> Result<Box<dyn ServerSession>, CliError> {
    // Validation runs before any lease is minted; this is a defensive fallback.
    Err(CliError::unsupported_feature(
        "http",
        crate::error::HTTP_DISABLED,
    ))
}

/// A running server plus the runtime that drives its task.
///
/// `wait` blocks until Ctrl-C, matching the CLI server mode. `stop`
/// performs the adapter's graceful shutdown and then drops the runtime, which
/// releases the server-lifetime lease.
#[cfg(feature = "http")]
struct HttpServerSession {
    runtime: Option<tokio::runtime::Runtime>,
    handle: Option<HttpServerHandle>,
}

#[cfg(feature = "http")]
impl ServerSession for HttpServerSession {
    fn wait(&mut self) -> Result<(), CliError> {
        let Some(runtime) = self.runtime.as_ref() else {
            return Ok(());
        };
        if self.handle.is_none() {
            return Ok(());
        }
        runtime.block_on(async {
            tokio::signal::ctrl_c()
                .await
                .map_err(|error| CliError::io(format!("could not listen for Ctrl-C: {error}")))
        })
    }

    fn stop(&mut self) -> Result<(), CliError> {
        let result = match (self.handle.take(), self.runtime.as_ref()) {
            (Some(handle), Some(runtime)) => runtime
                .block_on(handle.stop())
                .map_err(|error| CliError::app(format!("WebUI shutdown failed: {error}"))),
            _ => Ok(()),
        };

        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_timeout(Duration::from_secs(5));
        }
        result
    }
}

#[cfg(feature = "http")]
impl Drop for HttpServerSession {
    fn drop(&mut self) {
        // Idempotent; also covers a session dropped without an explicit stop.
        let _ = self.stop();
    }
}
