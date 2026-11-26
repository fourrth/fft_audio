//! Tauri IPC commands for the desktop shell and its payloads.
//!
//! Command names are Rust identifiers (the frontend maps dotted names to these)
//! and payload fields stay camelCase. Command bodies only touch the adapter's
//! command core; no HTTP listener or route surface exists in GUI mode.

use std::sync::Arc;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use fft_core::orders::{Cut, LineSpec};
use fft_core::session::SessionDocument;
use fft_core::settings::{
    AnalysisSettings, DisplaySettings, LineCutSettings, OrderCutSettings, Settings,
};
use fft_core::table::{PlotLayout, TablePlot};
use fft_core::theme::{ResolvedTheme, ThemeEntry};
use fft_runtime::{AnalysisId, RpmProfileSpec};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use crate::adapter::TauriAdapter;
use crate::dto::{
    AppInfo, CancelResult, CutsPayload, FileStat, OpenFileResult, OpenResult, PingResult,
    ScriptExampleInfo, ScriptPathResult, SpectrogramPayload,
};
use crate::events::TauriEmitter;
use crate::paths::{export_cuts_csv, read_script_file, write_png, write_script_file};

/// Rust command identifiers registered with Tauri. Kept in a stable order;
/// the web transport maps dotted command names onto these identifiers.
pub const COMMAND_NAMES: &[&str] = &[
    "app_info",
    "app_ping",
    "spectrogram_open",
    "spectrogram_get",
    "display_list",
    "display_get",
    "display_close",
    "display_show_spectrogram",
    "spectrogram_export_image",
    "image_save_png",
    "recording_save",
    "order_cuts_compute",
    "line_cuts_compute",
    "cuts_export_csv",
    "plot_load",
    "analysis_cancel",
    "theme_get",
    "theme_list",
    "session_get",
    "session_save",
    "settings_get",
    "settings_save",
    "config_paths",
    "open_path",
    "theme_duplicate",
    "settings_set_spectrogram_defaults",
    "dialog_open_audio",
    "script_run",
    "script_stop",
    "script_stat",
    "script_read_file",
    "script_write_file",
    "script_list_examples",
    "script_create_example",
    "script_pick_file",
];

/// Register the adapter's command handler on a Tauri builder. The desktop
/// product composes this with its own plugins/setup.
///
/// The handler is concrete over Tauri's default runtime because command
/// signatures take a concrete `AppHandle`.
pub fn install_commands(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    builder.invoke_handler(tauri::generate_handler![
        app_info,
        app_ping,
        spectrogram_open,
        spectrogram_get,
        display_list,
        display_get,
        display_close,
        display_show_spectrogram,
        spectrogram_export_image,
        image_save_png,
        recording_save,
        order_cuts_compute,
        line_cuts_compute,
        cuts_export_csv,
        plot_load,
        analysis_cancel,
        theme_get,
        theme_list,
        session_get,
        session_save,
        settings_get,
        settings_save,
        config_paths,
        open_path,
        theme_duplicate,
        settings_set_spectrogram_defaults,
        dialog_open_audio,
        script_run,
        script_stop,
        script_stat,
        script_read_file,
        script_write_file,
        script_list_examples,
        script_create_example,
        script_pick_file,
    ])
}

#[tauri::command]
pub fn app_info(state: State<'_, TauriAdapter>) -> AppInfo {
    state.app_info()
}

#[tauri::command]
pub fn app_ping(nonce: Option<String>) -> PingResult {
    PingResult::now(nonce)
}

#[tauri::command]
pub fn spectrogram_open(
    state: State<'_, TauriAdapter>,
    display_id: u64,
    path: String,
    settings: Option<AnalysisSettings>,
) -> Result<OpenResult, String> {
    state
        .core()
        .open_analysis(display_id, path, settings.unwrap_or_default())
}

#[tauri::command]
pub fn spectrogram_get(
    state: State<'_, TauriAdapter>,
    display_id: u64,
) -> Result<SpectrogramPayload, String> {
    state.core().spectrogram_payload(display_id)
}

#[tauri::command]
pub fn display_list(state: State<'_, TauriAdapter>) -> Result<Vec<fft_app::DisplayInfo>, String> {
    state.core().list_displays()
}

#[tauri::command]
pub fn display_get(
    state: State<'_, TauriAdapter>,
    display_id: u64,
) -> Result<fft_app::DisplayInfo, String> {
    state.core().get_display(display_id)
}

#[tauri::command]
pub fn display_close(state: State<'_, TauriAdapter>, display_id: u64) -> Result<(), String> {
    state.core().close_display(display_id)
}

#[tauri::command]
pub fn display_show_spectrogram(
    state: State<'_, TauriAdapter>,
    analysis_id: u64,
    ephemeral: Option<bool>,
) -> Result<fft_app::DisplayInfo, String> {
    state
        .core()
        .show_backend_spectrogram(AnalysisId(analysis_id), ephemeral.unwrap_or(true))
}

