//! Minimal in-browser/WASM surface for [`fft_core`].
//!
//! This crate is deliberately a thin transport helper: it owns in-memory
//! analyses and returns JSON/DTOs matching the desktop IPC shapes. It has no
//! DOM, filesystem, threading, or JS framework dependency. The web worker in
//! `web/src/wasm/worker.ts` owns the JS side.
//!
//! All analysis is synchronous inside the caller's worker. Large results are
//! returned as compact grids/PNG bytes, never as a second server-side state.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use fft_core::cutsession::{LineCutSession, OrderCutSession};
use fft_core::orders::{self, Cut, RpmProfile};
use fft_core::pipeline;
use fft_core::settings::{AnalysisSettings, LineCutSettings, OrderCutSettings, RpmSource};
use fft_core::sources::FileSource;
use fft_core::spectrogram::{RenderOptions, Spectrogram};
use fft_core::table::{self, TableFormat, TableInfo};
use fft_core::theme::{
    builtin_theme, theme_slug, Appearance, ResolvedTheme, ThemeDocument, ThemeEntry, BUILTIN_IDS,
};
use fft_core::traits::SignalSource;
use js_sys::Function;
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

const GRID_MAX_FRAMES: usize = 2048;
const GRID_SAMPLE_BUDGET: usize = 4_000_000;

/// One analyser result retained for a display id.
struct StoredAnalysis {
    spectrogram: Spectrogram,
    sample_rate: f64,
    analysis: AnalysisSettings,
    name: String,
    table: Option<TableInfo>,
}

/// Browser-side engine. The worker keeps one instance and keys analyses by the
/// frontend display id.
#[wasm_bindgen]
pub struct FftWasm {
    analyses: HashMap<u32, StoredAnalysis>,
}

#[wasm_bindgen]
impl FftWasm {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            analyses: HashMap::new(),
        }
    }

    /// Decode an audio/table buffer and run a spectrogram analysis.
    ///
    /// `name` supplies the original file name (extension used for format
    /// detection) and the display name. `settings_json` is `AnalysisSettings`.
    /// The analysis replaces any existing result for `display_id`.
    pub fn open(
        &mut self,
        display_id: u32,
        name: &str,
        bytes: &[u8],
        settings_json: &str,
    ) -> Result<(), JsValue> {
        self.open_inner(display_id, name, bytes, settings_json, None)
    }

    /// Like [`FftWasm::open`], but calls `progress(frames_done, frames_total)`
    /// after every analysed frame. The callback runs synchronously on the WASM
    /// thread, so the worker should throttle before posting UI events.
    pub fn open_with_progress(
        &mut self,
        display_id: u32,
        name: &str,
        bytes: &[u8],
        settings_json: &str,
        progress: &Function,
    ) -> Result<(), JsValue> {
        self.open_inner(display_id, name, bytes, settings_json, Some(progress))
    }

    /// Compact transport grid plus display metadata.
    ///
    /// Pass `0` for either limit to use the shared browser budget.
    pub fn payload(
        &self,
        display_id: u32,
        max_frames: u32,
        max_bins: u32,
    ) -> Result<String, JsValue> {
        self.payload_inner(display_id, max_frames as usize, max_bins as usize)
            .map_err(js_error)
    }

    /// Render the full-resolution spectrogram to a PNG.
    ///
    /// `options_json` may be `{}`; unknown fields are ignored. Supported fields
    /// are `width`, `height`, `logFreq`, `minHz`, `maxHz`, `rangeDb`, `dbMin`,
    /// and `dbMax`.
    pub fn export_png(&self, display_id: u32, options_json: &str) -> Result<Vec<u8>, JsValue> {
        self.export_png_inner(display_id, options_json)
            .map_err(js_error)
    }

    /// Compute all requested order/fixed-frequency cuts for a display.
    pub fn order_cuts(&self, request_json: &str) -> Result<String, JsValue> {
        self.order_cuts_inner(request_json).map_err(js_error)
    }

    /// Compute all requested line cuts for a display.
    pub fn line_cuts(&self, request_json: &str) -> Result<String, JsValue> {
        self.line_cuts_inner(request_json).map_err(js_error)
    }

    /// Serialize wire cuts to the shared long-format CSV.
    pub fn cuts_csv(&self, cuts_json: &str) -> Result<String, JsValue> {
        self.cuts_csv_inner(cuts_json).map_err(js_error)
    }

    /// Resolve a built-in or browser-local (TOML string map) theme.
    pub fn theme_get(&self, id: &str, custom_json: &str) -> Result<String, JsValue> {
        self.theme_get_inner(id, custom_json).map_err(js_error)
    }

    /// List built-in themes plus browser-local themes.
    pub fn theme_list(&self, custom_json: &str) -> Result<String, JsValue> {
        self.theme_list_inner(custom_json).map_err(js_error)
    }

    /// Flatten a theme into a standalone TOML document for local storage.
    pub fn theme_duplicate(
        &self,
        source: &str,
        name: &str,
        custom_json: &str,
    ) -> Result<String, JsValue> {
        self.theme_duplicate_inner(source, name, custom_json)
            .map_err(js_error)
    }

    /// Release an analysis/display's in-memory data.
    pub fn close(&mut self, display_id: u32) {
        self.analyses.remove(&display_id);
    }
}

