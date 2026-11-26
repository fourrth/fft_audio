//! IPC-facing DTOs shared by the GUI commands and events.
//!
//! These are transport shapes, deliberately kept in this crate so the desktop
//! shell can move in N7 without rewriting the adapter. Field names and serde
//! casing mirror the current Tauri/frontend contract.

use std::time::{SystemTime, UNIX_EPOCH};

use fft_app::DisplayInfo;
use fft_core::settings::AnalysisSettings;
use fft_core::table::{TableFormat, TableInfo};
use serde::Serialize;

/// `app_info` payload. `capabilities` contains the transport-level scripting
/// UI capability in addition to the app's own capabilities.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub mode: fft_app::AppMode,
    pub capabilities: AppCapabilitiesInfo,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppCapabilitiesInfo {
    pub displays: bool,
    /// `Some("python")` only when the optional engine is compiled in. This is
    /// a desktop UI capability owned by the adapter, not by `fft_app`.
    pub scripting: Option<&'static str>,
    pub table_formats: Vec<TableFormat>,
}

impl AppCapabilitiesInfo {
    pub fn from_capabilities(capabilities: fft_app::AppCapabilities) -> Self {
        Self {
            displays: capabilities.displays,
            scripting: scripting_capability(),
            table_formats: capabilities.table_formats,
        }
    }
}

/// Whether this build advertises the desktop scripting UI.
pub fn scripting_capability() -> Option<&'static str> {
    if cfg!(feature = "python") {
        Some("python")
    } else {
        None
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PingResult {
    pub nonce: Option<String>,
    pub at: u64,
}

impl PingResult {
    pub fn now(nonce: Option<String>) -> Self {
        let at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_millis() as u64);
        Self { nonce, at }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenResult {
    pub display_id: u64,
    /// Additive field; older frontends may ignore it.
    pub analysis_id: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelResult {
    pub cancelled: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisProgressEvent {
    pub display_id: u64,
    pub frames_done: usize,
    pub frames_total: Option<usize>,
    pub eta_seconds: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisDoneEvent {
    pub display_id: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisErrorEvent {
    pub display_id: u64,
    pub message: String,
}

/// Tauri payload for `display-open` / `display-updated`.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayEventPayload {
    pub display: DisplayInfo,
}

/// Tauri payload for `display-closed`.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayClosedEvent {
    pub id: u64,
    pub children: Vec<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenFileResult {
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpectrogramMeta {
    pub frames: usize,
    pub bins: usize,
    pub bin_hz: f32,
    pub frame_s: f64,
    pub duration_s: f64,
    pub nyquist: f32,
    pub max_db: f32,
    pub min_db: f32,
    pub sample_rate: f64,
    /// DSP settings that produced this analysis; script/backend displays can
    /// use it instead of local defaults.
    pub analysis_settings: AnalysisSettings,
    /// Present when the analysis came from a data table.
    pub table: Option<TableInfo>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpectrogramPayload {
    pub meta: SpectrogramMeta,
    /// Base64 of `frames * bins` little-endian i16 values (`dB * 100`).
    pub data: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CutsPayload {
    pub cuts: Vec<fft_core::orders::Cut>,
    pub duration_s: f64,
    pub nyquist: f32,
    pub bin_hz: f32,
    pub hop_s: f64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigPaths {
    pub config_dir: String,
    pub settings_path: String,
    pub session_path: String,
    pub themes_dir: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptOutputEvent {
    pub display_id: u64,
    pub stream: &'static str,
    pub line: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptExitEvent {
    pub display_id: u64,
    pub code: Option<i32>,
    pub success: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileStat {
    pub exists: bool,
    pub mtime_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptPathResult {
    pub path: String,
}

/// One Python example embedded in the desktop binary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptExampleInfo {
    /// Flattened file name written under `<config>/scripting-examples/`.
    pub filename: String,
}