#[tauri::command]
pub async fn order_cuts_compute(
    state: State<'_, TauriAdapter>,
    parent_id: u64,
    settings: OrderCutSettings,
    rpm: Option<RpmProfileSpec>,
) -> Result<CutsPayload, String> {
    // Readiness is checked on the command task so a pending parent returns the
    // not-ready error immediately; only the cut wait is blocking.
    let (analysis, data) = state.core().ready_parent(parent_id)?;
    let core = Arc::clone(state.core());
    tauri::async_runtime::spawn_blocking(move || {
        core.order_cuts_from_parent(analysis, data, settings, rpm)
    })
    .await
    .map_err(|error| format!("order cut task failed: {error}"))?
}

#[tauri::command]
pub async fn line_cuts_compute(
    state: State<'_, TauriAdapter>,
    parent_id: u64,
    settings: LineCutSettings,
    lines: Vec<LineSpec>,
) -> Result<CutsPayload, String> {
    let (analysis, data) = state.core().ready_parent(parent_id)?;
    let core = Arc::clone(state.core());
    tauri::async_runtime::spawn_blocking(move || {
        core.line_cuts_from_parent(analysis, data, settings, lines)
    })
    .await
    .map_err(|error| format!("line cut task failed: {error}"))?
}

#[tauri::command]
pub async fn plot_load(path: String, layout: PlotLayout) -> Result<TablePlot, String> {
    tauri::async_runtime::spawn_blocking(move || {
        fft_core::table::load_plot(std::path::Path::new(&path), layout)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| format!("plot load task failed: {error}"))?
}

#[tauri::command]
pub async fn cuts_export_csv(
    app: AppHandle,
    cuts: Vec<Cut>,
) -> Result<Option<OpenFileResult>, String> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter("CSV", &["csv"])
            .set_file_name("cuts.csv")
            .blocking_save_file()
    })
    .await
    .map_err(|error| error.to_string())?;
    let Some(path) = picked else {
        return Ok(None);
    };
    let path = path
        .into_path()
        .map_err(|error| format!("the picker returned an unusable path: {error}"))?;
    export_cuts_csv(&path, &cuts)?;
    Ok(Some(OpenFileResult {
        path: path.to_string_lossy().into_owned(),
    }))
}

#[tauri::command]
pub async fn spectrogram_export_image(
    app: AppHandle,
    state: State<'_, TauriAdapter>,
    display_id: u64,
) -> Result<Option<OpenFileResult>, String> {
    let image = state.core().render_spectrogram(display_id)?;
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter("PNG image", &["png"])
            .set_file_name("spectrogram.png")
            .blocking_save_file()
    })
    .await
    .map_err(|error| error.to_string())?;
    let Some(path) = picked else {
        return Ok(None);
    };
    let path = path
        .into_path()
        .map_err(|error| format!("the picker returned an unusable path: {error}"))?;
    let save_path = path.clone();
    tauri::async_runtime::spawn_blocking(move || write_png(&save_path, &image))
        .await
        .map_err(|error| error.to_string())??;
    Ok(Some(OpenFileResult {
        path: path.to_string_lossy().into_owned(),
    }))
}

/// Save a recorded frontend audio blob into the desktop config's
/// `recordings/` folder so it can be analysed like any other input file.
#[tauri::command]
pub fn recording_save(
    state: State<'_, TauriAdapter>,
    data_base64: String,
    filename: String,
) -> Result<OpenFileResult, String> {
    let bytes = STANDARD
        .decode(data_base64)
        .map_err(|error| format!("invalid recording payload: {error}"))?;
    let path = state.core().paths().save_recording(&filename, &bytes)?;
    Ok(OpenFileResult {
        path: path.to_string_lossy().into_owned(),
    })
}

/// Save a frontend-rendered PNG (spectrogram viewport or cut plot) through the
/// native save dialog. The browser/headless transports use their own download
/// path; this command exists only in the desktop adapter.
#[tauri::command]
pub async fn image_save_png(
    app: AppHandle,
    data_base64: String,
    filename: String,
) -> Result<Option<OpenFileResult>, String> {
    let bytes = STANDARD
        .decode(data_base64)
        .map_err(|error| format!("invalid PNG payload: {error}"))?;
    let suggested = if filename.trim().is_empty() {
        "plot.png".to_string()
    } else if filename.to_ascii_lowercase().ends_with(".png") {
        filename
    } else {
        format!("{filename}.png")
    };
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter("PNG image", &["png"])
            .set_file_name(suggested)
            .blocking_save_file()
    })
    .await
    .map_err(|error| error.to_string())?;
    let Some(path) = picked else {
        return Ok(None);
    };
    let path = path
        .into_path()
        .map_err(|error| format!("the picker returned an unusable path: {error}"))?;
    let save_path = path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::write(&save_path, bytes).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())??;
    Ok(Some(OpenFileResult {
        path: path.to_string_lossy().into_owned(),
    }))
}

#[tauri::command]
pub fn analysis_cancel(state: State<'_, TauriAdapter>, display_id: u64) -> CancelResult {
    state.core().cancel_analysis(display_id)
}