impl Default for FftWasm {
    fn default() -> Self {
        Self::new()
    }
}

impl FftWasm {
    fn open_inner(
        &mut self,
        display_id: u32,
        name: &str,
        bytes: &[u8],
        settings_json: &str,
        progress: Option<&Function>,
    ) -> Result<(), JsValue> {
        let settings: AnalysisSettings = serde_json::from_str(settings_json)
            .map_err(|error| js_error(format!("invalid analysis settings: {error}")))?;
        let mut emit = |done: usize, total: Option<usize>| {
            let Some(progress) = progress else {
                return;
            };
            let args = js_sys::Array::new();
            args.push(&JsValue::from_f64(done as f64));
            args.push(
                &total
                    .map(|value| JsValue::from_f64(value as f64))
                    .unwrap_or(JsValue::NULL),
            );
            let _ = progress.apply(&JsValue::NULL, &args);
        };
        let stored = decode_analysis(name, bytes, &settings, Some(&mut emit)).map_err(js_error)?;
        self.analyses.insert(display_id, stored);
        Ok(())
    }

    fn analysis(&self, display_id: u32) -> Result<&StoredAnalysis, String> {
        self.analyses
            .get(&display_id)
            .ok_or_else(|| format!("analysis for display {display_id} is not ready"))
    }

