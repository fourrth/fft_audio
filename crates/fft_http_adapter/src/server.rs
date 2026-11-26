//! HTTP state, router, and handlers.
//!
//! Handlers borrow the server-lifetime lease from [`HttpState`]. They never
//! construct an app, a runtime, or another lease.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::SystemTime;

use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Multipart, Path as AxumPath, Query, State};
use axum::http::{header, HeaderValue, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::broadcast;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::services::ServeDir;

use fft_app::{
    AppCapabilities, AppError, AppLease, AppMode, DisplayId, DisplayInfo, FftApp,
    SpectrogramDisplayOptions,
};
use fft_core::cutsession::OrderCutSession;
use fft_core::orders::{self, Cut, LineSpec, RpmProfile};
use fft_core::session::SessionDocument;
use fft_core::settings::{
    AnalysisSettings, CutAxis, DisplaySettings, LineCutSettings, OrderCutSettings, Settings,
};
use fft_core::spectrogram::{RenderOptions, SpectrogramGrid};
use fft_core::table::{PlotLayout, TableInfo, TablePlot};
use fft_core::theme::{theme_slug, ResolvedTheme, ThemeEntry, ThemeLoader, BUILTIN_IDS};
use fft_runtime::{
    AnalysisId, CutId, EntityId, FftRuntime, JobId, LineCutRequest, LoadFileRequest,
    OrderCutRequest, RpmProfileSpec, RuntimeError, SpectrogramData,
};

use crate::events::{EventBridge, EVENT_RING_CAPACITY};
use crate::render::{
    cuts_from_data, encode_pixels, order_cuts_from_data, render_order_cut_png, RenderError,
};

/// Construction-time configuration for [`crate::HttpAdapter`].
#[derive(Clone, Debug)]
pub struct HttpAdapterConfig {
    /// Listener address. Any local address may be bound; choosing a loopback
    /// address is a caller policy, not an adapter restriction.
    pub bind: SocketAddr,
    /// Built frontend directory served for non-`/api` routes.
    pub dist: PathBuf,
    /// Directory containing `settings.toml` and `session.json`.
    pub config_dir: PathBuf,
    /// Where multipart uploads are stored.
    pub uploads_dir: PathBuf,
    /// User theme directory (`themes/*.toml`).
    pub themes_dir: PathBuf,
}

impl HttpAdapterConfig {
    /// Build a config with the conventional `themes/` and
    /// `headless-uploads/` directories under `config_dir`.
    pub fn new(bind: SocketAddr, dist: impl Into<PathBuf>, config_dir: impl Into<PathBuf>) -> Self {
        let config_dir = config_dir.into();
        Self {
            bind,
            dist: dist.into(),
            uploads_dir: config_dir.join("headless-uploads"),
            themes_dir: config_dir.join("themes"),
            config_dir,
        }
    }
}

/// Shared state for every request. The lease is stored exactly once here.
pub(crate) struct HttpState {
    pub(crate) config: HttpAdapterConfig,
    pub(crate) lease: Arc<dyn AppLease>,
    pub(crate) bridge: Arc<EventBridge>,
    pub(crate) events: broadcast::Sender<Value>,
    pub(crate) settings_path: PathBuf,
    pub(crate) session_path: PathBuf,
}

impl HttpState {
    pub(crate) fn new(config: HttpAdapterConfig, lease: Arc<dyn AppLease>) -> Self {
        let settings_path = config.config_dir.join("settings.toml");
        let session_path = config.config_dir.join("session.json");
        let (events, _) = broadcast::channel(EVENT_RING_CAPACITY);
        let bridge = Arc::new(EventBridge::new(events.clone(), Arc::downgrade(&lease)));
        Self {
            config,
            lease,
            bridge,
            events,
            settings_path,
            session_path,
        }
    }

    /// Subscribe the event bridge exactly once, after the listener has bound.
    pub(crate) fn subscribe(&self) {
        self.bridge.prime(self.lease.app());
        let sink: Arc<dyn fft_app::AppEventSink> =
            Arc::clone(&self.bridge) as Arc<dyn fft_app::AppEventSink>;
        self.lease.app().subscribe(sink);
    }

    pub(crate) fn app(&self) -> &dyn FftApp {
        self.lease.app()
    }

    pub(crate) fn runtime(&self) -> Arc<dyn FftRuntime> {
        self.app().runtime()
    }

    fn load_settings(&self) -> Result<Settings, String> {
        if !self.settings_path.exists() {
            return Ok(Settings::default());
        }
        Settings::load(&self.settings_path).map_err(|error| error.to_string())
    }

    fn save_settings(&self, settings: &Settings) -> Result<(), String> {
        settings
            .save(&self.settings_path)
            .map_err(|error| error.to_string())
    }
}

/// Build the complete router. Kept crate-private so the public surface is the
/// adapter/server lifecycle, not axum types.
pub(crate) fn router(state: Arc<HttpState>) -> Router {
    let api = Router::new()
        .route("/app/info", get(app_info))
        .route("/app/ping", post(app_ping))
        .route("/settings", get(get_settings).put(put_settings))
        .route("/settings/defaults", post(put_defaults))
        .route("/config/paths", get(config_paths))
        .route("/open-path", post(open_path))
        .route("/theme", get(get_theme))
        .route("/themes", get(list_themes))
        .route("/themes/duplicate", post(duplicate_theme))
        .route("/session", get(get_session).put(put_session))
        .route("/upload", post(upload))
        .route("/spectrogram/open", post(open_spectrogram))
        .route("/analysis/cancel", post(cancel_analysis))
        .route("/spectrogram/{id}", get(get_spectrogram))
        .route("/spectrogram/{id}/image", get(export_image))
        .route("/displays", get(list_displays))
        .route("/displays/close", post(close_display))
        .route("/displays/spectrogram", post(show_spectrogram))
        .route("/displays/{id}", get(get_display))
        .route("/cuts/order", post(order_cuts))
        .route("/cuts/order/image", post(order_image))
        .route("/cuts/line", post(line_cuts))
        .route("/cuts/csv", post(cuts_csv))
        .route("/plot", post(plot_load))
        .fallback(api_not_found)
        .layer(DefaultBodyLimit::max(MAX_API_BODY_BYTES))
        .layer(loopback_cors())
        .with_state(Arc::clone(&state));

    Router::new()
        .route("/", get(index))
        .route("/ws", get(ws))
        .nest("/api", api)
        .fallback_service(ServeDir::new(state.config.dist.clone()))
        .with_state(state)
}

async fn index(State(state): State<Arc<HttpState>>) -> Response {
    let index = state.config.dist.join("index.html");
    let Ok(html) = tokio::fs::read_to_string(&index).await else {
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            "fft_audio headless server is running. Build the frontend with `npm --prefix web run build` and reload.",
        )
            .into_response();
    };
    let injected = html.replace(
        "</head>",
        "<script>window.__FFT_HEADLESS__=true;window.__FFT_HEADLESS_API__=window.location.origin;</script></head>",
    );
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        injected,
    )
        .into_response()
}

