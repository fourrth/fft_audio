//! Adapter construction and server lifecycle.
//!
//! The adapter is the server-lifetime owner of the one lease it receives. It
//! does not own or construct a host/runtime and exposes no lease-minting API.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use fft_app::{AppLease, LeaseId};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::server::{self, HttpAdapterConfig, HttpState};

/// Failure while constructing or running the HTTP adapter.
#[derive(Debug)]
pub enum HttpAdapterError {
    /// Local directory/file setup failed.
    Io {
        context: String,
        source: std::io::Error,
    },
    /// The listener could not bind the configured address.
    Bind {
        addr: SocketAddr,
        source: std::io::Error,
    },
    /// The server task returned an I/O error.
    Server(std::io::Error),
    /// The server task panicked or was cancelled unexpectedly.
    Task(tokio::task::JoinError),
    /// The server did not finish a graceful stop in time.
    ShutdownTimeout(Duration),
}

impl std::fmt::Display for HttpAdapterError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpAdapterError::Io { context, source } => {
                write!(formatter, "{context}: {source}")
            }
            HttpAdapterError::Bind { addr, source } => {
                write!(formatter, "could not bind {addr}: {source}")
            }
            HttpAdapterError::Server(error) => write!(formatter, "HTTP server failed: {error}"),
            HttpAdapterError::Task(error) => write!(formatter, "HTTP server task failed: {error}"),
            HttpAdapterError::ShutdownTimeout(timeout) => {
                write!(formatter, "HTTP server did not stop within {timeout:?}")
            }
        }
    }
}

impl std::error::Error for HttpAdapterError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            HttpAdapterError::Io { source, .. }
            | HttpAdapterError::Bind { source, .. }
            | HttpAdapterError::Server(source) => Some(source),
            HttpAdapterError::Task(error) => Some(error),
            HttpAdapterError::ShutdownTimeout(_) => None,
        }
    }
}

fn io_error(context: impl Into<String>, source: std::io::Error) -> HttpAdapterError {
    HttpAdapterError::Io {
        context: context.into(),
        source,
    }
}

/// HTTP/WebSocket adapter over one scoped app lease.
///
/// The lease is borrowed by every request for the lifetime of the server. The
/// adapter has no `lease()`/`try_lease()` method and never creates a host.
pub struct HttpAdapter {
    state: Arc<HttpState>,
}

impl HttpAdapter {
    /// Create the adapter and hold the lease.
    ///
    /// Fails only for local directory setup errors. The configured bind address
    /// is used as-is: callers decide whether to listen on loopback, a specific
    /// interface, or every interface. No listener is opened and no app event
    /// subscription is made until [`HttpAdapter::serve`] binds successfully.
    pub fn new(
        config: HttpAdapterConfig,
        lease: Arc<dyn AppLease>,
    ) -> Result<Self, HttpAdapterError> {
        std::fs::create_dir_all(&config.config_dir)
            .map_err(|error| io_error("could not create the config directory", error))?;
        std::fs::create_dir_all(&config.themes_dir)
            .map_err(|error| io_error("could not create the themes directory", error))?;
        std::fs::create_dir_all(&config.uploads_dir)
            .map_err(|error| io_error("could not create the uploads directory", error))?;
        Ok(Self {
            state: Arc::new(HttpState::new(config, lease)),
        })
    }

    /// The lease identity this server borrows (diagnostics only).
    pub fn lease_id(&self) -> LeaseId {
        self.state.lease.lease_id()
    }

    /// Bind the configured address and start serving.
    pub async fn serve(self) -> Result<HttpServerHandle, HttpAdapterError> {
        let addr = self.state.config.bind;
        let listener = TcpListener::bind(addr)
            .await
            .map_err(|source| HttpAdapterError::Bind { addr, source })?;
        let bound = listener
            .local_addr()
            .map_err(|error| io_error("could not read the bound listener address", error))?;
        // Subscribe after a successful bind so construction failures and
        // never-served adapters do not leave a dead sink in the app registry.
        self.state.subscribe();
        let router = server::router(Arc::clone(&self.state));
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let join = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.await;
                })
                .await
        });
        Ok(HttpServerHandle {
            addr: bound,
            shutdown: Some(shutdown_tx),
            join: Some(join),
        })
    }
}

/// Handle for a running HTTP server.
///
/// Drop signals shutdown and aborts the task. [`HttpServerHandle::stop`]
/// awaits shutdown so the server-lifetime lease is released before the call
/// returns.
pub struct HttpServerHandle {
    addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    join: Option<JoinHandle<std::io::Result<()>>>,
}

impl HttpServerHandle {
    /// The actual bound address (port 0 resolves to a concrete port).
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Alias for [`HttpServerHandle::addr`].
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Stop the server and wait for graceful completion.
    pub async fn stop(mut self) -> Result<(), HttpAdapterError> {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        let Some(mut join) = self.join.take() else {
            return Ok(());
        };
        match tokio::time::timeout(Duration::from_secs(5), &mut join).await {
            Ok(Ok(result)) => result.map_err(HttpAdapterError::Server),
            Ok(Err(error)) => Err(HttpAdapterError::Task(error)),
            Err(_) => {
                join.abort();
                let _ = join.await;
                Err(HttpAdapterError::ShutdownTimeout(Duration::from_secs(5)))
            }
        }
    }

    /// Wait for the server to finish without requesting shutdown.
    pub async fn wait(mut self) -> Result<(), HttpAdapterError> {
        let Some(join) = self.join.take() else {
            return Ok(());
        };
        match join.await {
            Ok(result) => result.map_err(HttpAdapterError::Server),
            Err(error) => Err(HttpAdapterError::Task(error)),
        }
    }
}

impl Drop for HttpServerHandle {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(join) = self.join.take() {
            join.abort();
        }
    }
}
