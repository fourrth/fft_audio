//! App-level display/event/lifecycle contract on top of [`fft_runtime`].
//!
//! `fft_app` owns the display registry, display events, app capabilities, and
//! lifecycle. It does not load or compute data; callers use
//! [`FftApp::runtime`] for that. It also knows nothing about HTTP, Tauri,
//! Python, or Lua.
//!
//! Storage is deliberately not part of this trait. A concrete adapter-side
//! file store is preferable to an `AppStorage` trait here, keeping the app
//! contract focused on displays, events, and lifecycle.
//!
//! Leases are host-constructed: adapters receive `Arc<dyn AppLease>` and cannot
//! construct a lease or mint another one. Only the concrete app host defines
//! its own private lease type.

pub mod display;
pub mod error;
pub mod events;
pub mod lease;
pub mod registry;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use fft_core::table::TableFormat;
use fft_runtime::FftRuntime;
use serde::{Deserialize, Serialize};

pub use fft_runtime::{AnalysisId, CutId, DisplayId, SignalId};

pub use display::{
    CutDisplayOptions, CutDisplaySettings, DisplayIdAllocator, DisplayInfo, DisplayKind,
    DisplayOwner, DisplayRecord, PlotDisplayOptions, SpectrogramDisplayOptions,
    SCRIPTED_DISPLAY_ID_BASE,
};
pub use error::AppError;
pub use events::{AppEvent, AppEventSink, Subscription};
pub use lease::{AppLease, LeaseId};
pub use registry::{AppClose, DisplayRegistry};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppMode {
    Headless,
    Desktop,
}

impl AppMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            AppMode::Headless => "headless",
            AppMode::Desktop => "desktop",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub mode: AppMode,
}

/// Typed capability report for the frontend.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppCapabilities {
    pub displays: bool,
    pub table_formats: Vec<TableFormat>,
}

impl AppCapabilities {
    /// Capability report for a build: displays on and table formats filtered by
    /// compiled features.
    pub fn for_build() -> Self {
        Self {
            displays: true,
            table_formats: available_table_formats(),
        }
    }
}

/// Table format families available in this build, deduplicated by the stable
/// capability name (`csv`, `parquet`, `json`, `jsonl`, `arrow`).
pub fn available_table_formats() -> Vec<TableFormat> {
    let mut formats = Vec::new();
    for extension in fft_core::table::available_table_extensions() {
        let path = format!("data.{extension}");
        let Some(format) = TableFormat::from_path(Path::new(&path)) else {
            continue;
        };
        if !formats
            .iter()
            .any(|existing: &TableFormat| existing.kind_name() == format.kind_name())
        {
            formats.push(format);
        }
    }
    formats
}

/// App/display/lifecycle service implemented by headless and desktop hosts.
///
/// `FftApp` does not compute; use [`FftApp::runtime`]. Displays are app-level
/// records, not runtime data handles.
pub trait FftApp: Send + Sync {
    fn info(&self) -> AppInfo;
    fn capabilities(&self) -> AppCapabilities;
    fn runtime(&self) -> Arc<dyn FftRuntime>;

    fn displays(&self) -> std::result::Result<Vec<DisplayInfo>, AppError>;
    fn display(&self, id: DisplayId) -> std::result::Result<DisplayInfo, AppError>;

    fn show_spectrogram(
        &self,
        analysis: AnalysisId,
        options: &SpectrogramDisplayOptions,
    ) -> std::result::Result<DisplayId, AppError>;

    fn show_order_cut(
        &self,
        parent: DisplayId,
        cut: CutId,
        options: &CutDisplayOptions,
    ) -> std::result::Result<DisplayId, AppError>;

    fn show_line_cut(
        &self,
        parent: DisplayId,
        cut: CutId,
        options: &CutDisplayOptions,
    ) -> std::result::Result<DisplayId, AppError>;

    /// Register a standalone generic Plot display for a table file.
    ///
    /// Plot displays are not tied to an analysis; their state keeps the table
    /// path and column layout so transports can load the series on demand.
    fn show_plot(&self, options: &PlotDisplayOptions) -> std::result::Result<DisplayId, AppError>;

    /// Register a spectrogram display immediately while its analysis is still
    /// pending. Used by scripted/backend opens; unlike [`FftApp::show_spectrogram`]
    /// this does not call `spectrogram_data` first.
    fn show_spectrogram_pending(
        &self,
        analysis: AnalysisId,
        signal: Option<SignalId>,
        options: &SpectrogramDisplayOptions,
    ) -> std::result::Result<DisplayId, AppError>;

    /// Register a user-owned spectrogram display under an explicit id while its
    /// analysis is still pending.
    ///
    /// UI/transport opens pass the frontend panel id here. Backend displays use
    /// [`FftApp::show_spectrogram_pending`] and the host allocator instead.
    fn register_user_spectrogram_with_id(
        &self,
        id: DisplayId,
        analysis: AnalysisId,
        signal: Option<SignalId>,
        options: &SpectrogramDisplayOptions,
    ) -> std::result::Result<DisplayId, AppError>;

    /// Register an Order Cuts child immediately while its cut is still pending.
    fn show_order_cut_pending(
        &self,
        parent: DisplayId,
        cut: CutId,
        options: &CutDisplayOptions,
    ) -> std::result::Result<DisplayId, AppError>;

    /// Register a Line Cuts child immediately while its cut is still pending.
    fn show_line_cut_pending(
        &self,
        parent: DisplayId,
        cut: CutId,
        options: &CutDisplayOptions,
    ) -> std::result::Result<DisplayId, AppError>;

    /// Change a display's visibility and emit `display.updated`.
    fn set_display_visible(
        &self,
        id: DisplayId,
        visible: bool,
    ) -> std::result::Result<DisplayInfo, AppError>;

    /// Close a display and its descendants. Unknown/already-closed displays
    /// are a successful no-op.
    fn close_display(&self, id: DisplayId) -> std::result::Result<(), AppError>;

    fn subscribe(&self, sink: Arc<dyn AppEventSink>) -> Subscription;

    /// Wait for jobs associated with known displays. Jobs that are not
    /// attached to a display are not included.
    fn wait_all(&self, timeout: Option<Duration>) -> std::result::Result<(), AppError>;

    /// Stop the app. Must be idempotent.
    fn shutdown(&self);
}
