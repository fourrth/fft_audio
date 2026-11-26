//! All user-tunable settings, saved to / loaded from a TOML file.
//!
//! Default location: `<config dir>/fft_audio/settings.toml`
//! (`~/.config` on Linux, `~/Library/Application Support` on macOS,
//! `%APPDATA%` on Windows). Headless overrides the root with `--config-dir`.
//!
//! Every field has a default, so partial or older files still load.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::stages::Window;
use crate::traits::Result;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WindowKind {
    Hann,
    Hamming,
    Blackman,
    Nuttall,
    Triangular,
    Rectangular,
    /// No windowing; identical to a rectangular/identity window.
    None,
}

impl WindowKind {
    pub const ALL: [WindowKind; 7] = [
        WindowKind::Hann,
        WindowKind::Hamming,
        WindowKind::Blackman,
        WindowKind::Nuttall,
        WindowKind::Triangular,
        WindowKind::Rectangular,
        WindowKind::None,
    ];

    pub fn name(self) -> &'static str {
        match self {
            WindowKind::Hann => "hann",
            WindowKind::Hamming => "hamming",
            WindowKind::Blackman => "blackman",
            WindowKind::Nuttall => "nuttall",
            WindowKind::Triangular => "triangular",
            WindowKind::Rectangular => "rectangular",
            WindowKind::None => "none",
        }
    }

    pub fn stage(self) -> Window {
        match self {
            WindowKind::Hann => Window::hann(),
            WindowKind::Hamming => Window::hamming(),
            WindowKind::Blackman => Window::blackman(),
            WindowKind::Nuttall => Window::nuttall(),
            WindowKind::Triangular => Window::triangular(),
            WindowKind::Rectangular | WindowKind::None => Window::rectangular(),
        }
    }
}

/// What the x axis of the cut plot shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CutAxis {
    Rpm,
    Time,
    /// Frequency of the order at that moment (`order * rpm / 60`).
    /// Fixed-frequency cuts have no meaningful value here and are hidden.
    Freq,
}

/// x axis of the line-cut plot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LineAxis {
    /// Time, unless most lines are (nearly) vertical, then frequency.
    Auto,
    Time,
    Freq,
}

/// How a column of a data table (csv, tsv, parquet, json, ...) becomes the
/// signal that is analysed. Ignored for audio files.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DataSettings {
    /// Column holding the signal (absent = the first numeric column that is not the time column).
    pub column: Option<String>,
    /// Column holding the sample times in seconds (absent = look for one named
    /// `time`, `t`, `seconds`, ...). The sample rate is derived from it.
    pub time_column: Option<String>,
    /// Fixed sample rate in Hz. Wins over the time column; with neither, 1 Hz is assumed.
    pub sample_rate: Option<f64>,
    /// Subtract the mean first. Tables usually carry an offset (a temperature of
    /// 20 degrees) that would otherwise swamp the spectrum.
    pub remove_mean: bool,
}

impl Default for DataSettings {
    fn default() -> Self {
        Self {
            column: None,
            time_column: None,
            sample_rate: None,
            remove_mean: true,
        }
    }
}

/// Settings that change the FFT itself: changing them means re-analysing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AnalysisSettings {
    /// Samples per frame.
    pub frame: usize,
    /// Samples between frames (absent = frame / 4).
    pub hop: Option<usize>,
    /// FFT size = frame * pad (zero padding).
    pub pad: usize,
    pub window: WindowKind,
    pub highpass: Option<f32>,
    pub lowpass: Option<f32>,
    /// Exponential averaging of the spectrogram over time: time constant in
    /// seconds (0 = off). Reduces noise; smears fast changes.
    pub smooth_s: f32,
    /// Which column of a data table is the signal (not used for audio).
    pub data: DataSettings,
}

impl Default for AnalysisSettings {
    fn default() -> Self {
        Self {
            frame: 2048,
            hop: None,
            pad: 2,
            window: WindowKind::Hann,
            highpass: None,
            lowpass: None,
            smooth_s: 0.0,
            data: DataSettings::default(),
        }
    }
}

impl AnalysisSettings {
    /// The hop actually used, clamped to `1..=frame`.
    pub fn hop_samples(&self) -> usize {
        self.hop
            .unwrap_or(self.frame / 4)
            .clamp(1, self.frame.max(1))
    }