    fn payload_inner(
        &self,
        display_id: u32,
        max_frames: usize,
        max_bins: usize,
    ) -> Result<String, String> {
        let stored = self.analysis(display_id)?;
        let frames = if max_frames == 0 {
            GRID_MAX_FRAMES
        } else {
            max_frames.min(GRID_MAX_FRAMES)
        };
        let bins = if max_bins == 0 {
            grid_bins_budget(stored.spectrogram.frames())
        } else {
            max_bins
        };
        let grid = stored.spectrogram.to_grid(frames, bins);
        let mut bytes = Vec::with_capacity(grid.data.len() * 2);
        for value in &grid.data {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let payload = SpectrogramPayload {
            meta: meta_for(stored, &grid),
            data: STANDARD.encode(bytes),
        };
        serde_json::to_string(&payload).map_err(|error| error.to_string())
    }

    fn export_png_inner(&self, display_id: u32, options_json: &str) -> Result<Vec<u8>, String> {
        let stored = self.analysis(display_id)?;
        let request: RenderRequest = if options_json.trim().is_empty() {
            RenderRequest::default()
        } else {
            serde_json::from_str(options_json)
                .map_err(|error| format!("invalid render options: {error}"))?
        };
        let options = RenderOptions {
            width: request.width.unwrap_or(1600).max(1),
            height: request.height.unwrap_or(900).max(1),
            log_freq: request.log_freq.unwrap_or(true),
            min_hz: request.min_hz.unwrap_or(10.0),
            max_hz: request.max_hz,
            dynamic_range_db: request.range_db.unwrap_or(80.0),
            db_min: request.db_min,
            db_max: request.db_max,
        };
        let image = stored.spectrogram.render(&options);
        let mut rgb = Vec::with_capacity(image.width * image.height * 3);
        for pixel in &image.pixels {
            rgb.push(((pixel >> 16) & 0xff) as u8);
            rgb.push(((pixel >> 8) & 0xff) as u8);
            rgb.push((pixel & 0xff) as u8);
        }
        encode_png(image.width as u32, image.height as u32, &rgb)
    }

    fn order_cuts_inner(&self, request_json: &str) -> Result<String, String> {
        let request: OrderRequest = serde_json::from_str(request_json)
            .map_err(|error| format!("invalid order-cut request: {error}"))?;
        let stored = self.analysis(request.parent_id)?;
        let mut session = OrderCutSession::new(request.settings.clone());
        let rpm = match &request.rpm {
            Some(spec) => spec.to_profile()?,
            None if request.settings.rpm.source == RpmSource::Csv => {
                return Err("CSV RPM profiles are not available in the browser build".into())
            }
            None => session
                .rpm_profile_for(&stored.spectrogram)
                .map_err(|error| error.to_string())?,
        };
        session.compute_with_profile(&stored.spectrogram, &rpm);
        cuts_payload(stored, session.cuts().to_vec())
    }

    fn line_cuts_inner(&self, request_json: &str) -> Result<String, String> {
        let request: LineRequest = serde_json::from_str(request_json)
            .map_err(|error| format!("invalid line-cut request: {error}"))?;
        let stored = self.analysis(request.parent_id)?;
        let mut session = LineCutSession::new(request.settings);
        session.set_lines(request.lines);
        session.compute(&stored.spectrogram);
        cuts_payload(stored, session.cuts().to_vec())
    }

    fn cuts_csv_inner(&self, cuts_json: &str) -> Result<String, String> {
        let request: CutsRequest = serde_json::from_str(cuts_json)
            .map_err(|error| format!("invalid cut data: {error}"))?;
        orders::csv_string(&request.cuts).map_err(|error| error.to_string())
    }

    fn theme_get_inner(&self, id: &str, custom_json: &str) -> Result<String, String> {
        let custom = parse_custom_themes(custom_json)?;
        let theme = resolve_theme(id, &custom, &mut Vec::new())?;
        let errors = theme.validate();
        if !errors.is_empty() {
            return Err(format!("theme '{id}' is invalid: {}", errors.join("; ")));
        }
        serde_json::to_string(&theme).map_err(|error| error.to_string())
    }

    fn theme_list_inner(&self, custom_json: &str) -> Result<String, String> {
        let custom = parse_custom_themes(custom_json)?;
        let mut entries = Vec::new();
        for id in BUILTIN_IDS {
            let theme = resolve_theme(id, &custom, &mut Vec::new())?;
            entries.push(ThemeEntry {
                id: id.to_string(),
                name: theme.name.clone(),
                builtin: true,
                appearance: Some(theme.appearance),
                preview: theme.preview(),
                parse_error: None,
            });
        }
        for (id, toml) in &custom {
            match ThemeDocument::from_toml(toml) {
                Ok(document) => {
                    let resolved = resolve_theme(id, &custom, &mut Vec::new()).ok();
                    entries.push(ThemeEntry {
                        id: id.clone(),
                        name: document.display_name(id).to_string(),
                        builtin: false,
                        appearance: document
                            .appearance
                            .or_else(|| resolved.as_ref().map(|theme| theme.appearance)),
                        preview: resolved.as_ref().and_then(ResolvedTheme::preview),
                        parse_error: None,
                    });
                }
                Err(error) => entries.push(ThemeEntry {
                    id: id.clone(),
                    name: id.clone(),
                    builtin: false,
                    appearance: None,
                    preview: None,
                    parse_error: Some(error.to_string()),
                }),
            }
        }
        serde_json::to_string(&entries).map_err(|error| error.to_string())
    }

    fn theme_duplicate_inner(
        &self,
        source: &str,
        name: &str,
        custom_json: &str,
    ) -> Result<String, String> {
        let custom = parse_custom_themes(custom_json)?;
        let theme = resolve_theme(source, &custom, &mut Vec::new())?;
        let mut id = theme_slug(name);
        if BUILTIN_IDS.contains(&id.as_str()) || custom.contains_key(&id) {
            let base = id.clone();
            let mut suffix = 2;
            loop {
                id = format!("{base}-{suffix}");
                if !BUILTIN_IDS.contains(&id.as_str()) && !custom.contains_key(&id) {
                    break;
                }
                suffix += 1;
            }
        }
        let document = theme.to_document(name.to_string());
        let toml = document.to_toml().map_err(|error| error.to_string())?;
        serde_json::to_string(&DuplicateTheme { id, toml }).map_err(|error| error.to_string())
    }
}

fn decode_analysis(
    name: &str,
    bytes: &[u8],
    settings: &AnalysisSettings,
    mut progress: Option<&mut dyn FnMut(usize, Option<usize>)>,
) -> Result<StoredAnalysis, String> {
    settings
        .validate()
        .map_err(|error| format!("invalid analysis settings: {error}"))?;

    let extension = Path::new(name).extension().and_then(|value| value.to_str());
    let (samples, sample_rate, table) = match TableFormat::from_path(Path::new(name)) {
        Some(format) => {
            let signal = table::load_bytes(bytes, format, &settings.data, extension)
                .map_err(|error| error.to_string())?;
            (signal.samples, signal.sample_rate, Some(signal.info))
        }
        None => {
            let mut source = FileSource::open_bytes(bytes.to_vec(), extension)
                .map_err(|error| format!("could not decode audio: {error}"))?;
            let sample_rate = source.sample_rate();
            let mut samples = Vec::new();
            let mut buffer = [0.0f32; 8192];
            loop {
                let read = source
                    .read(&mut buffer)
                    .map_err(|error| format!("could not decode audio: {error}"))?;
                if read == 0 {
                    break;
                }
                samples.extend_from_slice(&buffer[..read]);
            }
            (samples, sample_rate, None)
        }
    };

    if samples.is_empty() {
        return Err("no samples decoded".into());
    }
    let analysis =
        pipeline::analyze_samples_with_progress(samples, sample_rate, settings, |done, total| {
            if let Some(progress) = progress.as_deref_mut() {
                progress(done, total);
            }
            true
        })
        .map_err(|error| error.to_string())?;
    Ok(StoredAnalysis {
        spectrogram: analysis.spectrogram,
        sample_rate: analysis.sample_rate,
        analysis: settings.clone(),
        name: name.to_string(),
        table,
    })
}

fn meta_for(
    stored: &StoredAnalysis,
    grid: &fft_core::spectrogram::SpectrogramGrid,
) -> SpectrogramMeta {
    SpectrogramMeta {
        frames: grid.frames,
        bins: grid.bins,
        bin_hz: grid.bin_hz,
        frame_s: grid.frame_s,
        duration_s: grid.duration_s,
        nyquist: grid.nyquist,
        max_db: grid.max_db,
        min_db: grid.min_db,
        sample_rate: stored.sample_rate,
        analysis_settings: stored.analysis.clone(),
        name: stored.name.clone(),
        table: stored.table.clone(),
    }
}

fn grid_bins_budget(frames: usize) -> usize {
    let out_frames = frames.clamp(1, GRID_MAX_FRAMES);
    (GRID_SAMPLE_BUDGET / out_frames).max(256)
}

fn cuts_payload(stored: &StoredAnalysis, cuts: Vec<Cut>) -> Result<String, String> {
    let spectrogram = &stored.spectrogram;
    let payload = CutsPayload {
        cuts,
        duration_s: spectrogram.end_s(),
        nyquist: spectrogram.nyquist(),
        bin_hz: spectrogram.bin_hz(),
        hop_s: spectrogram.hop_s(),
    };
    serde_json::to_string(&payload).map_err(|error| error.to_string())
}

fn encode_png(width: u32, height: u32, rgb: &[u8]) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut output, width, height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
        writer
            .write_image_data(rgb)
            .map_err(|error| error.to_string())?;
    }
    Ok(output)
}

