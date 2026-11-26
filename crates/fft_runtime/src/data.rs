//! Typed requests, summaries, and runtime-owned data payloads.

use std::path::PathBuf;
use std::sync::Arc;

use fft_core::orders::LineSpec;
use fft_core::settings::{AnalysisSettings, LineCutSettings, OrderCutSettings};
use fft_core::spectrogram::Spectrogram;
use fft_core::table::TableInfo;
use serde::{Deserialize, Serialize};

use crate::ids::{AnalysisId, CutId, SignalId};

// ------------------------------------------------------------------ requests ---

/// Load a signal from a file path (audio or a data table).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadFileRequest {
    pub path: PathBuf,
    /// Display/script name; defaults to the file name.
    pub name: Option<String>,
}

impl LoadFileRequest {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            name: None,
        }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }
}

/// Load already-decoded mono samples at a known sample rate.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadSignalRequest {
    pub samples: Vec<f32>,
    pub sample_rate: f64,
    pub name: Option<String>,
}

impl LoadSignalRequest {
    pub fn new(samples: Vec<f32>, sample_rate: f64) -> Self {
        Self {
            samples,
            sample_rate,
            name: None,
        }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }
}

/// Explicit RPM profile data for an order cut.
///
/// `None` on [`OrderCutRequest`] means "use `OrderCutSettings.rpm`". A request
/// may override it when a script already has RPM values (for example a
/// fundamental-frequency array from Python).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum RpmProfileSpec {
    Constant(f64),
    Ramp {
        t0_s: f64,
        t1_s: f64,
        rpm0: f64,
        rpm1: f64,
    },
    Csv(PathBuf),
    Points(Vec<RpmPoint>),
}

/// One `(time, rpm)` sample.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpmPoint {
    pub time_s: f64,
    pub rpm: f64,
}

/// Request for one order-cut result (all requested order tracks share a
/// `CutId`). Fixed frequencies remain in `settings.freqs`, matching the
/// existing UI/settings surface.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OrderCutRequest {
    pub settings: OrderCutSettings,
    /// Optional explicit RPM source; `None` uses `settings.rpm`.
    pub rpm: Option<RpmProfileSpec>,
}

impl Default for OrderCutRequest {
    fn default() -> Self {
        Self {
            settings: OrderCutSettings::default(),
            rpm: None,
        }
    }
}

impl From<OrderCutSettings> for OrderCutRequest {
    fn from(settings: OrderCutSettings) -> Self {
        Self {
            settings,
            rpm: None,
        }
    }
}

/// Request for one line-cut result. Each line becomes one contiguous
/// [`LineCutData`] track in the returned [`CutData::Line`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LineCutRequest {
    pub settings: LineCutSettings,
    pub lines: Vec<LineSpec>,
}

impl Default for LineCutRequest {
    fn default() -> Self {
        Self {
            settings: LineCutSettings::default(),
            lines: Vec::new(),
        }
    }
}

impl LineCutRequest {
    pub fn new(settings: LineCutSettings, lines: Vec<LineSpec>) -> Self {
        Self { settings, lines }
    }
}

// ------------------------------------------------------------------- summaries ---

/// Typed summary of a loaded signal.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignalInfo {
    pub id: SignalId,
    pub name: String,
    /// `None` for in-memory samples.
    pub path: Option<PathBuf>,
    pub sample_rate: f64,
    pub sample_count: usize,
    pub duration_s: f64,
}

/// Typed summary of a completed analysis.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisInfo {
    pub id: AnalysisId,
    pub signal: SignalId,
    pub name: String,
    pub path: Option<PathBuf>,
    pub sample_rate: f64,
    pub frame_s: f64,
    pub frames: usize,
    pub bins: usize,
    pub bin_hz: f32,
    pub hop_s: f64,
    pub duration_s: f64,
    pub nyquist: f32,
    pub max_db: f32,
    pub min_db: f32,
    pub table: Option<TableInfo>,
}

/// Runtime result category. Unlike `fft_core::orders::CutKind`, this describes
/// a whole result set, not one curve.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CutKind {
    Order,
    Line,
}

/// Typed summary of a cut result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CutInfo {
    pub id: CutId,
    pub analysis: AnalysisId,
    pub kind: CutKind,
    pub name: String,
    /// Number of order tracks (`CutKind::Order`) or line tracks (`CutKind::Line`).
    pub tracks: usize,
}

// --------------------------------------------------------------- spectrogram ---

