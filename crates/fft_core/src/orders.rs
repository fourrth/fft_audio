//! Cuts through the spectrogram.
//!
//! * **Order cut** – follows `order * rpm / 60` Hz as the RPM changes (a
//!   sloped line on the spectrogram). An order is a multiple of the shaft
//!   rotation frequency.
//! * **Frequency cut** – reads one fixed frequency over time (a horizontal
//!   line). Useful for resonances / structure-borne tones that do not move
//!   with RPM; plotting it against RPM shows where the machine excites it.
//!
//! * **Line cut** – a straight line you draw on the spectrogram, at any
//!   angle: vertical = the spectrum at one moment, horizontal = a frequency
//!   cut, sloped = something like an order with a constant sweep rate.
//!
//! Audio files carry no tachometer, so the RPM comes from an `RpmProfile`
//! (a CSV of `time_s,rpm`, or a linear ramp).

use std::io::Write;
use std::path::Path;

use crate::settings::{CutAxis, LineAxis};
use crate::spectrogram::Spectrogram;
use crate::traits::Result;

/// RPM as a function of time, linearly interpolated between points and held
/// constant before the first / after the last point.
#[derive(Clone, Debug)]
pub struct RpmProfile {
    pts: Vec<(f64, f64)>, // (time_s, rpm), sorted by time
}

impl RpmProfile {
    pub fn constant(rpm: f64) -> Self {
        Self {
            pts: vec![(0.0, rpm)],
        }
    }

    /// Straight line from `rpm0` at `t0` to `rpm1` at `t1`.
    pub fn ramp(t0: f64, t1: f64, rpm0: f64, rpm1: f64) -> Self {
        Self {
            pts: vec![(t0, rpm0), (t1.max(t0 + 1e-6), rpm1)],
        }
    }