fn parse_custom_themes(json: &str) -> Result<BTreeMap<String, String>, String> {
    if json.trim().is_empty() {
        return Ok(BTreeMap::new());
    }
    serde_json::from_str(json).map_err(|error| format!("invalid custom themes: {error}"))
}

fn resolve_theme(
    id: &str,
    custom: &BTreeMap<String, String>,
    stack: &mut Vec<String>,
) -> Result<ResolvedTheme, String> {
    if id == "dark" || id == "light" {
        let appearance = if id == "dark" {
            Appearance::Dark
        } else {
            Appearance::Light
        };
        return builtin_theme(appearance).map_err(|error| error.to_string());
    }
    if stack.iter().any(|item| item == id) {
        let mut chain = stack.join(" -> ");
        chain.push_str(" -> ");
        chain.push_str(id);
        return Err(format!("theme extends cycle: {chain}"));
    }
    if stack.len() >= 16 {
        return Err("theme extends chain is too deep (max 16)".into());
    }
    let toml = custom
        .get(id)
        .ok_or_else(|| format!("theme '{id}' is not installed"))?;
    let document = ThemeDocument::from_toml(toml)
        .map_err(|error| format!("theme '{id}' could not be parsed: {error}"))?;
    stack.push(id.to_string());
    let base = match document.extends.as_deref() {
        Some(parent) => resolve_theme(parent, custom, stack)?,
        None => builtin_theme(document.appearance.unwrap_or_default())
            .map_err(|error| error.to_string())?,
    };
    stack.pop();

    let appearance = document.appearance.unwrap_or(base.appearance);
    let mut colors = base.colors;
    colors.extend(document.colors);
    let mut status = base.status;
    status.extend(document.status);
    let plot_palette = if document.plot_palette.is_empty() {
        base.plot_palette
    } else {
        document.plot_palette
    };
    Ok(ResolvedTheme {
        name: document.name.unwrap_or(base.name),
        appearance,
        colors,
        plot_palette,
        status,
    })
}