/// Runtime-owned spectrogram snapshot handed to displays and language
/// bindings through `Arc`.
///
/// The payload intentionally includes the analysis settings and source
/// identity so a child display can recompute cuts without asking the adapter.
/// The underlying [`Spectrogram`] stores contiguous frame-major buffers; this
/// `Arc`-based wrapper adds the source identity and generation.
pub struct SpectrogramData {
    /// Full-resolution analysed data.
    pub spectrogram: Spectrogram,
    /// Display/script name of the source.
    pub name: String,
    /// `None` for in-memory signals.
    pub path: Option<PathBuf>,
    pub sample_rate: f64,
    /// Present when the source was a data table.
    pub table: Option<TableInfo>,
    /// Increases when data is replaced, so children can detect staleness.
    pub generation: u64,
    pub analysis: AnalysisSettings,
}

impl std::fmt::Debug for SpectrogramData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpectrogramData")
            .field("name", &self.name)
            .field("path", &self.path)
            .field("sample_rate", &self.sample_rate)
            .field("frames", &self.spectrogram.len())
            .field("bins", &self.spectrogram.nbins())
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

impl SpectrogramData {
    /// Contiguous frame-major dB values, `frames * bins` entries.
    pub fn data(&self) -> &[f32] {
        self.spectrogram.data()
    }

    /// Number of stored frames.
    pub fn frames(&self) -> usize {
        self.spectrogram.frames()
    }

    /// Number of frequency bins per frame.
    pub fn bins(&self) -> usize {
        self.spectrogram.bins()
    }

    /// Build the typed summary for this snapshot.
    pub fn info(&self, id: AnalysisId, signal: SignalId) -> AnalysisInfo {
        let spec = &self.spectrogram;
        let mut min_db = f32::INFINITY;
        for &value in spec.data() {
            if value.is_finite() {
                min_db = min_db.min(value);
            }
        }
        if !min_db.is_finite() {
            min_db = 0.0;
        }
        let max_db = if spec.max_db().is_finite() {
            spec.max_db()
        } else {
            0.0
        };
        let frame_s = if self.sample_rate > 0.0 {
            self.analysis.frame as f64 / self.sample_rate
        } else {
            0.0
        };
        AnalysisInfo {
            id,
            signal,
            name: self.name.clone(),
            path: self.path.clone(),
            sample_rate: self.sample_rate,
            frame_s,
            frames: spec.len(),
            bins: spec.nbins(),
            bin_hz: spec.bin_hz(),
            hop_s: spec.hop_s(),
            duration_s: spec.end_s(),
            nyquist: spec.nyquist(),
            max_db,
            min_db,
            table: self.table.clone(),
        }
    }
}

// ----------------------------------------------------------------- cut data ---

/// Level matrix for order cuts: `frames × order_count` dB values, row-major.
///
/// All buffers are `Arc<Vec<_>>` so language bindings can expose read-only
/// zero-copy views over contiguous storage while the runtime keeps ownership.
#[derive(Clone, Debug, PartialEq)]
pub struct OrderCutData {
    pub frames: usize,
    pub order_count: usize,
    /// Loudest level for each `(frame, order)`, row-major, `frames * order_count`.
    pub values: Arc<Vec<f32>>,
    /// Frame-centre times, one per frame.
    pub times: Arc<Vec<f64>>,
    /// Order numbers, one per column.
    pub orders: Arc<Vec<f64>>,
    /// Frequency actually read for each `(frame, order)`, row-major.
    pub frequencies: Arc<Vec<f32>>,
}

impl OrderCutData {
    pub fn new(values: Vec<f32>, times: Vec<f64>, orders: Vec<f64>, frequencies: Vec<f32>) -> Self {
        let frames = times.len();
        let order_count = orders.len();
        debug_assert_eq!(values.len(), frames * order_count);
        debug_assert_eq!(frequencies.len(), frames * order_count);
        Self {
            frames,
            order_count,
            values: Arc::new(values),
            times: Arc::new(times),
            orders: Arc::new(orders),
            frequencies: Arc::new(frequencies),
        }
    }

    pub fn is_consistent(&self) -> bool {
        let cells = self.frames * self.order_count;
        self.values.len() == cells
            && self.frequencies.len() == cells
            && self.times.len() == self.frames
            && self.orders.len() == self.order_count
    }