    pub fn validate(&self) -> Result<()> {
        if self.frame < 16 {
            return Err("frame must be at least 16 samples".into());
        }
        if self.pad < 1 {
            return Err("pad must be at least 1".into());
        }
        if !(self.smooth_s >= 0.0 && self.smooth_s.is_finite()) {
            return Err("smoothing time constant must be >= 0".into());
        }
        if let Some(sr) = self.data.sample_rate {
            if !(sr.is_finite() && sr > 0.0) {
                return Err("sample rate must be a positive number".into());
            }
        }
        if let Some(h) = self.hop {
            if h == 0 || h > self.frame {
                return Err("hop must be between 1 and frame".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplaySettings {
    pub log_freq: bool,
    /// Draw tick-aligned grid lines behind the plot data.
    pub show_grid: bool,
    /// Upper limit of the frequency axis (absent = Nyquist).
    pub max_hz: Option<f32>,
    /// Lower limit of the frequency axis and the log-axis floor (absent = the
    /// renderer's default floor).
    pub min_hz: Option<f32>,
    /// color scale span below the top. Used when only `db_max` is set; when
    /// both `db_min` and `db_max` are set their difference is the span.
    pub range_db: f32,
    /// Absolute bottom of the color scale (absent = relative to the top).
    pub db_min: Option<f32>,
    /// Absolute top of the color scale (absent = the loudest bin).
    pub db_max: Option<f32>,
}

impl Default for DisplaySettings {
    fn default() -> Self {
        Self {
            log_freq: true,
            show_grid: true,
            max_hz: None,
            min_hz: None,
            range_db: 80.0,
            db_min: None,
            db_max: None,
        }
    }
}

impl DisplaySettings {
    /// Reject nonsensical limits instead of silently clamping them.
    ///
    /// All present values must be finite; `min_hz >= 0`, `max_hz > min_hz`
    /// (when both are present), `range_db > 0`, and `db_min < db_max` (when
    /// both are present).
    pub fn validate(&self) -> Result<()> {
        if !(self.range_db.is_finite() && self.range_db > 0.0) {
            return Err("display range must be a positive number of dB".into());
        }
        if let Some(hz) = self.min_hz {
            if !hz.is_finite() {
                return Err("display minimum frequency must be finite".into());
            }
            if hz < 0.0 {
                return Err("display minimum frequency must be >= 0".into());
            }
        }
        if let Some(hz) = self.max_hz {
            if !hz.is_finite() {
                return Err("display maximum frequency must be finite".into());
            }
            if hz <= 0.0 {
                return Err("display maximum frequency must be > 0".into());
            }
        }
        if let (Some(lo), Some(hi)) = (self.min_hz, self.max_hz) {
            if hi <= lo {
                return Err("display maximum frequency must be greater than the minimum".into());
            }
        }
        if let Some(db) = self.db_min {
            if !db.is_finite() {
                return Err("display minimum level must be finite".into());
            }
        }
        if let Some(db) = self.db_max {
            if !db.is_finite() {
                return Err("display maximum level must be finite".into());
            }
        }
        if let (Some(lo), Some(hi)) = (self.db_min, self.db_max) {
            if lo >= hi {
                return Err("display maximum level must be greater than the minimum".into());
            }
        }
        Ok(())
    }
}

/// Properties of an **Order Cuts** display.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OrderCutSettings {
    /// Orders to cut, e.g. `"1, 2, 4.5"`.
    pub orders: String,
    /// Half width of the order window, in orders.
    pub order_width: f64,
    /// Half width in Hz. When set, it replaces `order_width` (the window no
    /// longer scales with RPM).
    pub order_width_hz: Option<f64>,
    /// Fixed frequencies to cut, in Hz, e.g. `"120, 440"`.
    pub freqs: String,
    /// Half width of the fixed-frequency window, in Hz.
    pub freq_width_hz: f64,
    pub axis: CutAxis,
    /// Extra exponential smoothing of the cut curves along time (time constant
    /// in seconds, 0 = off). This comes *on top of* whatever smoothing the
    /// parent display already applied to the data it hands down.
    pub smooth_s: f64,
    /// Hovering a curve snaps the read-out to a nearby peak / resonance.
    pub peak_snap: bool,
    /// Smallest prominence (dB) that counts as a peak when snapping.
    pub peak_min_db: f64,
    /// Where the RPM comes from (order cuts follow `order * rpm / 60` Hz).
    pub rpm: RpmSettings,
}

impl Default for OrderCutSettings {
    fn default() -> Self {
        Self {
            orders: "1,2,4".into(),
            order_width: 0.2,
            order_width_hz: None,
            freqs: String::new(),
            freq_width_hz: 5.0,
            axis: CutAxis::Rpm,
            smooth_s: 0.0,
            peak_snap: true,
            peak_min_db: 1.0,
            rpm: RpmSettings::default(),
        }
    }
}

/// Properties of a **Line Cuts** display.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LineCutSettings {
    /// Half width (Hz) of the peak window. 0 = plain bilinear interpolation
    /// of the spectrogram along the line.
    pub line_width_hz: f64,
    /// Exact total number of points to sample per line (1..=100_000). `None`
    /// keeps the automatic density that touches every frame and bin.
    pub samples: Option<usize>,
    pub line_axis: LineAxis,
    /// While drawing / dragging a line, snap it to exactly horizontal or
    /// vertical when it is within `snap_deg` degrees (on screen) of one.
    pub snap: bool,
    pub snap_deg: f64,
    /// Hovering a curve snaps the read-out to a nearby peak / resonance.
    pub peak_snap: bool,
    /// Smallest prominence (dB) that counts as a peak when snapping.
    pub peak_min_db: f64,
}

impl Default for LineCutSettings {
    fn default() -> Self {
        Self {
            line_width_hz: 0.0,
            samples: None,
            line_axis: LineAxis::Auto,
            snap: true,
            snap_deg: 2.0,
            peak_snap: true,
            peak_min_db: 1.0,
        }
    }
}

/// Where the RPM comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RpmSource {
    /// One fixed RPM for the whole file (e.g. a steady drive at 2400 rpm).
    Constant,
    /// Linear ramp from `start` to `end` over the whole file.
    Ramp,
    /// `time_s,rpm` file.
    Csv,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RpmSettings {
    pub source: RpmSource,
    pub constant: f64,
    pub start: f64,
    pub end: f64,
    pub csv: Option<PathBuf>,
}

impl Default for RpmSettings {
    fn default() -> Self {
        Self {
            source: RpmSource::Constant,
            constant: 2000.0,
            start: 1000.0,
            end: 3000.0,
            csv: None,
        }
    }
}

/// Properties of the **Main** window (the root of the display hierarchy).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionSettings {
    /// On start-up, reopen the displays (and their layout) that were open
    /// last time. With no saved session the window simply starts empty.
    pub reopen_last: bool,
}

impl Default for SessionSettings {
    fn default() -> Self {
        Self { reopen_last: true }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppearanceSettings {
    /// Theme id (`dark`, `light`, or a user theme file stem).
    pub theme: String,
    /// UI scale as a fraction (0.75 ..= 2.0).
    pub ui_scale: f32,
    pub reduce_motion: bool,
    pub high_contrast: bool,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            theme: "dark".into(),
            ui_scale: 1.0,
            reduce_motion: false,
            high_contrast: false,
        }
    }
}

/// The settings file. `analysis`, `display`, `order_cuts` and `line_cuts` are
/// the **defaults for newly opened displays** (each display then has its own
/// copy, edited in its Inspector); `session` and `appearance` belong to the
/// app.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub analysis: AnalysisSettings,
    pub display: DisplaySettings,
    pub order_cuts: OrderCutSettings,
    pub line_cuts: LineCutSettings,
    pub session: SessionSettings,
    pub appearance: AppearanceSettings,
}

impl Settings {
    pub fn default_path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("fft_audio").join("settings.toml"))
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&text)?)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir)?;
            }
        }
        std::fs::write(path, toml::to_string_pretty(self)?)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_toml() {
        let mut s = Settings::default();
        s.analysis.frame = 8192;
        s.analysis.highpass = Some(30.0);
        s.analysis.smooth_s = 0.25;
        s.order_cuts.freqs = "120, 440".into();
        s.order_cuts.axis = CutAxis::Time;
        s.order_cuts.smooth_s = 0.1;
        s.order_cuts.peak_snap = false;
        s.order_cuts.rpm.source = RpmSource::Csv;
        s.order_cuts.rpm.csv = Some(PathBuf::from("rpm.csv"));
        s.line_cuts.line_axis = LineAxis::Freq;
        s.line_cuts.line_width_hz = 3.0;
        s.line_cuts.snap = false;
        s.line_cuts.snap_deg = 8.0;
        s.line_cuts.peak_min_db = 2.5;
        s.session.reopen_last = false;
        s.appearance = AppearanceSettings {
            theme: "light".into(),
            ui_scale: 1.25,
            reduce_motion: true,
            high_contrast: true,
        };
        s.analysis.data = DataSettings {
            column: Some("ax".into()),
            time_column: Some("t".into()),
            sample_rate: Some(0.5),
            remove_mean: false,
        };
        let text = toml::to_string_pretty(&s).unwrap();
        assert!(!text.contains("[scripting]"));
        let back: Settings = toml::from_str(&text).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn settings_with_a_removed_scripting_table_still_load() {
        let old = "[scripting]\ncode_path = \"/opt/editor/bin/code\"\n";
        let s: Settings = toml::from_str(old).unwrap();
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn default_settings_do_not_serialize_a_scripting_table() {
        let text = toml::to_string_pretty(&Settings::default()).unwrap();
        assert!(!text.contains("[scripting]"));
    }

    #[test]
    fn none_window_round_trips_and_default_stays_hann() {
        let mut settings = Settings::default();
        settings.analysis.window = WindowKind::None;
        let text = toml::to_string_pretty(&settings).unwrap();
        assert!(text.contains("window = \"none\""));
        let back: Settings = toml::from_str(&text).unwrap();
        assert_eq!(back.analysis.window, WindowKind::None);
        assert_eq!(WindowKind::None.name(), "none");
        assert_eq!(AnalysisSettings::default().window, WindowKind::Hann);
    }

    #[test]
    fn partial_file_uses_defaults() {
        let s: Settings = toml::from_str("[analysis]\nframe = 4096\n").unwrap();
        assert_eq!(s.analysis.frame, 4096);
        assert_eq!(s.analysis.pad, 2);
        assert_eq!(s.order_cuts, OrderCutSettings::default());
        assert_eq!(s.line_cuts, LineCutSettings::default());
        assert!(
            s.session.reopen_last,
            "reopening the last session is on by default"
        );
        assert_eq!(s.appearance.theme, "dark");
        assert!((s.appearance.ui_scale - 1.0).abs() < 1e-6);
    }

    #[test]
    fn unknown_setting_tables_are_ignored() {
        // Files written by older versions keep loading; unknown tables and
        // fields are ignored while known values are kept.
        let old = "[analysis]\nframe = 1024\n[cuts]\norders = \"3\"\n[rpm]\nsource = \"ramp\"\n";
        let s: Settings = toml::from_str(old).unwrap();
        assert_eq!(s.analysis.frame, 1024);
    }

    #[test]
    fn display_fields_default_for_old_files_and_round_trip() {
        let old: Settings =
            toml::from_str("[display]\nlog_freq = false\nmax_hz = 5000.0\nrange_db = 60.0\n")
                .unwrap();
        assert_eq!(old.display.min_hz, None);
        assert_eq!(old.display.db_min, None);
        assert_eq!(old.display.db_max, None);
        assert_eq!(old.display.max_hz, Some(5000.0));
        assert!(old.display.show_grid, "grid lines default on for old files");

        let mut s = Settings::default();
        s.display.min_hz = Some(20.0);
        s.display.db_min = Some(-100.0);
        s.display.db_max = Some(0.0);
        s.display.show_grid = false;
        let text = toml::to_string_pretty(&s).unwrap();
        assert!(text.contains("min_hz = 20.0"));
        assert!(text.contains("db_min = -100.0"));
        assert!(text.contains("db_max = 0.0"));
        assert!(text.contains("show_grid = false"));
        let back: Settings = toml::from_str(&text).unwrap();
        assert_eq!(back, s);
        assert_eq!(DisplaySettings::default().min_hz, None);
        assert!(DisplaySettings::default().show_grid);
    }

    #[test]
    fn display_validation_rejects_invalid_values_without_clamping() {
        let mut d = DisplaySettings::default();
        d.validate().unwrap();

        d.min_hz = Some(-1.0);
        assert!(d.validate().unwrap_err().to_string().contains("minimum"));
        d.min_hz = Some(f32::NAN);
        assert!(d.validate().is_err());
        d.min_hz = Some(100.0);
        d.max_hz = Some(100.0);
        assert!(d.validate().unwrap_err().to_string().contains("greater"));
        d.max_hz = Some(50.0);
        assert!(d.validate().is_err());
        d.max_hz = Some(f32::INFINITY);
        assert!(d.validate().is_err());

        let mut d = DisplaySettings::default();
        d.db_min = Some(0.0);
        d.db_max = Some(0.0);
        assert!(d.validate().unwrap_err().to_string().contains("greater"));
        d.db_max = Some(-1.0);
        assert!(d.validate().is_err());
        d.db_max = Some(f32::NAN);
        assert!(d.validate().is_err());

        let mut d = DisplaySettings::default();
        d.range_db = 0.0;
        assert!(d.validate().is_err());
        d.range_db = f32::INFINITY;
        assert!(d.validate().is_err());
    }

    #[test]
    fn order_width_hz_and_line_samples_are_backward_compatible() {
        let old: Settings =
            toml::from_str("[order_cuts]\norders = \"2\"\n[line_cuts]\nline_width_hz = 2.0\n")
                .unwrap();
        assert_eq!(old.order_cuts.order_width_hz, None);
        assert_eq!(old.line_cuts.samples, None);

        let mut s = Settings::default();
        s.order_cuts.order_width_hz = Some(12.5);
        s.line_cuts.samples = Some(256);
        let text = toml::to_string_pretty(&s).unwrap();
        assert!(text.contains("order_width_hz = 12.5"));
        assert!(text.contains("samples = 256"));
        let back: Settings = toml::from_str(&text).unwrap();
        assert_eq!(back, s);
        assert_eq!(OrderCutSettings::default().order_width_hz, None);
        assert_eq!(LineCutSettings::default().samples, None);
    }
}