    /// Read `time_s,rpm` lines (comma, semicolon, tab or space separated).
    /// A non-numeric header line and `#` comments are skipped.
    pub fn from_csv(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path)?;
        let mut pts = Vec::new();
        for (n, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut it = line
                .split(|c: char| c == ',' || c == ';' || c.is_whitespace())
                .filter(|s| !s.is_empty());
            let a = it.next().and_then(|s| s.parse::<f64>().ok());
            let b = it.next().and_then(|s| s.parse::<f64>().ok());
            match (a, b) {
                (Some(t), Some(r)) => pts.push((t, r)),
                _ if pts.is_empty() => continue, // header line
                _ => {
                    return Err(
                        format!("{}:{}: expected `time_s,rpm`", path.display(), n + 1).into(),
                    )
                }
            }
        }
        if pts.is_empty() {
            return Err(format!("{}: no `time_s,rpm` rows found", path.display()).into());
        }
        pts.sort_by(|a, b| a.0.total_cmp(&b.0));
        Ok(Self { pts })
    }

    /// Build a profile from explicit `(time_s, rpm)` points.
    ///
    /// Points are sorted by time. Times and RPM values must be finite, and
    /// duplicate timestamps are rejected (rather than making the interpolation
    /// depend on input order). A single point is a constant profile.
    pub fn from_points(mut points: Vec<(f64, f64)>) -> Result<Self> {
        if points.is_empty() {
            return Err("RPM profile needs at least one `(time_s, rpm)` point".into());
        }
        for (index, (time, rpm)) in points.iter().enumerate() {
            if !time.is_finite() {
                return Err(format!("RPM point {} has a non-finite time", index + 1).into());
            }
            if !rpm.is_finite() {
                return Err(format!("RPM point {} has a non-finite RPM", index + 1).into());
            }
        }
        points.sort_by(|a, b| a.0.total_cmp(&b.0));
        for pair in points.windows(2) {
            if pair[0].0 == pair[1].0 {
                return Err(format!(
                    "RPM profile has duplicate time {} s; keep one value per timestamp",
                    pair[0].0
                )
                .into());
            }
        }
        Ok(Self { pts: points })
    }

    pub fn len(&self) -> usize {
        self.pts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pts.is_empty()
    }

    /// True if the RPM never changes (so "level vs RPM" would be a single
    /// vertical line and level vs time is the useful view).
    pub fn is_constant(&self) -> bool {
        self.pts.iter().all(|p| (p.1 - self.pts[0].1).abs() < 1e-9)
    }

    pub fn rpm_at(&self, t: f64) -> f64 {
        let p = &self.pts;
        if t <= p[0].0 {
            return p[0].1;
        }
        let last = p[p.len() - 1];
        if t >= last.0 {
            return last.1;
        }
        let i = p.partition_point(|q| q.0 <= t); // first point after t, i >= 1
        let (a, b) = (p[i - 1], p[i]);
        a.1 + (b.1 - a.1) * (t - a.0) / (b.0 - a.0)
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CutPoint {
    pub time_s: f64,
    pub rpm: f64,
    /// Frequency that was read: `order * rpm / 60` for order cuts, the fixed
    /// frequency for frequency cuts.
    pub freq_hz: f64,
    pub db: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum CutKind {
    Order(f64),
    Freq(f64),
    /// A user-drawn line; the number is the line's 1-based id.
    Line(usize),
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Cut {
    pub kind: CutKind,
    pub points: Vec<CutPoint>,
}

impl Cut {
    /// Short name for legends: `2x` or `120 Hz`.
    pub fn label(&self) -> String {
        match self.kind {
            CutKind::Order(o) => format!("{o}x"),
            CutKind::Freq(f) => format!("{f} Hz"),
            CutKind::Line(id) => format!("line {id}"),
        }
    }
}

fn parse_positive(text: &str) -> Vec<f64> {
    text.split(|c: char| c == ',' || c == ';' || c.is_whitespace())
        .filter_map(|s| s.parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v > 0.0)
        .collect()
}

/// Parse `"1, 2 3.5"` into `[1.0, 2.0, 3.5]`; junk and non-positive values are dropped.
pub fn parse_orders(text: &str) -> Vec<f64> {
    parse_positive(text)
}

/// Parse a list of frequencies in Hz, same rules as `parse_orders`.
pub fn parse_freqs(text: &str) -> Vec<f64> {
    parse_positive(text)
}

/// Width of an order-cut window around the order line.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum CutWidth {
    /// Half width in orders; scales with RPM (`half_width * rpm / 60` Hz).
    Orders(f64),
    /// Fixed half width in Hz, independent of RPM.
    Hz(f64),
}

impl CutWidth {
    /// Half width in Hz at `rpm`. Negative widths are treated as zero.
    pub fn half_width_hz(self, rpm: f64) -> f64 {
        match self {
            CutWidth::Orders(orders) => (orders.max(0.0) * rpm / 60.0).max(0.0),
            CutWidth::Hz(hz) => hz.max(0.0),
        }
    }
}

/// Contiguous frame × order level matrix for runtime/Python array views.
///
/// `values` and `frequencies` are row-major (`frame * order_count + order`).
/// A cell whose order frequency is non-positive or above Nyquist is not read:
/// both `values` and `frequencies` are `NaN` there, so every row still has
/// exactly `order_count` entries.
#[derive(Clone, Debug, PartialEq)]
pub struct OrderMatrix {
    pub frames: usize,
    pub order_count: usize,
    /// dB level per cell, `frames * order_count`, frame-major.
    pub values: Vec<f32>,
    /// Frame-centre times, one per frame.
    pub times: Vec<f64>,
    /// Order numbers, one per column.
    pub orders: Vec<f64>,
    /// Frequency read per cell in Hz, `frames * order_count`, frame-major.
    pub frequencies: Vec<f32>,
}

impl OrderMatrix {
    /// `(frame, order)` level, if the cell exists.
    pub fn value(&self, frame: usize, order: usize) -> Option<f32> {
        if frame >= self.frames || order >= self.order_count {
            return None;
        }
        self.values.get(frame * self.order_count + order).copied()
    }

    /// Shape and buffer invariants hold.
    pub fn is_consistent(&self) -> bool {
        let cells = self.frames * self.order_count;
        self.values.len() == cells
            && self.frequencies.len() == cells
            && self.times.len() == self.frames
            && self.orders.len() == self.order_count
    }
}

/// Loudest bin of `col` in `[f_lo, f_hi]` Hz, widened to at least the bins
/// directly around the range so a narrow window still finds a value.
fn level(col: &[f32], bin_hz: f64, f_lo: f64, f_hi: f64) -> f32 {
    let hi = ((f_hi / bin_hz).ceil() as usize).min(col.len() - 1);
    let lo = ((f_lo / bin_hz).floor().max(0.0) as usize).min(hi);
    col[lo..=hi]
        .iter()
        .cloned()
        .fold(f32::NEG_INFINITY, f32::max)
}

/// Level of one order over time.
///
/// For every frame the loudest bin within `order ± half_width_orders` is
/// taken, so the cut keeps following the order even if the RPM estimate is
/// slightly off or the order smears during fast sweeps. Frames where the
/// order is above Nyquist (or the RPM is not positive) are skipped.
pub fn order_cut(spec: &Spectrogram, rpm: &RpmProfile, order: f64, half_width_orders: f64) -> Cut {
    order_cut_with_width(spec, rpm, order, CutWidth::Orders(half_width_orders))
}

/// [`order_cut`] with an explicit window width, either in orders (scales with
/// RPM) or fixed Hz.
pub fn order_cut_with_width(
    spec: &Spectrogram,
    rpm: &RpmProfile,
    order: f64,
    width: CutWidth,
) -> Cut {
    let mut points = Vec::new();
    if spec.nbins() > 0 {
        let nyq = spec.nyquist() as f64;
        let bin = spec.bin_hz() as f64;
        for i in 0..spec.len() {
            let t = spec.frame_center(i);
            let r = rpm.rpm_at(t);
            let f = order * r / 60.0;
            if f <= 0.0 || f > nyq {
                continue;
            }
            let half = width.half_width_hz(r);
            let db = level(spec.column(i), bin, f - half, f + half);
            points.push(CutPoint {
                time_s: t,
                rpm: r,
                freq_hz: f,
                db,
            });
        }
    }
    Cut {
        kind: CutKind::Order(order),
        points,
    }
}

/// Materialize the level of several orders as one contiguous frame × order
/// matrix. See [`OrderMatrix`] for the layout and `NaN` cells.
pub fn order_matrix(
    spec: &Spectrogram,
    rpm: &RpmProfile,
    orders: &[f64],
    width: CutWidth,
) -> OrderMatrix {
    let frames = spec.len();
    let order_count = orders.len();
    let mut values = vec![f32::NAN; frames * order_count];
    let mut frequencies = vec![f32::NAN; frames * order_count];
    let mut times = Vec::with_capacity(frames);
    let (nyq, bin) = (spec.nyquist() as f64, spec.bin_hz() as f64);
    for frame in 0..frames {
        let t = spec.frame_center(frame);
        let r = rpm.rpm_at(t);
        times.push(t);
        if spec.nbins() == 0 {
            continue;
        }
        let half = width.half_width_hz(r);
        for (column, &order) in orders.iter().enumerate() {
            let f = order * r / 60.0;
            if f <= 0.0 || f > nyq {
                continue;
            }
            let index = frame * order_count + column;
            values[index] = level(spec.column(frame), bin, f - half, f + half);
            frequencies[index] = f as f32;
        }
    }
    OrderMatrix {
        frames,
        order_count,
        values,
        times,
        orders: orders.to_vec(),
        frequencies,
    }
}

/// Level at one fixed frequency over time (loudest bin within
/// `freq ± half_width_hz`). The RPM at each frame is recorded so the cut can
/// also be plotted against RPM.
pub fn freq_cut(spec: &Spectrogram, rpm: &RpmProfile, freq_hz: f64, half_width_hz: f64) -> Cut {
    let mut points = Vec::new();
    if spec.nbins() > 0 && freq_hz > 0.0 && freq_hz <= spec.nyquist() as f64 {
        let bin = spec.bin_hz() as f64;
        let half = half_width_hz.max(0.0);
        for i in 0..spec.len() {
            let t = spec.frame_center(i);
            let db = level(spec.column(i), bin, freq_hz - half, freq_hz + half);
            points.push(CutPoint {
                time_s: t,
                rpm: rpm.rpm_at(t),
                freq_hz,
                db,
            });
        }
    }
    Cut {
        kind: CutKind::Freq(freq_hz),
        points,
    }
}

/// A straight line on the spectrogram from `(t0, f0)` to `(t1, f1)`
/// (seconds, Hz). `log_f` says in which space it is straight: drawn on a
/// linear-frequency view it is linear in Hz, drawn on a log-frequency view
/// the frequency changes geometrically, so the cut follows exactly what you
/// saw on screen.
///
/// Kept as a tiny type on purpose: later this can grow into a polyline that
/// follows an order through a changing RPM.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LineSpec {
    pub t0: f64,
    pub f0: f64,
    pub t1: f64,
    pub f1: f64,
    #[serde(alias = "log_f")]
    pub log_f: bool,
}

impl LineSpec {
    /// Point at fraction `s` (0..=1) along the line, as `(time_s, freq_hz)`.
    pub fn at(&self, s: f64) -> (f64, f64) {
        let t = self.t0 + (self.t1 - self.t0) * s;
        let f = if self.log_f && self.f0 > 0.0 && self.f1 > 0.0 {
            self.f0 * (self.f1 / self.f0).powf(s)
        } else {
            self.f0 + (self.f1 - self.f0) * s
        };
        (t, f)
    }

    /// `n + 1` points along the line (for drawing).
    pub fn path(&self, n: usize) -> Vec<(f64, f64)> {
        (0..=n).map(|k| self.at(k as f64 / n as f64)).collect()
    }
}

/// Read the spectrogram along `line`.
///
/// The line is sampled densely enough to touch every frame and every bin it
/// crosses, so a vertical line gives the full spectrum at that moment and a
/// horizontal one a level-vs-time curve. With `half_width_hz == 0` the value
/// is bilinearly interpolated; with a positive width it is the loudest bin
/// within `± half_width_hz` of the line in the nearest frame (useful for
/// sloped lines that only roughly follow a ridge, but it flattens vertical
/// lines, so leave it at 0 for those). Parts outside the data are skipped.
pub fn line_cut(
    spec: &Spectrogram,
    rpm: &RpmProfile,
    line: &LineSpec,
    half_width_hz: f64,
    id: usize,
) -> Cut {
    line_cut_with_samples(spec, rpm, line, half_width_hz, id, None)
}

/// [`line_cut`] with an optional exact sample count.
///
/// `samples = Some(n)` produces at most `n` output points spaced evenly along
/// the line: `n = 1` is the start point, `n > 1` has `n - 1` equal intervals
/// including both ends. `n` is clamped to `1..=100_000`. `None` keeps the
/// automatic density. Points outside the spectrogram are skipped, so a line
/// that leaves the data can return fewer than `n` points.
pub fn line_cut_with_samples(
    spec: &Spectrogram,
    rpm: &RpmProfile,
    line: &LineSpec,
    half_width_hz: f64,
    id: usize,
    samples: Option<usize>,
) -> Cut {
    let mut points = Vec::new();
    if spec.nbins() > 0 && !spec.is_empty() {
        let hop = spec.hop_s();
        let bin = spec.bin_hz() as f64;
        let (end, nyq) = (spec.end_s(), spec.nyquist() as f64);
        let fractions: Vec<f64> = match samples {
            Some(n) => {
                let n = n.clamp(1, 100_000);
                if n == 1 {
                    vec![0.0]
                } else {
                    (0..n).map(|k| k as f64 / (n - 1) as f64).collect()
                }
            }
            None => {
                let mut steps =
                    ((line.t1 - line.t0).abs() / hop).max((line.f1 - line.f0).abs() / bin);
                if line.log_f && line.f0 > 0.0 && line.f1 > 0.0 {
                    // Geometric spacing: the step is largest at the top frequency.
                    let top = line.f0.max(line.f1);
                    steps = steps.max((line.f1 / line.f0).ln().abs() * top / bin);
                }
                let steps = steps.ceil().clamp(1.0, 100_000.0) as usize;
                (0..=steps).map(|k| k as f64 / steps as f64).collect()
            }
        };
        for s in fractions {
            let (t, f) = line.at(s);
            let db = if half_width_hz > 0.0 {
                if t < 0.0 || t > end || f < 0.0 || f > nyq {
                    None
                } else {
                    let col = spec.column(spec.nearest_frame(t));
                    Some(level(col, bin, f - half_width_hz, f + half_width_hz))
                }
            } else {
                spec.db_at(t, f)
            };
            if let Some(db) = db {
                points.push(CutPoint {
                    time_s: t,
                    rpm: rpm.rpm_at(t),
                    freq_hz: f,
                    db,
                });
            }
        }
    }
    Cut {
        kind: CutKind::Line(id),
        points,
    }
}

// ---------------------------------------------------------- cut-plot axes ---

/// The x value of a cut point on the given axis.
pub fn cut_x(p: &CutPoint, axis: CutAxis) -> f64 {
    match axis {
        CutAxis::Rpm => p.rpm,
        CutAxis::Time => p.time_s,
        CutAxis::Freq => p.freq_hz,
    }
}

/// Which axis the order/frequency cut plot really uses. With a constant RPM
/// "vs RPM" and "vs order frequency" would collapse every order cut onto a
/// single vertical line, so the plot shows level vs time instead; the
/// returned text explains why (so the radio buttons do not seem dead).
pub fn resolve_axis(requested: CutAxis, rpm_constant: bool) -> (CutAxis, Option<&'static str>) {
    if rpm_constant && requested != CutAxis::Time {
        (
            CutAxis::Time,
            Some("RPM is constant, so level is shown against time"),
        )
    } else {
        (requested, None)
    }
}

/// True if the cut does not extend along `axis` (so it cannot be drawn
/// there): e.g. a fixed-frequency cut on the frequency axis, a vertical line
/// on the time axis, or an empty cut. `hop_s` / `bin_hz` set what counts as
/// "no extent" on the time / frequency axis.
pub fn is_degenerate(cut: &Cut, axis: CutAxis, hop_s: f64, bin_hz: f64) -> bool {
    if cut.points.len() < 2 {
        return true;
    }
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for p in &cut.points {
        let x = cut_x(p, axis);
        lo = lo.min(x);
        hi = hi.max(x);
    }
    let tol = match axis {
        CutAxis::Rpm => 0.01,
        CutAxis::Time => 0.5 * hop_s,
        CutAxis::Freq => 0.5 * bin_hz,
    };
    hi - lo < tol
}

/// Axis for the line-cut plot. `Auto` picks the one on which more lines can
/// be drawn (time unless more lines are steeper than 45° in frame/bin terms).
pub fn resolve_line_axis(cuts: &[Cut], requested: LineAxis, hop_s: f64, bin_hz: f64) -> CutAxis {
    match requested {
        LineAxis::Time => CutAxis::Time,
        LineAxis::Freq => CutAxis::Freq,
        LineAxis::Auto => {
            let visible = |axis| {
                cuts.iter()
                    .filter(|c| !is_degenerate(c, axis, hop_s, bin_hz))
                    .count()
            };
            if visible(CutAxis::Freq) > visible(CutAxis::Time) {
                CutAxis::Freq
            } else {
                CutAxis::Time
            }
        }
    }
}

/// Exponential smoothing of a cut along time (power domain), time constant
/// `tau_s` seconds; uses the real time step between points, so gaps are
/// handled. `tau_s <= 0` leaves the cut untouched.
pub fn smooth_cut(cut: &mut Cut, tau_s: f64) {
    if tau_s <= 0.0 || cut.points.len() < 2 {
        return;
    }
    let mut state = 10f64.powf(cut.points[0].db as f64 / 10.0);
    let mut prev_t = cut.points[0].time_s;
    for p in cut.points.iter_mut().skip(1) {
        let a = 1.0 - (-(p.time_s - prev_t).max(0.0) / tau_s).exp();
        prev_t = p.time_s;
        state += a * (10f64.powf(p.db as f64 / 10.0) - state);
        p.db = (10.0 * state.max(1e-20).log10()) as f32;
    }
}

/// Long-format cut CSV: `kind,value,time_s,rpm,freq_hz,db`
/// (`kind` is `order`, `freq`, or `line`; `value` is the order number, Hz, or
/// line id).
pub fn csv_string(cuts: &[Cut]) -> Result<String> {
    let mut bytes = Vec::new();
    write_csv_to(&mut bytes, cuts)?;
    String::from_utf8(bytes).map_err(|error| error.into())
}

fn write_csv_to(mut writer: impl Write, cuts: &[Cut]) -> Result<()> {
    writeln!(writer, "kind,value,time_s,rpm,freq_hz,db")?;
    for cut in cuts {
        let (kind, value) = match cut.kind {
            CutKind::Order(o) => ("order", o),
            CutKind::Freq(f) => ("freq", f),
            CutKind::Line(id) => ("line", id as f64),
        };
        for p in &cut.points {
            writeln!(
                writer,
                "{},{},{:.6},{:.3},{:.3},{:.3}",
                kind, value, p.time_s, p.rpm, p.freq_hz, p.db
            )?;
        }
    }
    Ok(())
}

/// Write [`csv_string`] output to `path`.
pub fn write_csv(path: impl AsRef<Path>, cuts: &[Cut]) -> Result<()> {
    let mut writer = std::io::BufWriter::new(std::fs::File::create(path)?);
    write_csv_to(&mut writer, cuts)?;
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::Spectrum;
    use realfft::num_complex::Complex;

    /// 3 frames, 11 bins 10 Hz apart (0..=100 Hz), a 40 dB peak at 50 Hz.
    fn spec_with_peak() -> Spectrogram {
        let mut s = Spectrogram::new();
        s.set_frame_len_s(0.1);
        for i in 0..3 {
            let mut bins = vec![Complex::new(1.0f32, 0.0); 11];
            bins[5] = Complex::new(100.0, 0.0);
            s.push(&Spectrum {
                bins,
                bin_hz: 10.0,
                frame_index: i,
                time_s: i as f64 * 0.05,
            });
        }
        s
    }

    #[test]
    fn rpm_interpolates_and_clamps() {
        let p = RpmProfile::ramp(0.0, 10.0, 1000.0, 3000.0);
        assert_eq!(p.rpm_at(-5.0), 1000.0);
        assert!((p.rpm_at(5.0) - 2000.0).abs() < 1e-9);
        assert_eq!(p.rpm_at(99.0), 3000.0);
        assert_eq!(RpmProfile::constant(900.0).rpm_at(42.0), 900.0);
    }

    #[test]
    fn lists_parse() {
        assert_eq!(parse_orders("1, 2  3.5;x,-1,0"), vec![1.0, 2.0, 3.5]);
        assert_eq!(parse_freqs("120,440"), vec![120.0, 440.0]);
    }

    #[test]
    fn freq_cut_finds_the_peak() {
        let s = spec_with_peak();
        let rpm = RpmProfile::constant(3000.0);
        let on = freq_cut(&s, &rpm, 50.0, 0.0);
        assert_eq!(on.points.len(), 3);
        assert!((on.points[0].db - 40.0).abs() < 1e-3);
        assert_eq!(on.points[0].rpm, 3000.0);
        let off = freq_cut(&s, &rpm, 100.0, 0.0);
        assert!(off.points[0].db.abs() < 1e-3);
        assert!(freq_cut(&s, &rpm, 500.0, 0.0).points.is_empty()); // above Nyquist
    }

    #[test]
    fn constant_rpm_detected() {
        assert!(RpmProfile::constant(2400.0).is_constant());
        assert!(RpmProfile::ramp(0.0, 5.0, 2400.0, 2400.0).is_constant());
        assert!(!RpmProfile::ramp(0.0, 5.0, 1000.0, 2400.0).is_constant());
    }

    #[test]
    fn smoothing_lags_a_step() {
        let pt = |t: f64, db: f32| CutPoint {
            time_s: t,
            rpm: 0.0,
            freq_hz: 0.0,
            db,
        };
        let mut cut = Cut {
            kind: CutKind::Freq(1.0),
            points: vec![
                pt(0.0, 0.0),
                pt(1.0, 0.0),
                pt(2.0, 20.0),
                pt(3.0, 20.0),
                pt(4.0, 20.0),
            ],
        };
        smooth_cut(&mut cut, 1.0);
        let db: Vec<f32> = cut.points.iter().map(|p| p.db).collect();
        assert!(db[1].abs() < 1e-3);
        assert!(db[2] > 0.0 && db[2] < 20.0); // lags behind the step
        assert!(db[4] > db[3] && db[3] > db[2]); // and converges towards it
        let mut off = cut.clone();
        smooth_cut(&mut off, 0.0); // tau = 0 is a no-op
        assert_eq!(off.points[2].db, cut.points[2].db);
    }

    #[test]
    fn db_at_interpolates_and_bounds() {
        let s = spec_with_peak();
        let t = 0.10; // centre of the middle frame
        assert!((s.db_at(t, 50.0).unwrap() - 40.0).abs() < 1e-3);
        assert!((s.db_at(t, 45.0).unwrap() - 20.0).abs() < 1e-3); // halfway down the flank
        assert!(s.db_at(t, 10.0).unwrap().abs() < 1e-3);
        assert!(s.db_at(-0.1, 50.0).is_none());
        assert!(s.db_at(t, 500.0).is_none());
    }

    #[test]
    fn line_cut_vertical_is_a_spectrum() {
        let s = spec_with_peak();
        let rpm = RpmProfile::constant(3000.0);
        let v = line_cut(
            &s,
            &rpm,
            &LineSpec {
                t0: 0.1,
                f0: 0.0,
                t1: 0.1,
                f1: 100.0,
                log_f: false,
            },
            0.0,
            1,
        );
        assert_eq!(v.points.len(), 11); // one sample per bin
        let peak = v
            .points
            .iter()
            .max_by(|a, b| a.db.total_cmp(&b.db))
            .unwrap();
        assert!((peak.freq_hz - 50.0).abs() < 1e-9 && (peak.db - 40.0).abs() < 1e-3);
        assert_eq!(v.label(), "line 1");
    }

    #[test]
    fn axis_resolution() {
        assert_eq!(resolve_axis(CutAxis::Rpm, true).0, CutAxis::Time);
        assert!(resolve_axis(CutAxis::Rpm, true).1.is_some());
        assert_eq!(resolve_axis(CutAxis::Freq, false), (CutAxis::Freq, None));
        assert_eq!(resolve_axis(CutAxis::Time, true), (CutAxis::Time, None));
    }

    #[test]
    fn order_cut_follows_rpm() {
        let s = spec_with_peak();
        // 3000 rpm = 50 Hz, so order 1 sits on the peak and order 2 is at 100 Hz.
        let rpm = RpmProfile::constant(3000.0);
        assert!((order_cut(&s, &rpm, 1.0, 0.0).points[0].db - 40.0).abs() < 1e-3);
        assert!(order_cut(&s, &rpm, 2.0, 0.0).points[0].db.abs() < 1e-3);
        assert!(order_cut(&s, &rpm, 3.0, 0.0).points.is_empty()); // 150 Hz > Nyquist
    }

    #[test]
    fn hz_width_matches_order_width_at_a_known_rpm() {
        let s = spec_with_peak();
        let rpm = RpmProfile::constant(3000.0); // order 1 = 50 Hz
        let orders = order_cut(&s, &rpm, 1.0, 0.4); // ±0.4 order = ±20 Hz
        let hz = order_cut_with_width(&s, &rpm, 1.0, CutWidth::Hz(20.0));
        assert_eq!(orders, hz);
        assert_eq!(
            orders,
            order_cut_with_width(&s, &rpm, 1.0, CutWidth::Orders(0.4))
        );
    }

    #[test]
    fn a_width_wider_than_the_bin_spacing_finds_neighbours() {
        let s = spec_with_peak();
        let rpm = RpmProfile::constant(4800.0); // order 1 = 80 Hz
        let narrow = order_cut_with_width(&s, &rpm, 1.0, CutWidth::Hz(5.0));
        let wide = order_cut_with_width(&s, &rpm, 1.0, CutWidth::Hz(40.0)); // 40..120 Hz
        assert!(narrow.points[0].db.abs() < 1e-3);
        assert!((wide.points[0].db - 40.0).abs() < 1e-3);
    }

    #[test]
    fn zero_and_negative_widths_collapse_to_the_line() {
        let s = spec_with_peak();
        let rpm = RpmProfile::constant(3000.0);
        let zero = order_cut(&s, &rpm, 1.0, 0.0);
        assert_eq!(
            order_cut_with_width(&s, &rpm, 1.0, CutWidth::Hz(-5.0)),
            zero
        );
        assert_eq!(
            order_cut_with_width(&s, &rpm, 1.0, CutWidth::Orders(-0.5)),
            zero
        );
        assert_eq!(CutWidth::Orders(-0.5).half_width_hz(3000.0), 0.0);
        assert_eq!(CutWidth::Hz(-5.0).half_width_hz(3000.0), 0.0);
    }

    fn vertical(t0: f64, f0: f64, f1: f64) -> LineSpec {
        LineSpec {
            t0,
            f0,
            t1: t0,
            f1,
            log_f: false,
        }
    }

    #[test]
    fn exact_line_samples_cover_vertical_horizontal_and_log_lines() {
        let s = spec_with_peak();
        let rpm = RpmProfile::constant(0.0);

        let one = line_cut_with_samples(&s, &rpm, &vertical(0.1, 10.0, 100.0), 0.0, 1, Some(1));
        assert_eq!(one.points.len(), 1);
        assert_eq!(one.points[0].freq_hz, 10.0);
        assert_eq!(one.points[0].time_s, 0.1);
        let clamped = line_cut_with_samples(&s, &rpm, &vertical(0.1, 10.0, 100.0), 0.0, 1, Some(0));
        assert_eq!(clamped.points.len(), 1);

        let two = line_cut_with_samples(&s, &rpm, &vertical(0.1, 0.0, 100.0), 0.0, 2, Some(2));
        assert_eq!(two.points.len(), 2);
        assert_eq!(two.points[0].freq_hz, 0.0);
        assert_eq!(two.points[1].freq_hz, 100.0);

        let v = line_cut_with_samples(&s, &rpm, &vertical(0.1, 0.0, 100.0), 0.0, 3, Some(256));
        assert_eq!(v.points.len(), 256);
        assert!(v.points.iter().all(|p| (p.time_s - 0.1).abs() < 1e-12));
        assert!((v.points[255].freq_hz - 100.0).abs() < 1e-9);

        let horizontal = LineSpec {
            t0: 0.0,
            f0: 50.0,
            t1: s.end_s(),
            f1: 50.0,
            log_f: false,
        };
        let h = line_cut_with_samples(&s, &rpm, &horizontal, 0.0, 4, Some(256));
        assert_eq!(h.points.len(), 256);
        assert!(h.points.iter().all(|p| p.freq_hz == 50.0));
        assert!(h.points.windows(2).all(|w| w[1].time_s > w[0].time_s));

        let log_line = LineSpec {
            t0: 0.0,
            f0: 10.0,
            t1: s.end_s(),
            f1: 100.0,
            log_f: true,
        };
        let g = line_cut_with_samples(&s, &rpm, &log_line, 0.0, 5, Some(256));
        assert_eq!(g.points.len(), 256);
        assert!((g.points[0].freq_hz - 10.0).abs() < 1e-9);
        assert!((g.points[255].freq_hz - 100.0).abs() < 1e-9);
        let geometric_mid = (10.0f64 * 100.0).sqrt();
        assert!((g.points[128].freq_hz - geometric_mid).abs() < 0.2);
    }

    #[test]
    fn exact_line_samples_skip_points_outside_the_data() {
        let s = spec_with_peak();
        let rpm = RpmProfile::constant(0.0);
        // f = 50 + 150*k/9: four points (50, 66.7, 83.3, 100) are in range.
        let cut = line_cut_with_samples(&s, &rpm, &vertical(0.1, 50.0, 200.0), 0.0, 1, Some(10));
        assert_eq!(cut.points.len(), 4);
        assert!(cut.points.iter().all(|p| p.freq_hz <= 100.0));
        let auto = line_cut(&s, &rpm, &vertical(0.1, 50.0, 200.0), 0.0, 1);
        assert!(auto.points.iter().all(|p| p.freq_hz <= 100.0));
        assert!(auto.points.len() >= cut.points.len());
    }

    #[test]
    fn rpm_profile_from_points_sorts_validates_and_rejects_duplicates() {
        let p = RpmProfile::from_points(vec![(2.0, 2000.0), (0.0, 1000.0), (1.0, 1500.0)])
            .expect("valid profile");
        assert_eq!(p.len(), 3);
        assert!((p.rpm_at(0.5) - 1250.0).abs() < 1e-9);
        assert_eq!(p.rpm_at(-1.0), 1000.0);
        assert_eq!(p.rpm_at(9.0), 2000.0);

        let single = RpmProfile::from_points(vec![(3.0, 900.0)]).expect("single point");
        assert_eq!(single.rpm_at(0.0), 900.0);

        assert!(RpmProfile::from_points(Vec::new()).is_err());
        assert!(RpmProfile::from_points(vec![(f64::NAN, 1000.0)]).is_err());
        assert!(RpmProfile::from_points(vec![(0.0, f64::INFINITY)]).is_err());
        assert!(RpmProfile::from_points(vec![(1.0, 1000.0), (1.0, 1200.0)]).is_err());
    }

    #[test]
    fn order_matrix_is_frame_major_with_nan_outside_nyquist() {
        let s = spec_with_peak();
        let rpm = RpmProfile::constant(3000.0); // order 1 = 50, 2 = 100, 3 = 150 Hz
        let m = order_matrix(&s, &rpm, &[1.0, 2.0, 3.0], CutWidth::Orders(0.0));
        assert_eq!(m.frames, s.len());
        assert_eq!(m.order_count, 3);
        assert_eq!(m.orders, vec![1.0, 2.0, 3.0]);
        assert_eq!(m.times.len(), m.frames);
        assert!(m.is_consistent());
        assert_eq!(m.values.len(), m.frames * m.order_count);
        assert!((m.value(0, 0).unwrap() - 40.0).abs() < 1e-3);
        assert!(m.value(0, 1).unwrap().abs() < 1e-3);
        assert!(m.value(0, 2).unwrap().is_nan());
        assert!((m.frequencies[0] - 50.0).abs() < 1e-3);
        assert!(m.frequencies[2].is_nan());
        // Frame-major: frame 1 starts after all three orders of frame 0.
        assert!((m.values[1 * m.order_count] - 40.0).abs() < 1e-3);
        assert_eq!(m.value(m.frames, 0), None);
    }
}