#[tauri::command]
pub fn theme_get(
    state: State<'_, TauriAdapter>,
    name: Option<String>,
) -> Result<ResolvedTheme, String> {
    state.core().paths().get_theme(name.as_deref())
}

#[tauri::command]
pub fn theme_list(state: State<'_, TauriAdapter>) -> Result<Vec<ThemeEntry>, String> {
    state.core().paths().list_themes()
}

#[tauri::command]
pub fn session_get(state: State<'_, TauriAdapter>) -> Result<Option<SessionDocument>, String> {
    state.core().paths().get_session()
}

#[tauri::command]
pub fn session_save(
    state: State<'_, TauriAdapter>,
    session: SessionDocument,
) -> Result<(), String> {
    state.core().paths().save_session(&session)
}

#[tauri::command]
pub fn settings_get(state: State<'_, TauriAdapter>) -> Result<Settings, String> {
    state.core().paths().get_settings()
}

#[tauri::command]
pub fn settings_save(state: State<'_, TauriAdapter>, settings: Settings) -> Result<(), String> {
    state.core().paths().save_settings(&settings)
}

#[tauri::command]
pub fn config_paths(state: State<'_, TauriAdapter>) -> Result<crate::dto::ConfigPaths, String> {
    state.core().paths().config_paths()
}

#[tauri::command]
pub fn open_path(app: AppHandle, path: String) -> Result<(), String> {
    app.opener()
        .open_path(path, None::<String>)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn theme_duplicate(
    state: State<'_, TauriAdapter>,
    source: String,
    name: String,
) -> Result<String, String> {
    state.core().paths().duplicate_theme(&source, name)
}

#[tauri::command]
pub fn settings_set_spectrogram_defaults(
    state: State<'_, TauriAdapter>,
    analysis: AnalysisSettings,
    display: DisplaySettings,
) -> Result<(), String> {
    state
        .core()
        .paths()
        .set_spectrogram_defaults(analysis, display)
}

#[tauri::command]
pub async fn dialog_open_audio(app: AppHandle) -> Result<Option<OpenFileResult>, String> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        let table_extensions = fft_core::table::available_table_extensions();
        let audio_extensions = ["wav", "flac", "mp3", "ogg", "aac", "m4a", "opus", "caf"];
        let mut all_extensions: Vec<&str> = audio_extensions.to_vec();
        all_extensions.extend(table_extensions.iter().copied());
        app.dialog()
            .file()
            // Default to everything the product can analyse. Audio and table
            // filters remain available for users who want to narrow the list.
            .add_filter("All Files", all_extensions.as_slice())
            .add_filter("Audio", &audio_extensions)
            .add_filter("Data tables", table_extensions.as_slice())
            .blocking_pick_file()
    })
    .await
    .map_err(|error| error.to_string())?;
    match picked {
        Some(path) => {
            let path = path
                .into_path()
                .map_err(|error| format!("the picker returned an unusable path: {error}"))?;
            Ok(Some(OpenFileResult {
                path: path.to_string_lossy().into_owned(),
            }))
        }
        None => Ok(None),
    }
}

#[tauri::command]
pub fn script_run(
    app: AppHandle,
    state: State<'_, TauriAdapter>,
    display_id: u64,
    path: String,
) -> Result<(), String> {
    let emitter: Arc<dyn crate::events::EventEmitter> = Arc::new(TauriEmitter::new(app));
    state.script_run(display_id, path, emitter)
}

#[tauri::command]
pub fn script_stop(state: State<'_, TauriAdapter>, display_id: u64) -> Result<(), String> {
    state.script_stop(display_id)
}

#[tauri::command]
pub fn script_stat(path: String) -> FileStat {
    crate::paths::script_stat(&path)
}

#[tauri::command]
pub fn script_read_file(path: String) -> Result<String, String> {
    read_script_file(&path)
}

#[tauri::command]
pub fn script_write_file(path: String, contents: String) -> Result<(), String> {
    write_script_file(&path, &contents)
}

#[tauri::command]
pub fn script_list_examples(state: State<'_, TauriAdapter>) -> Vec<ScriptExampleInfo> {
    state.core().paths().list_script_examples()
}

#[tauri::command]
pub fn script_create_example(
    state: State<'_, TauriAdapter>,
    filename: String,
) -> Result<ScriptPathResult, String> {
    state.core().paths().create_script_example(&filename)
}

#[tauri::command]
pub async fn script_pick_file(app: AppHandle) -> Result<Option<OpenFileResult>, String> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter("Python", &["py"])
            .blocking_pick_file()
    })
    .await
    .map_err(|error| error.to_string())?;
    let Some(path) = picked else {
        return Ok(None);
    };
    let path = path
        .into_path()
        .map_err(|error| format!("the picker returned an unusable path: {error}"))?;
    Ok(Some(OpenFileResult {
        path: path.to_string_lossy().into_owned(),
    }))
}

/// Stop embedded scripts, the translator, and the root lease on app exit.
pub fn shutdown(app: &AppHandle) {
    if let Some(adapter) = app.try_state::<TauriAdapter>() {
        adapter.shutdown();
    }
}
