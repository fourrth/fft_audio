//! Generic HTTP/WebSocket adapter for an [`FftApp`](fft_app::FftApp) lease.
//!
//! The adapter receives one scoped [`AppLease`](fft_app::AppLease) when the
//! server starts, holds it for the server lifetime, and borrows the leased app
//! from request handlers. It never constructs an app implementation, never
//! constructs a runtime, and never mints leases.
//!
//! The transport intentionally exposes no scripting endpoints: scripts run
//! only through the local CLI/Python adapter. Script events are owned by the
//! scripting layer and are not part of [`AppEvent`](fft_app::AppEvent).
//!
//! ```no_run
//! use std::sync::Arc;
//!
//! use fft_app::AppLease;
//! use fft_http_adapter::{HttpAdapter, HttpAdapterConfig};
//!
//! # async fn run(lease: Arc<dyn AppLease>, config: HttpAdapterConfig) -> Result<(), Box<dyn std::error::Error>> {
//! let adapter = HttpAdapter::new(config, lease)?;
//! let server = adapter.serve().await?;
//! println!("listening on http://{}", server.addr());
//! server.wait().await?;
//! # Ok(())
//! # }
//! ```

mod adapter;
mod events;
mod render;
mod server;

#[cfg(test)]
mod tests;

pub use adapter::{HttpAdapter, HttpAdapterError, HttpServerHandle};
pub use server::HttpAdapterConfig;