async fn ws(ws: axum::extract::WebSocketUpgrade, State(state): State<Arc<HttpState>>) -> Response {
    let mut events = state.events.subscribe();
    ws.on_upgrade(move |socket| async move {
        let (mut sender, mut receiver) = socket.split();
        loop {
            tokio::select! {
                event = events.recv() => {
                    match event {
                        Ok(value) => {
                            if sender
                                .send(axum::extract::ws::Message::Text(value.to_string().into()))
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(_) => break,
                    }
                }
                incoming = receiver.next() => {
                    if incoming.is_none() {
                        break;
                    }
                }
            }
        }
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppInfoPayload {
    name: String,
    version: String,
    mode: AppMode,
    capabilities: AppCapabilitiesInfo,
}

/// Transport shape kept for the current web UI.
///
/// `scripting` is an adapter-level UI field, not part of
/// `fft_app::AppCapabilities`. The HTTP surface always reports `null`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppCapabilitiesInfo {
    displays: bool,
    scripting: Option<&'static str>,
    table_formats: Vec<fft_core::table::TableFormat>,
}

impl From<AppCapabilities> for AppCapabilitiesInfo {
    fn from(capabilities: AppCapabilities) -> Self {
        Self {
            displays: capabilities.displays,
            scripting: None,
            table_formats: capabilities.table_formats,
        }
    }
}

async fn app_info(State(state): State<Arc<HttpState>>) -> Json<AppInfoPayload> {
    let app = state.app();
    let info = app.info();
    Json(AppInfoPayload {
        name: info.name,
        version: info.version,
        mode: info.mode,
        capabilities: AppCapabilitiesInfo::from(app.capabilities()),
    })
}

#[derive(Deserialize, Serialize)]
struct Ping {
    #[serde(default)]
    nonce: Option<String>,
    #[serde(default)]
    at: u64,
}

async fn app_ping(Json(mut ping): Json<Ping>) -> Json<Ping> {
    ping.at = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as u64);
    Json(ping)
}

async fn get_settings(State(state): State<Arc<HttpState>>) -> Result<Json<Settings>, ApiError> {
    state.load_settings().map(Json).map_err(internal)
}

async fn put_settings(
    State(state): State<Arc<HttpState>>,
    Json(settings): Json<Settings>,
) -> Result<StatusCode, ApiError> {
    state.save_settings(&settings).map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct DefaultsRequest {
    analysis: AnalysisSettings,
    display: DisplaySettings,
}

async fn put_defaults(
    State(state): State<Arc<HttpState>>,
    Json(request): Json<DefaultsRequest>,
) -> Result<StatusCode, ApiError> {
    let mut settings = state.load_settings().map_err(internal)?;
    settings.analysis = request.analysis;
    settings.display = request.display;
    state.save_settings(&settings).map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Paths {
    config_dir: String,
    settings_path: String,
    session_path: String,
    themes_dir: String,
}

async fn config_paths(State(state): State<Arc<HttpState>>) -> Json<Paths> {
    Json(Paths {
        config_dir: state.config.config_dir.to_string_lossy().into_owned(),
        settings_path: state.settings_path.to_string_lossy().into_owned(),
        session_path: state.session_path.to_string_lossy().into_owned(),
        themes_dir: state.config.themes_dir.to_string_lossy().into_owned(),
    })
}

async fn open_path() -> Result<StatusCode, ApiError> {
    Err(ApiError::bad(
        "opening folders is a desktop-only action; use the Files page paths in headless mode",
    ))
}

#[derive(Deserialize)]
struct ThemeQuery {
    name: Option<String>,
}

async fn get_theme(
    State(state): State<Arc<HttpState>>,
    Query(query): Query<ThemeQuery>,
) -> Result<Json<ResolvedTheme>, ApiError> {
    let loader = ThemeLoader::new(state.config.themes_dir.clone());
    loader
        .load(query.name.as_deref().unwrap_or("dark"))
        .map(Json)
        .map_err(|error| ApiError(StatusCode::NOT_FOUND, error.to_string()))
}

async fn list_themes(
    State(state): State<Arc<HttpState>>,
) -> Result<Json<Vec<ThemeEntry>>, ApiError> {
    ThemeLoader::new(state.config.themes_dir.clone())
        .list()
        .map(Json)
        .map_err(internal)
}

#[derive(Deserialize)]
struct DuplicateRequest {
    source: String,
    name: String,
}

#[derive(Serialize)]
struct DuplicateResult {
    id: String,
}

async fn duplicate_theme(
    State(state): State<Arc<HttpState>>,
    Json(request): Json<DuplicateRequest>,
) -> Result<Json<DuplicateResult>, ApiError> {
    let loader = ThemeLoader::new(state.config.themes_dir.clone());
    let theme = loader.load(&request.source).map_err(internal)?;
    let mut id = theme_slug(&request.name);
    if BUILTIN_IDS.contains(&id.as_str()) {
        id.push_str("-user");
    }
    let path = state.config.themes_dir.join(format!("{id}.toml"));
    if path.exists() {
        return Err(ApiError::conflict("that theme name already exists"));
    }
    theme
        .to_document(request.name)
        .save(&path)
        .map_err(internal)?;
    Ok(Json(DuplicateResult { id }))
}

async fn get_session(
    State(state): State<Arc<HttpState>>,
) -> Result<Json<Option<SessionDocument>>, ApiError> {
    SessionDocument::load_json(&state.session_path)
        .map(Json)
        .map_err(internal)
}

async fn put_session(
    State(state): State<Arc<HttpState>>,
    Json(session): Json<SessionDocument>,
) -> Result<StatusCode, ApiError> {
    session.save_json(&state.session_path).map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

fn sanitize_upload(name: &str) -> String {
    let base = Path::new(name)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("upload.bin");
    base.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || "._-".contains(character) {
                character
            } else {
                '_'
            }
        })
        .collect()
}

static UPLOAD_SEQUENCE: AtomicU64 = AtomicU64::new(0);

async fn upload(
    State(state): State<Arc<HttpState>>,
    mut multipart: Multipart,
) -> Result<Json<Value>, ApiError> {
    let Some(field) = multipart
        .next_field()
        .await
        .map_err(|error| ApiError::bad(error.to_string()))?
    else {
        return Err(ApiError::bad("the upload did not contain a file field"));
    };

    let name = field.file_name().unwrap_or("upload.bin").to_string();
    let bytes = field
        .bytes()
        .await
        .map_err(|error| ApiError::bad(error.to_string()))?;
    let file_name = format!(
        "{}_{}_{}",
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_millis()),
        UPLOAD_SEQUENCE.fetch_add(1, Ordering::Relaxed),
        sanitize_upload(&name)
    );
    let path = state.config.uploads_dir.join(&file_name);
    if let Err(error) = tokio::fs::write(&path, &bytes).await {
        let _ = tokio::fs::remove_file(&path).await;
        return Err(internal(error));
    }
    Ok(Json(json!({
        "path": path.to_string_lossy(),
        "name": name,
    })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenRequest {
    display_id: u64,
    path: String,
    settings: Option<AnalysisSettings>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OpenResult {
    display_id: u64,
    /// Runtime analysis created by the open request.
    analysis_id: u64,
}

fn user_display_options(path: &str) -> SpectrogramDisplayOptions {
    let name = Path::new(path)
        .file_stem()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_string);
    SpectrogramDisplayOptions {
        name,
        ..SpectrogramDisplayOptions::default()
    }
}

/// Start a file load + analysis and register a caller-owned user display.
///
/// The frontend panel id is the display id. The analysis stays lazy and its job
/// events are mapped by the bridge once the display record exists.
async fn open_spectrogram(
    State(state): State<Arc<HttpState>>,
    Json(request): Json<OpenRequest>,
) -> Result<Json<OpenResult>, ApiError> {
    if request.display_id == 0 || request.display_id >= fft_app::SCRIPTED_DISPLAY_ID_BASE {
        return Err(ApiError::bad(format!(
            "user display id {} must be between 1 and {}",
            request.display_id,
            fft_app::SCRIPTED_DISPLAY_ID_BASE - 1
        )));
    }
    let settings = request.settings.unwrap_or_default();
    settings
        .validate()
        .map_err(|error| ApiError::bad(format!("invalid analysis settings: {error}")))?;

    let app = state.app();
    let _ = app.close_display(DisplayId(request.display_id));

    let runtime = state.runtime();
    let signal = runtime
        .load_file(LoadFileRequest::new(request.path.clone()))
        .map_err(runtime_bad)?;
    if let Some(job) = runtime.job_for(EntityId::Signal(signal)) {
        state.bridge.ignore(job);
    }
    let analysis = match runtime.analyze(signal, &settings) {
        Ok(analysis) => analysis,
        Err(error) => {
            let _ = runtime.close(EntityId::Signal(signal));
            return Err(runtime_bad(error));
        }
    };
    let options = user_display_options(&request.path);
    let display = match app.register_user_spectrogram_with_id(
        DisplayId(request.display_id),
        analysis,
        Some(signal),
        &options,
    ) {
        Ok(display) => display,
        Err(error) => {
            if let Some(job) = runtime.job_for(EntityId::Analysis(analysis)) {
                state.bridge.ignore(job);
            }
            let _ = runtime.close(EntityId::Signal(signal));
            return Err(app_error(error));
        }
    };
    Ok(Json(OpenResult {
        display_id: display.get(),
        analysis_id: analysis.get(),
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ShowSpectrogramRequest {
    analysis_id: u64,
    /// In-memory analyses are ephemeral by default; file-backed displays may
    /// explicitly opt out once they can be restored.
    #[serde(default = "default_ephemeral")]
    ephemeral: bool,
}

fn default_ephemeral() -> bool {
    true
}

async fn list_displays(
    State(state): State<Arc<HttpState>>,
) -> Result<Json<Vec<DisplayInfo>>, ApiError> {
    state.app().displays().map(Json).map_err(internal)
}

async fn get_display(
    State(state): State<Arc<HttpState>>,
    AxumPath(id): AxumPath<u64>,
) -> Result<Json<DisplayInfo>, ApiError> {
    state
        .app()
        .display(DisplayId(id))
        .map(Json)
        .map_err(|error| ApiError(StatusCode::NOT_FOUND, error.message().to_string()))
}

#[derive(Deserialize)]
struct CloseDisplayRequest {
    id: u64,
}

async fn close_display(
    State(state): State<Arc<HttpState>>,
    Json(request): Json<CloseDisplayRequest>,
) -> Result<StatusCode, ApiError> {
    state
        .app()
        .close_display(DisplayId(request.id))
        .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Create a backend-owned display with a `1_000_000+` id.
async fn show_spectrogram(
    State(state): State<Arc<HttpState>>,
    Json(request): Json<ShowSpectrogramRequest>,
) -> Result<Json<DisplayInfo>, ApiError> {
    let options = SpectrogramDisplayOptions {
        ephemeral: request.ephemeral,
        ..SpectrogramDisplayOptions::default()
    };
    let app = state.app();
    let id = app
        .show_spectrogram(AnalysisId(request.analysis_id), &options)
        .map_err(app_error)?;
    app.display(id).map(Json).map_err(app_error)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CancelRequest {
    display_id: u64,
}

#[derive(Serialize)]
struct CancelResult {
    cancelled: bool,
}

async fn cancel_analysis(
    State(state): State<Arc<HttpState>>,
    Json(request): Json<CancelRequest>,
) -> Json<CancelResult> {
    let runtime = state.runtime();
    let cancelled = analysis_for_display(&state, request.display_id)
        .and_then(|analysis| runtime.job_for(EntityId::Analysis(analysis)))
        .map(|job| {
            if runtime.job_status(job).is_active() {
                runtime.cancel(job).is_ok()
            } else {
                false
            }
        })
        .unwrap_or(false);
    if cancelled {
        // The app event contract has no cancelled variant (runtime Cancelled
        // events are not app events), so the transport reports an advisory
        // `analysis.error` itself.
        let _ = state.events.send(json!({
            "type": "analysis.error",
            "displayId": request.display_id,
            "message": "analysis cancelled",
        }));
        // A cancelled analysis does not leave its display/signal/analysis
        // behind. `display.closed` is emitted by the app contract because the
        // generic transport cannot abandon a record silently.
        let _ = state.app().close_display(DisplayId(request.display_id));
    }
    Json(CancelResult { cancelled })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SpectrogramMeta {
    frames: usize,
    bins: usize,
    bin_hz: f32,
    frame_s: f64,
    duration_s: f64,
    nyquist: f32,
    max_db: f32,
    min_db: f32,
    sample_rate: f64,
    analysis_settings: AnalysisSettings,
    /// Present when the analysis came from a data table.
    table: Option<TableInfo>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SpectrogramPayload {
    meta: SpectrogramMeta,
    data: String,
}

fn payload_from_grid(
    grid: SpectrogramGrid,
    sample_rate: f64,
    analysis_settings: AnalysisSettings,
    table: Option<TableInfo>,
) -> SpectrogramPayload {
    let mut bytes = Vec::with_capacity(grid.data.len() * 2);
    for value in &grid.data {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    SpectrogramPayload {
        meta: SpectrogramMeta {
            frames: grid.frames,
            bins: grid.bins,
            bin_hz: grid.bin_hz,
            frame_s: grid.frame_s,
            duration_s: grid.duration_s,
            nyquist: grid.nyquist,
            max_db: grid.max_db,
            min_db: grid.min_db,
            sample_rate,
            analysis_settings,
            table,
        },
        data: STANDARD.encode(bytes),
    }
}

fn analysis_not_ready() -> ApiError {
    ApiError(StatusCode::NOT_FOUND, "analysis is not ready".into())
}

const GRID_MAX_FRAMES: usize = 2048;
const GRID_SAMPLE_BUDGET: usize = 4_000_000;

fn grid_bins_budget(frames: usize) -> usize {
    let out_frames = frames.clamp(1, GRID_MAX_FRAMES);
    (GRID_SAMPLE_BUDGET / out_frames).max(256)
}

async fn get_spectrogram(
    State(state): State<Arc<HttpState>>,
    AxumPath(id): AxumPath<u64>,
) -> Result<Json<SpectrogramPayload>, ApiError> {
    let analysis = analysis_for_display(&state, id).ok_or_else(analysis_not_ready)?;
    let data = state
        .runtime()
        .spectrogram_data(analysis)
        .map_err(|_| analysis_not_ready())?;
    let grid = data
        .spectrogram
        .to_grid(GRID_MAX_FRAMES, grid_bins_budget(data.frames()));
    Ok(Json(payload_from_grid(
        grid,
        data.sample_rate,
        data.analysis.clone(),
        data.table.clone(),
    )))
}

#[derive(Debug, Default, Deserialize)]
struct SpectrogramImageQuery {
    log: Option<bool>,
    range_db: Option<f32>,
    max_hz: Option<f32>,
    width: Option<usize>,
    height: Option<usize>,
}

fn png_response(bytes: Vec<u8>, disposition: &'static str) -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, HeaderValue::from_static("image/png")),
            (
                header::CONTENT_DISPOSITION,
                HeaderValue::from_static(disposition),
            ),
        ],
        Body::from(bytes),
    )
        .into_response()
}

fn render_error(error: RenderError) -> ApiError {
    match error {
        RenderError::BadSize => ApiError::bad(error.to_string()),
        RenderError::Png(_) => internal(error),
    }
}

async fn export_image(
    State(state): State<Arc<HttpState>>,
    AxumPath(id): AxumPath<u64>,
    Query(query): Query<SpectrogramImageQuery>,
) -> Result<Response, ApiError> {
    let options = RenderOptions {
        width: query.width.unwrap_or(1600).max(1),
        height: query.height.unwrap_or(900).max(1),
        log_freq: query.log.unwrap_or(true),
        min_hz: 10.0,
        max_hz: query.max_hz,
        dynamic_range_db: query.range_db.unwrap_or(80.0),
        db_min: None,
        db_max: None,
    };
    let analysis = analysis_for_display(&state, id).ok_or_else(analysis_not_ready)?;
    let data = state
        .runtime()
        .spectrogram_data(analysis)
        .map_err(|_| analysis_not_ready())?;
    let rendered = data.spectrogram.render(&options);
    let png =
        encode_pixels(rendered.width, rendered.height, &rendered.pixels).map_err(render_error)?;
    Ok(png_response(
        png,
        "attachment; filename=\"spectrogram.png\"",
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OrderRequest {
    parent_id: u64,
    settings: OrderCutSettings,
    #[serde(default)]
    rpm: Option<RpmProfileSpec>,
}

fn rpm_profile_from_spec(spec: &RpmProfileSpec) -> Result<RpmProfile, String> {
    match spec {
        RpmProfileSpec::Constant(rpm) => Ok(RpmProfile::constant(*rpm)),
        RpmProfileSpec::Ramp {
            t0_s,
            t1_s,
            rpm0,
            rpm1,
        } => Ok(RpmProfile::ramp(*t0_s, *t1_s, *rpm0, *rpm1)),
        RpmProfileSpec::Csv(path) => RpmProfile::from_csv(path).map_err(|error| error.to_string()),
        RpmProfileSpec::Points(points) => RpmProfile::from_points(
            points
                .iter()
                .map(|point| (point.time_s, point.rpm))
                .collect(),
        )
        .map_err(|error| error.to_string()),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CutsPayload {
    cuts: Vec<Cut>,
    duration_s: f64,
    nyquist: f32,
    bin_hz: f32,
    hop_s: f64,
}

fn cuts_payload(data: &SpectrogramData, cuts: Vec<Cut>) -> CutsPayload {
    let spectrogram = &data.spectrogram;
    CutsPayload {
        cuts,
        duration_s: spectrogram.end_s(),
        nyquist: spectrogram.nyquist(),
        bin_hz: spectrogram.bin_hz(),
        hop_s: spectrogram.hop_s(),
    }
}

fn empty_cuts() -> CutsPayload {
    CutsPayload {
        cuts: Vec::new(),
        duration_s: 0.0,
        nyquist: 0.0,
        bin_hz: 0.0,
        hop_s: 0.0,
    }
}

/// Busy/closed/unknown jobs mean the analysis is not ready: cut requests keep
/// returning an empty 200 instead of an error.
fn runtime_unavailable(error: &RuntimeError) -> bool {
    matches!(error.code(), "busy" | "not_found" | "closed" | "cancelled")
}

fn analysis_for_display(state: &HttpState, display_id: u64) -> Option<AnalysisId> {
    let display = state.app().display(DisplayId(display_id)).ok()?;
    display
        .state
        .get("analysisId")
        .and_then(Value::as_u64)
        .map(AnalysisId)
}

async fn wait_for_job(runtime: Arc<dyn FftRuntime>, job: JobId) -> Result<(), RuntimeError> {
    match tokio::task::spawn_blocking(move || runtime.wait(job, None)).await {
        Ok(result) => result,
        Err(error) => Err(RuntimeError::internal(format!(
            "job wait task failed: {error}"
        ))),
    }
}

/// Adapter cut requests are transient: close the runtime cut (and its completed
/// job record) on success, error, and request cancellation alike.
struct TransientCut {
    runtime: Arc<dyn FftRuntime>,
    cut: CutId,
}

impl TransientCut {
    fn new(runtime: Arc<dyn FftRuntime>, cut: CutId) -> Self {
        Self { runtime, cut }
    }
}

impl Drop for TransientCut {
    fn drop(&mut self) {
        let _ = self.runtime.close(EntityId::Cut(self.cut));
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlotRequest {
    path: String,
    layout: PlotLayout,
}

/// Map a table file onto generic plot series for the Plot display.
async fn plot_load(Json(request): Json<PlotRequest>) -> Result<Json<TablePlot>, ApiError> {
    fft_core::table::load_plot(Path::new(&request.path), request.layout)
        .map(Json)
        .map_err(|error| ApiError::bad(error.to_string()))
}

async fn order_cuts(
    State(state): State<Arc<HttpState>>,
    Json(request): Json<OrderRequest>,
) -> Result<Json<CutsPayload>, ApiError> {
    let Some(analysis) = analysis_for_display(&state, request.parent_id) else {
        return Ok(Json(empty_cuts()));
    };
    let runtime = state.runtime();
    let Ok(data) = runtime.spectrogram_data(analysis) else {
        return Ok(Json(empty_cuts()));
    };

    let settings = request.settings;
    let rpm_spec = request.rpm;
    let cut_request = OrderCutRequest {
        settings: settings.clone(),
        rpm: rpm_spec.clone(),
    };
    let cut = match runtime.order_cut(analysis, &cut_request) {
        Ok(cut) => cut,
        Err(error) if runtime_unavailable(&error) => return Ok(Json(empty_cuts())),
        Err(error) => return Err(ApiError::bad(error.to_string())),
    };
    if let Some(job) = runtime.job_for(EntityId::Cut(cut)) {
        state.bridge.ignore(job);
        match wait_for_job(Arc::clone(&runtime), job).await {
            Ok(()) => {}
            Err(error) if runtime_unavailable(&error) => return Ok(Json(empty_cuts())),
            Err(error) => return Err(ApiError::bad(error.to_string())),
        }
    }
    let _cut = TransientCut::new(Arc::clone(&runtime), cut);
    let cut_data = match runtime.cut_data(cut) {
        Ok(data) => data,
        Err(error) if runtime_unavailable(&error) => return Ok(Json(empty_cuts())),
        Err(error) => return Err(ApiError::bad(error.to_string())),
    };
    let cuts = if orders::parse_freqs(&settings.freqs).is_empty() {
        // No fixed-frequency cuts: the RPM profile is not needed to rebuild the
        // order matrix, so do not read a CSV/RPM source a second time.
        order_cuts_from_data(
            &cut_data,
            &settings,
            &data.spectrogram,
            &RpmProfile::constant(0.0),
        )
    } else {
        let rpm = match rpm_spec.as_ref() {
            Some(spec) => rpm_profile_from_spec(spec).map_err(ApiError::bad)?,
            None => OrderCutSession::new(settings.clone())
                .rpm_profile_for(&data.spectrogram)
                .map_err(|error| ApiError::bad(error.to_string()))?,
        };
        order_cuts_from_data(&cut_data, &settings, &data.spectrogram, &rpm)
    };
    Ok(Json(cuts_payload(&data, cuts)))
}

async fn order_image(
    State(state): State<Arc<HttpState>>,
    Json(request): Json<OrderRequest>,
) -> Result<Response, ApiError> {
    let OrderRequest {
        parent_id,
        settings,
        rpm: rpm_spec,
    } = request;
    let requested_axis = settings.axis;
    let empty_image = |axis: CutAxis| {
        render_order_cut_png(&[], axis)
            .map(|png| png_response(png, "attachment; filename=\"order-cuts.png\""))
            .map_err(render_error)
    };

    let Some(analysis) = analysis_for_display(&state, parent_id) else {
        // Match `/api/cuts/order`: a child can mount before the parent analysis
        // exists, so return an empty (200) plot instead of a 404.
        return empty_image(requested_axis);
    };
    let runtime = state.runtime();
    let Ok(data) = runtime.spectrogram_data(analysis) else {
        return empty_image(requested_axis);
    };

    let session = OrderCutSession::new(settings.clone());
    let rpm = match rpm_spec.as_ref() {
        Some(spec) => rpm_profile_from_spec(spec).map_err(ApiError::bad)?,
        None => session
            .rpm_profile_for(&data.spectrogram)
            .map_err(|error| ApiError::bad(error.to_string()))?,
    };
    let (axis, _) = orders::resolve_axis(requested_axis, rpm.is_constant());

    let cut_settings = settings.clone();
    let cut_request = OrderCutRequest {
        settings,
        rpm: rpm_spec,
    };
    let cut = match runtime.order_cut(analysis, &cut_request) {
        Ok(cut) => cut,
        Err(error) if runtime_unavailable(&error) => return empty_image(axis),
        Err(error) => return Err(ApiError::bad(error.to_string())),
    };
    if let Some(job) = runtime.job_for(EntityId::Cut(cut)) {
        state.bridge.ignore(job);
        match wait_for_job(Arc::clone(&runtime), job).await {
            Ok(()) => {}
            Err(error) if runtime_unavailable(&error) => return empty_image(axis),
            Err(error) => return Err(ApiError::bad(error.to_string())),
        }
    }
    let _cut = TransientCut::new(Arc::clone(&runtime), cut);
    let cut_data = match runtime.cut_data(cut) {
        Ok(data) => data,
        Err(error) if runtime_unavailable(&error) => return empty_image(axis),
        Err(error) => return Err(ApiError::bad(error.to_string())),
    };
    let cuts = order_cuts_from_data(&cut_data, &cut_settings, &data.spectrogram, &rpm);
    let png = render_order_cut_png(&cuts, axis).map_err(render_error)?;
    Ok(png_response(png, "attachment; filename=\"order-cuts.png\""))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LineRequest {
    parent_id: u64,
    settings: LineCutSettings,
    lines: Vec<LineSpec>,
}

async fn line_cuts(
    State(state): State<Arc<HttpState>>,
    Json(request): Json<LineRequest>,
) -> Result<Json<CutsPayload>, ApiError> {
    let Some(analysis) = analysis_for_display(&state, request.parent_id) else {
        return Ok(Json(empty_cuts()));
    };
    let runtime = state.runtime();
    let Ok(data) = runtime.spectrogram_data(analysis) else {
        return Ok(Json(empty_cuts()));
    };

    let cut_request = LineCutRequest {
        settings: request.settings,
        lines: request.lines,
    };
    let cut = match runtime.line_cut(analysis, &cut_request) {
        Ok(cut) => cut,
        Err(error) if runtime_unavailable(&error) => return Ok(Json(empty_cuts())),
        Err(error) => return Err(ApiError::bad(error.to_string())),
    };
    if let Some(job) = runtime.job_for(EntityId::Cut(cut)) {
        state.bridge.ignore(job);
        match wait_for_job(Arc::clone(&runtime), job).await {
            Ok(()) => {}
            Err(error) if runtime_unavailable(&error) => return Ok(Json(empty_cuts())),
            Err(error) => return Err(ApiError::bad(error.to_string())),
        }
    }
    let _cut = TransientCut::new(Arc::clone(&runtime), cut);
    let cut_data = match runtime.cut_data(cut) {
        Ok(data) => data,
        Err(error) if runtime_unavailable(&error) => return Ok(Json(empty_cuts())),
        Err(error) => return Err(ApiError::bad(error.to_string())),
    };
    Ok(Json(cuts_payload(&data, cuts_from_data(&cut_data))))
}

#[derive(Deserialize)]
struct CsvRequest {
    cuts: Vec<Cut>,
}

static CSV_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Removes the temporary CSV file even when reading/serializing fails.
struct TempCsv(PathBuf);

impl Drop for TempCsv {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn temp_csv_path() -> PathBuf {
    std::env::temp_dir().join(format!(
        "fft_http_adapter_cuts_{}_{}.csv",
        std::process::id(),
        CSV_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ))
}

async fn cuts_csv(Json(request): Json<CsvRequest>) -> Result<Response, ApiError> {
    let path = TempCsv(temp_csv_path());
    orders::write_csv(&path.0, &request.cuts).map_err(internal)?;
    let bytes = tokio::fs::read(&path.0).await.map_err(internal)?;
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, HeaderValue::from_static("text/csv")),
            (
                header::CONTENT_DISPOSITION,
                HeaderValue::from_static("attachment; filename=\"cuts.csv\""),
            ),
        ],
        Body::from(bytes),
    )
        .into_response())
}

#[derive(Debug)]
struct ApiError(StatusCode, String);

impl ApiError {
    fn bad(message: impl Into<String>) -> Self {
        Self(StatusCode::BAD_REQUEST, message.into())
    }

    fn conflict(message: impl Into<String>) -> Self {
        Self(StatusCode::CONFLICT, message.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

fn internal(error: impl ToString) -> ApiError {
    ApiError(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
}

fn runtime_bad(error: RuntimeError) -> ApiError {
    ApiError::bad(error.to_string())
}

/// Map typed app failures to the HTTP categories the UI expects: an unknown
/// analysis/display is 404; a busy/unready one is 400 and creates no display.
fn app_error(error: AppError) -> ApiError {
    match error.code() {
        "not_found" => ApiError(StatusCode::NOT_FOUND, error.to_string()),
        "busy" | "invalid_input" | "closed" | "unsupported" => ApiError::bad(error.to_string()),
        _ => internal(error),
    }
}

/// Unknown `/api/*` paths are API errors, not static assets; keep every method
/// (including POST) at 404 instead of falling through to `ServeDir`'s 405.
async fn api_not_found() -> StatusCode {
    StatusCode::NOT_FOUND
}

/// Audio files are routinely larger than axum's 2 MB default body limit.
const MAX_API_BODY_BYTES: usize = 512 * 1024 * 1024;

fn is_loopback_origin(origin: &HeaderValue) -> bool {
    let Ok(origin) = origin.to_str() else {
        return false;
    };
    let Ok(uri) = origin.parse::<Uri>() else {
        return false;
    };
    if !matches!(uri.scheme_str(), Some("http") | Some("https")) {
        return false;
    }
    let Some(authority) = uri.authority() else {
        return false;
    };
    let host = authority.host();
    host.eq_ignore_ascii_case("localhost")
        || host == "127.0.0.1"
        || host == "[::1]"
        || host == "::1"
}

fn loopback_cors() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|origin, _| {
            is_loopback_origin(origin)
        }))
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::OPTIONS])
        .allow_headers([header::CONTENT_TYPE])
}
