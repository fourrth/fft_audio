//! Tauri IPC adapter for an [`fft_app::FftApp`] host in GUI mode.
//!
//! The product root owns the concrete app host and mints the leases.
//! [`TauriAdapter`] borrows one GUI lifetime lease and, when scripting is
//! enabled, a separate scripting lease plus an injected engine. It never mints
//! a lease or shuts down the host.
//!
//! Normal GUI operation uses Tauri IPC exclusively. This crate has no HTTP
//! dependency and no HTTP listener/route surface.
//!
//! The transport shapes and names are the shared desktop IPC contract:
//! [`COMMAND_NAMES`] and [`EVENT_NAMES`] list the command/event surface, and
//! the DTOs use camelCase fields.

#![allow(unused_crate_dependencies)] // tauri-plugin-fs is composed by the shell

mod adapter;
mod commands;
mod core;
mod dto;
mod events;
mod paths;

/// Examples embedded at compile time by `build.rs`.
mod embedded_examples {
    include!(concat!(env!("OUT_DIR"), "/embedded_examples.rs"));
}

#[cfg(test)]
mod tests;

pub use adapter::{TauriAdapter, TauriOptions, TauriScripting};
pub use commands::{install_commands, shutdown, COMMAND_NAMES};
pub use core::DisplayHostOps;
pub use dto::{
    scripting_capability, AnalysisDoneEvent, AnalysisErrorEvent, AnalysisProgressEvent,
    AppCapabilitiesInfo, AppInfo, CancelResult, ConfigPaths, CutsPayload, DisplayClosedEvent,
    DisplayEventPayload, FileStat, OpenFileResult, OpenResult, PingResult, ScriptExampleInfo,
    ScriptExitEvent, ScriptOutputEvent, ScriptPathResult, SpectrogramMeta, SpectrogramPayload,
};
pub use events::{APP_EVENT_NAMES, EVENT_NAMES, SCRIPT_EVENT_NAMES};