    /// `(frame, order)` level, if the cell exists.
    pub fn value(&self, frame: usize, order: usize) -> Option<f32> {
        if frame >= self.frames || order >= self.order_count {
            return None;
        }
        self.values.get(frame * self.order_count + order).copied()
    }
}

/// One contiguous line-cut track. `times`, `frequencies`, and `fractions` share
/// a length; `db` is the same length.
#[derive(Clone, Debug, PartialEq)]
pub struct LineCutData {
    pub times: Arc<Vec<f64>>,
    pub frequencies: Arc<Vec<f64>>,
    /// Normalised position along the requested line, 0..=1.
    pub fractions: Arc<Vec<f64>>,
    pub db: Arc<Vec<f32>>,
}

impl LineCutData {
    pub fn new(times: Vec<f64>, frequencies: Vec<f64>, fractions: Vec<f64>, db: Vec<f32>) -> Self {
        debug_assert_eq!(times.len(), frequencies.len());
        debug_assert_eq!(times.len(), fractions.len());
        debug_assert_eq!(times.len(), db.len());
        Self {
            times: Arc::new(times),
            frequencies: Arc::new(frequencies),
            fractions: Arc::new(fractions),
            db: Arc::new(db),
        }
    }

    pub fn len(&self) -> usize {
        self.times.len()
    }

    pub fn is_empty(&self) -> bool {
        self.times.is_empty()
    }
}

/// Materialized cut result.
///
/// Order cuts keep all requested order tracks in one matrix. Line cuts keep
/// one track per requested line, in request order; the common scripting case
/// has exactly one element.
#[derive(Clone, Debug, PartialEq)]
pub enum CutData {
    Order(OrderCutData),
    Line(Vec<LineCutData>),
}

impl CutData {
    pub const fn kind(&self) -> CutKind {
        match self {
            CutData::Order(_) => CutKind::Order,
            CutData::Line(_) => CutKind::Line,
        }
    }

    pub fn tracks(&self) -> usize {
        match self {
            CutData::Order(data) => data.order_count,
            CutData::Line(lines) => lines.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_matrix_is_contiguous_and_indexes_by_frame_major() {
        let data = OrderCutData::new(
            vec![1.0, 2.0, 3.0, 4.0],
            vec![0.0, 0.5],
            vec![1.0, 2.0],
            vec![10.0, 20.0, 10.0, 20.0],
        );
        assert_eq!(data.frames, 2);
        assert_eq!(data.order_count, 2);
        assert_eq!(data.value(0, 1), Some(2.0));
        assert_eq!(data.value(1, 0), Some(3.0));
        assert_eq!(data.value(2, 0), None);
        assert!(data.is_consistent());
        assert_eq!(data.values.len(), data.frames * data.order_count);
    }

    #[test]
    fn line_tracks_keep_parallel_buffers() {
        let track = LineCutData::new(
            vec![0.0, 1.0],
            vec![10.0, 20.0],
            vec![0.0, 1.0],
            vec![-1.0, -2.0],
        );
        assert_eq!(track.len(), 2);
        assert!(!track.is_empty());
        let data = CutData::Line(vec![track]);
        assert_eq!(data.kind(), CutKind::Line);
        assert_eq!(data.tracks(), 1);
    }

    #[test]
    fn analysis_summary_is_typed_json() {
        let info = AnalysisInfo {
            id: AnalysisId(1),
            signal: SignalId(2),
            name: "tone".into(),
            path: None,
            sample_rate: 8_000.0,
            frame_s: 0.25,
            frames: 4,
            bins: 8,
            bin_hz: 500.0,
            hop_s: 0.0625,
            duration_s: 1.0,
            nyquist: 4_000.0,
            max_db: 0.0,
            min_db: -80.0,
            table: None,
        };
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(json["sampleRate"], 8_000.0);
        assert_eq!(json["signal"], 2);
        assert!(json["table"].is_null());
        let back: AnalysisInfo = serde_json::from_value(json).unwrap();
        assert_eq!(back, info);
    }

    #[test]
    fn requests_default_to_the_current_settings_types() {
        let order = OrderCutRequest::default();
        assert_eq!(order.settings, OrderCutSettings::default());
        assert!(order.rpm.is_none());

        let line = LineCutRequest::new(
            LineCutSettings::default(),
            vec![LineSpec {
                t0: 0.0,
                f0: 100.0,
                t1: 1.0,
                f1: 200.0,
                log_f: false,
            }],
        );
        assert_eq!(line.lines.len(), 1);
    }
}