fn js_error(message: impl Into<String>) -> JsValue {
    JsValue::from_str(&message.into())
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
    name: String,
    table: Option<TableInfo>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SpectrogramPayload {
    meta: SpectrogramMeta,
    data: String,
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

#[derive(Deserialize)]
struct CutsRequest {
    cuts: Vec<Cut>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OrderRequest {
    parent_id: u32,
    settings: OrderCutSettings,
    #[serde(default)]
    rpm: Option<RpmSpec>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LineRequest {
    parent_id: u32,
    settings: LineCutSettings,
    lines: Vec<fft_core::orders::LineSpec>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
enum RpmSpec {
    Constant(f64),
    Ramp {
        t0_s: f64,
        t1_s: f64,
        rpm0: f64,
        rpm1: f64,
    },
    Csv(String),
    Points(Vec<RpmPoint>),
}

impl RpmSpec {
    fn to_profile(&self) -> Result<RpmProfile, String> {
        match self {
            RpmSpec::Constant(rpm) => Ok(RpmProfile::constant(*rpm)),
            RpmSpec::Ramp {
                t0_s,
                t1_s,
                rpm0,
                rpm1,
            } => Ok(RpmProfile::ramp(*t0_s, *t1_s, *rpm0, *rpm1)),
            RpmSpec::Csv(path) => Err(format!(
                "CSV RPM profile '{path}' is not available in the browser build"
            )),
            RpmSpec::Points(points) => RpmProfile::from_points(
                points
                    .iter()
                    .map(|point| (point.time_s, point.rpm))
                    .collect(),
            )
            .map_err(|error| error.to_string()),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RpmPoint {
    time_s: f64,
    rpm: f64,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct RenderRequest {
    width: Option<usize>,
    height: Option<usize>,
    log_freq: Option<bool>,
    min_hz: Option<f32>,
    max_hz: Option<f32>,
    range_db: Option<f32>,
    db_min: Option<f32>,
    db_max: Option<f32>,
}

#[derive(Serialize)]
struct DuplicateTheme {
    id: String,
    toml: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use fft_core::testutil::{tones, write_wav};

    fn settings() -> AnalysisSettings {
        AnalysisSettings {
            frame: 512,
            hop: Some(128),
            pad: 2,
            ..Default::default()
        }
    }

    #[test]
    fn analyzes_audio_bytes_and_renders_grid_and_png() {
        let samples = tones(8000, 0.5, &[(50.0, 0.8)]);
        let path = write_wav("wasm_audio", 8000, &samples);
        let bytes = std::fs::read(&path).unwrap();
        let _ = std::fs::remove_file(&path);

        let mut engine = FftWasm::new();
        engine.open_inner_for_test("tone.wav", &bytes).unwrap();
        let payload = engine.payload_inner(1, 0, 0).unwrap();
        let value: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert!(value["meta"]["frames"].as_u64().unwrap() > 0);
        assert!(value["meta"]["bins"].as_u64().unwrap() > 0);
        assert!(!value["data"].as_str().unwrap().is_empty());

        let png = engine.export_png_inner(1, "{}").unwrap();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    }

    #[test]
    fn reports_per_frame_analysis_progress() {
        let samples = tones(8000, 0.5, &[(50.0, 0.8)]);
        let path = write_wav("wasm_progress", 8000, &samples);
        let bytes = std::fs::read(&path).unwrap();
        let _ = std::fs::remove_file(&path);

        let mut events = Vec::new();
        let mut progress = |done: usize, total: Option<usize>| {
            events.push((done, total));
        };
        let stored = decode_analysis("tone.wav", &bytes, &settings(), Some(&mut progress)).unwrap();
        assert!(stored.spectrogram.frames() > 1);
        assert_eq!(events.len(), stored.spectrogram.frames());
        assert_eq!(events.first().unwrap().0, 1);
        let (done, total) = events.last().unwrap();
        assert_eq!(*done, stored.spectrogram.frames());
        // The estimate may omit the final partial frame; the worker clamps it
        // to the completed count before publishing UI progress.
        assert!(total.unwrap() >= 1);
    }

    #[test]
    fn analyzes_csv_bytes_and_computes_cuts() {
        let csv = b"time,value\n0.0,0.0\n0.1,0.8\n0.2,0.0\n0.3,-0.8\n";
        let mut engine = FftWasm::new();
        let mut table_settings = settings();
        table_settings.frame = 16;
        table_settings.hop = Some(4);
        table_settings.data.sample_rate = Some(10.0);
        engine.analyses.insert(
            7,
            decode_analysis("data.csv", csv, &table_settings, None).unwrap(),
        );

        let order_json = serde_json::json!({
            "parentId": 7,
            "settings": OrderCutSettings {
                orders: "1".into(),
                rpm: fft_core::settings::RpmSettings {
                    source: RpmSource::Constant,
                    constant: 600.0,
                    ..Default::default()
                },
                ..Default::default()
            }
        });
        let cuts = engine.order_cuts_inner(&order_json.to_string()).unwrap();
        let cuts_value: serde_json::Value = serde_json::from_str(&cuts).unwrap();
        assert!(cuts_value["cuts"].as_array().is_some());

        let line_json = serde_json::json!({
            "parentId": 7,
            "settings": LineCutSettings::default(),
            "lines": [{
                "t0": 0.0,
                "f0": 2.0,
                "t1": 0.3,
                "f1": 2.0,
                "logF": false
            }]
        });
        let line = engine.line_cuts_inner(&line_json.to_string()).unwrap();
        let line_value: serde_json::Value = serde_json::from_str(&line).unwrap();
        let cuts = line_value["cuts"].as_array().unwrap();
        assert_eq!(cuts.len(), 1);
        let csv = engine
            .cuts_csv_inner(&serde_json::json!({ "cuts": cuts }).to_string())
            .unwrap();
        assert!(csv.starts_with("kind,value,time_s,rpm,freq_hz,db"));
    }

    #[test]
    fn themes_list_and_duplicate_with_custom_toml() {
        let custom = serde_json::json!({
            "ocean": "name = \"Ocean\"\nextends = \"dark\"\n"
        })
        .to_string();
        let list = engine_list(&custom);
        let entries: Vec<serde_json::Value> = serde_json::from_str(&list).unwrap();
        assert!(entries.iter().any(|entry| entry["id"] == "dark"));
        assert!(entries.iter().any(|entry| entry["id"] == "ocean"));

        let engine = FftWasm::new();
        let theme = engine.theme_get_inner("ocean", &custom).unwrap();
        assert!(theme.contains("\"appearance\":\"dark\""));
        let duplicate = engine
            .theme_duplicate_inner("ocean", "Ocean Copy", &custom)
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&duplicate).unwrap();
        assert_eq!(value["id"], "ocean-copy");
        assert!(value["toml"]
            .as_str()
            .unwrap()
            .contains("name = \"Ocean Copy\""));
    }

    fn engine_list(custom: &str) -> String {
        FftWasm::new().theme_list_inner(custom).unwrap()
    }

    impl FftWasm {
        fn open_inner_for_test(&mut self, name: &str, bytes: &[u8]) -> Result<(), String> {
            let settings = settings();
            let stored = decode_analysis(name, bytes, &settings, None)?;
            self.analyses.insert(1, stored);
            Ok(())
        }
    }
}
