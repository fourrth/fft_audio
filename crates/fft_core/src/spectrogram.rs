//! Contiguous frame-major dB spectrogram storage plus software rendering of a
//! frequency/time view into an RGB framebuffer.

use crate::traits::Spectrum;

/// A plain RGB framebuffer, one `0x00RRGGBB` pixel per entry, row-major.
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u32>,
}

#[derive(Clone, Debug)]
pub struct RenderOptions {
    pub width: usize,
    pub height: usize,
    /// Logarithmic frequency axis (better for audio) instead of linear.
    pub log_freq: bool,
    /// Lowest frequency shown on a log axis.
    pub min_hz: f32,
    /// Highest frequency shown (default: Nyquist).
    pub max_hz: Option<f32>,
    /// Everything this many dB below the loudest bin is drawn black.
    pub dynamic_range_db: f32,
    /// Absolute bottom of the color scale. `None` keeps the top-relative
    /// `dynamic_range_db` behaviour. When both bounds are set, `db_max` is the
    /// top and `db_max - db_min` is the span.
    pub db_min: Option<f32>,
    /// Absolute top of the color scale (default: the loudest bin).
    pub db_max: Option<f32>,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            width: 1200,
            height: 600,
            log_freq: true,
            min_hz: 20.0,
            max_hz: None,
            dynamic_range_db: 80.0,
            db_min: None,
            db_max: None,
        }
    }
}

/// A rectangle of the time/frequency plane, used by the interactive viewer
/// to render just the part that is currently on screen.
#[derive(Clone, Copy, Debug)]
pub struct View {
    pub t0: f64,
    pub t1: f64,
    pub f0: f32,
    pub f1: f32,
    pub log_freq: bool,
}

/// Collects spectra over time (as dB) and renders them as an image.
///
/// Values are stored contiguously, frame-major: frame `i` occupies
/// `data[i * bins .. (i + 1) * bins]`. This is the canonical layout for
/// zero-copy array views; [`Spectrogram::column`] still returns one frame.
/// Memory use is `frames * bins * 4` bytes; use a larger hop for long files.
pub struct Spectrogram {
    /// Flat frame-major dB values, `frames * bins`.
    data: Vec<f32>,
    /// Number of frequency bins per frame (0 while empty).
    bins: usize,
    bin_hz: f32,
    /// Start time of the last frame, seconds.
    duration_s: f64,
    /// Length of one analysis frame in seconds (see `set_frame_len_s`).
    frame_s: f64,
    max_db: f32,
}

impl Default for Spectrogram {
    fn default() -> Self {
        Self {
            data: Vec::new(),
            bins: 0,
            bin_hz: 0.0,
            duration_s: 0.0,
            frame_s: 0.0,
            max_db: f32::NEG_INFINITY,
        }
    }
}

impl Spectrogram {
    pub fn new() -> Self {
        Self::default()
    }

    /// Tell the spectrogram how long one frame is (`frame_len / sample_rate`)
    /// so that frame *centre* times are right. Call before `push`.
    pub fn set_frame_len_s(&mut self, s: f64) {
        self.frame_s = s;
    }

    pub fn push(&mut self, s: &Spectrum) {
        self.bin_hz = s.bin_hz;
        self.duration_s = s.time_s;
        let col: Vec<f32> = s
            .magnitudes()
            .map(|m| 20.0 * m.max(1e-10).log10())
            .collect();
        if self.bins == 0 {
            self.bins = col.len();
        }
        debug_assert_eq!(
            col.len(),
            self.bins,
            "every spectrum must have the same number of bins"
        );
        for &d in &col {
            if d > self.max_db {
                self.max_db = d;
            }
        }
        self.data.extend_from_slice(&col);
    }

    pub fn len(&self) -> usize {
        self.frames()
    }

    /// Number of frames stored (frame-major rows of `bins` values).
    pub fn frames(&self) -> usize {
        if self.bins == 0 {
            0
        } else {
            self.data.len() / self.bins
        }
    }

    pub fn is_empty(&self) -> bool {
        self.frames() == 0
    }

    /// All dB values, contiguous and frame-major: `frames * bins` entries.
    pub fn data(&self) -> &[f32] {
        &self.data
    }

    /// Number of frequency bins per frame.
    pub fn bins(&self) -> usize {
        self.bins
    }

    /// Start time of the last frame, in seconds.
    pub fn duration_s(&self) -> f64 {
        self.duration_s
    }

    pub fn bin_hz(&self) -> f32 {
        self.bin_hz
    }

    pub fn nbins(&self) -> usize {
        self.bins
    }

    pub fn nyquist(&self) -> f32 {
        self.nbins().saturating_sub(1) as f32 * self.bin_hz
    }

    /// Loudest bin in the whole spectrogram (dB).
    pub fn max_db(&self) -> f32 {
        self.max_db
    }

    /// Seconds between consecutive frames.
    pub fn hop_s(&self) -> f64 {
        let n = self.len();
        let hop = if n > 1 {
            self.duration_s / (n - 1) as f64
        } else {
            self.frame_s
        };
        hop.max(1e-9)
    }

    /// End of the analysed audio, seconds.
    pub fn end_s(&self) -> f64 {
        self.len().saturating_sub(1) as f64 * self.hop_s() + self.frame_s
    }

    /// Time at the middle of frame `i`.
    pub fn frame_center(&self, i: usize) -> f64 {
        i as f64 * self.hop_s() + self.frame_s * 0.5
    }

    pub fn nearest_frame(&self, t: f64) -> usize {
        let i = ((t - self.frame_s * 0.5) / self.hop_s()).round();
        i.clamp(0.0, self.len().saturating_sub(1) as f64) as usize
    }

    /// Causal exponential averaging of every bin over time (in the power
    /// domain), time constant `tau_s` seconds: `y[n] = y[n-1] + a (x[n] - y[n-1])`
    /// with `a = 1 - exp(-hop / tau)`. `tau_s <= 0` leaves the data untouched.
    pub fn smooth_time(&mut self, tau_s: f64) {
        if tau_s <= 0.0 || self.len() < 2 || self.bins == 0 {
            return;
        }
        let a = (1.0 - (-self.hop_s() / tau_s).exp()) as f32;
        let pow = |d: f32| 10f32.powf(d / 10.0);
        let bins = self.bins;
        let mut state: Vec<f32> = self.data[..bins].iter().map(|&d| pow(d)).collect();
        let mut max_db = f32::NEG_INFINITY;
        for frame in 0..self.len() {
            let col = &mut self.data[frame * bins..(frame + 1) * bins];
            for (d, s) in col.iter_mut().zip(state.iter_mut()) {
                *s += a * (pow(*d) - *s);
                *d = 10.0 * s.max(1e-20).log10();
                if *d > max_db {
                    max_db = *d;
                }
            }
        }
        self.max_db = max_db;
    }

    /// Spectrogram level at an arbitrary point of the time/frequency plane,
    /// bilinearly interpolated between frames and bins. `None` outside the data.
    pub fn db_at(&self, t: f64, f: f64) -> Option<f32> {
        let n = self.len();
        let nb = self.nbins();
        if n == 0 || nb == 0 || !t.is_finite() || !f.is_finite() {
            return None;
        }
        if t < 0.0 || t > self.end_s() || f < 0.0 || f > self.nyquist() as f64 {
            return None;
        }
        let fi = ((t - self.frame_s * 0.5) / self.hop_s()).clamp(0.0, (n - 1) as f64);
        let i0 = fi.floor() as usize;
        let i1 = (i0 + 1).min(n - 1);
        let a = (fi - i0 as f64) as f32;
        let fb = (f / self.bin_hz as f64).clamp(0.0, (nb - 1) as f64);
        let j0 = fb.floor() as usize;
        let j1 = (j0 + 1).min(nb - 1);
        let b = (fb - j0 as f64) as f32;
        let lerp_bins = |c: &[f32]| c[j0] * (1.0 - b) + c[j1] * b;
        let v0 = lerp_bins(self.column(i0));
        let v1 = lerp_bins(self.column(i1));
        Some(v0 * (1.0 - a) + v1 * a)
    }

    /// dB value of every bin of frame `i`.
    pub fn column(&self, i: usize) -> &[f32] {
        let start = i * self.bins;
        &self.data[start..start + self.bins]
    }

    /// Render an arbitrary rectangle of the time/frequency plane at `w` x `h`
    /// pixels. Zoomed out, several frames/bins per pixel are max-pooled so
    /// narrow peaks stay visible; zoomed in, bins are interpolated.
    pub fn render_view(
        &self,
        view: &View,
        w: usize,
        h: usize,
        top_db: f32,
        range_db: f32,
    ) -> Image {
        let mut pixels = vec![0u32; w * h];
        let n = self.len();
        let bad = n == 0
            || self.bins == 0
            || w == 0
            || h == 0
            || view.t1 <= view.t0
            || view.f1 <= view.f0
            || (view.log_freq && view.f0 <= 0.0)
            || range_db <= 0.0;
        if bad {
            return Image {
                width: w,
                height: h,
                pixels,
            };
        }

        let nbins = self.bins;
        let last = nbins - 1;
        let lut = inferno_lut();
        let bottom_db = top_db - range_db;
        let bin_hz = self.bin_hz as f64;
        let (f0, f1) = (view.f0 as f64, view.f1 as f64);
        let freq_at = |u: f64| -> f64 {
            if view.log_freq {
                f0 * (f1 / f0).powf(u)
            } else {
                f0 + (f1 - f0) * u
            }
        };

        #[derive(Clone, Copy)]
        enum Row {
            Interp(usize, usize, f32),
            Max(usize, usize),
        }
        let rows: Vec<Row> = (0..h)
            .map(|y| {
                let fb = freq_at(1.0 - (y + 1) as f64 / h as f64) / bin_hz;
                let ft = freq_at(1.0 - y as f64 / h as f64) / bin_hz;
                if ft - fb <= 1.0 {
                    let c = ((fb + ft) / 2.0).clamp(0.0, last as f64);
                    let lo = c.floor() as usize;
                    Row::Interp(lo, (lo + 1).min(last), (c - lo as f64) as f32)
                } else {
                    Row::Max(
                        (fb.floor().max(0.0) as usize).min(last),
                        (ft.ceil().max(0.0) as usize).min(last),
                    )
                }
            })
            .collect();

        let hop = self.hop_s();
        let off = self.frame_s * 0.5;
        let span = view.t1 - view.t0;
        for x in 0..w {
            let ta = view.t0 + span * x as f64 / w as f64;
            let tb = view.t0 + span * (x + 1) as f64 / w as f64;
            let c0 = (((ta - off) / hop + 0.5).floor().max(0.0) as usize).min(n - 1);
            let c1 = ((((tb - off) / hop + 0.5).floor().max(0.0) as usize).max(c0 + 1)).min(n);
            for (y, row) in rows.iter().enumerate() {
                let db = match *row {
                    Row::Interp(lo, hi, fr) => (c0..c1)
                        .map(|frame| {
                            let base = frame * nbins;
                            self.data[base + lo] * (1.0 - fr) + self.data[base + hi] * fr
                        })
                        .fold(f32::NEG_INFINITY, f32::max),
                    Row::Max(lo, hi) => (c0..c1)
                        .map(|frame| {
                            let base = frame * nbins;
                            self.data[base + lo..=base + hi]
                                .iter()
                                .cloned()
                                .fold(f32::NEG_INFINITY, f32::max)
                        })
                        .fold(f32::NEG_INFINITY, f32::max),
                };
                let t = ((db - bottom_db) / range_db).clamp(0.0, 1.0);
                pixels[y * w + x] = lut[(t * 255.0) as usize];
            }
        }
        Image {
            width: w,
            height: h,
            pixels,
        }
    }

    /// [`render_view`](Self::render_view) with whole-display [`RenderOptions`].
    ///
    /// The view still selects the time/frequency rectangle, but `min_hz` acts
    /// as a lower bound (and log-axis floor) and `db_min`/`db_max` select the
    /// absolute color-scale bounds. With no absolute bounds, the color scale
    /// is top-relative exactly like [`render_view`](Self::render_view).
    pub fn render_view_with_options(
        &self,
        view: &View,
        w: usize,
        h: usize,
        opt: &RenderOptions,
    ) -> Image {
        let mut view = *view;
        let floor = if view.log_freq {
            opt.min_hz.max(1.0)
        } else {
            opt.min_hz.max(0.0)
        };
        view.f0 = view.f0.max(floor);
        let (top, range) = self.render_db_bounds(opt);
        self.render_view(&view, w, h, top, range)
    }

    /// Top and span of the color scale for `opt`, preserving top-relative
    /// behaviour when no absolute bounds are requested.
    fn render_db_bounds(&self, opt: &RenderOptions) -> (f32, f32) {
        match (opt.db_max, opt.db_min) {
            (None, None) => (self.max_db, opt.dynamic_range_db),
            (Some(top), None) => (top, opt.dynamic_range_db),
            (None, Some(bottom)) => (self.max_db, self.max_db - bottom),
            (Some(top), Some(bottom)) => (top, top - bottom),
        }
    }

    /// Whole-file static render (used for `--out file.png`).
    /// Time on the x axis, frequency on the y axis (high at the top).
    pub fn render(&self, opt: &RenderOptions) -> Image {
        let (w, h) = (opt.width, opt.height);
        let mut pixels = vec![0u32; w * h];
        if self.is_empty() || w == 0 || h == 0 {
            return Image {
                width: w,
                height: h,
                pixels,
            };
        }

        let n = self.len();
        let nbins = self.bins;
        let nyquist = (nbins - 1) as f32 * self.bin_hz;
        let f_hi = opt.max_hz.unwrap_or(nyquist).min(nyquist).max(1.0);
        let f_lo = if opt.log_freq {
            opt.min_hz.max(1.0).min(f_hi * 0.5)
        } else {
            0.0
        };

        let (db_max, range_db) = self.render_db_bounds(opt);
        let db_min = db_max - range_db;
        let lut = inferno_lut();

        // Frequency bin shown by each pixel row.
        let rows: Vec<usize> = (0..h)
            .map(|y| {
                let t = 1.0 - (y as f32 + 0.5) / h as f32;
                let f = if opt.log_freq {
                    f_lo * (f_hi / f_lo).powf(t)
                } else {
                    f_lo + (f_hi - f_lo) * t
                };
                ((f / self.bin_hz).round() as usize).min(nbins - 1)
            })
            .collect();

        for x in 0..w {
            // Several frames can map onto one pixel column: keep the loudest.
            let c0 = x * n / w;
            let c1 = ((x + 1) * n / w).max(c0 + 1).min(n);
            for (y, &bin) in rows.iter().enumerate() {
                let db = (c0..c1)
                    .map(|frame| self.data[frame * nbins + bin])
                    .fold(f32::NEG_INFINITY, f32::max);
                let t = ((db - db_min) / range_db).clamp(0.0, 1.0);
                pixels[y * w + x] = lut[(t * 255.0) as usize];
            }
        }
        Image {
            width: w,
            height: h,
            pixels,
        }
    }
}

/// A compact, transport-friendly spectrogram: dB values scaled by 100 and
/// max-pooled down to at most `frames × bins` cells (frame-major).
#[derive(Clone, Debug, PartialEq)]
pub struct SpectrogramGrid {
    pub frames: usize,
    pub bins: usize,
    pub bin_hz: f32,
    pub frame_s: f64,
    pub duration_s: f64,
    pub nyquist: f32,
    pub max_db: f32,
    pub min_db: f32,
    /// `frames * bins` values, frame-major, `dB * 100` as little-endian i16.
    pub data: Vec<i16>,
}

impl Spectrogram {
    /// Downsample for the UI transport. Max-pooling preserves peaks; the grid
    /// is never larger than `max_frames × max_bins`.
    pub fn to_grid(&self, max_frames: usize, max_bins: usize) -> SpectrogramGrid {
        let frames = self.len().min(max_frames.max(1));
        let bins = self.nbins().min(max_bins.max(1));
        if frames == 0 || bins == 0 {
            return SpectrogramGrid {
                frames: 0,
                bins: 0,
                bin_hz: 0.0,
                frame_s: 0.0,
                duration_s: 0.0,
                nyquist: 0.0,
                max_db: 0.0,
                min_db: 0.0,
                data: Vec::new(),
            };
        }

        let mut pooled = vec![i16::MIN; frames * bins];
        let total_frames = self.len();
        for frame in 0..total_frames {
            let column = self.column(frame);
            let out_frame = frame * frames / total_frames;
            for (bin, &db) in column.iter().enumerate() {
                if !db.is_finite() {
                    continue;
                }
                let out_bin = bin * bins / column.len();
                let value = (db * 100.0).clamp(i16::MIN as f32, i16::MAX as f32) as i16;
                let slot = &mut pooled[out_frame * bins + out_bin];
                if value > *slot {
                    *slot = value;
                }
            }
        }
        for value in &mut pooled {
            if *value == i16::MIN {
                *value = 0;
            }
        }
        let min_db = pooled
            .iter()
            .map(|&v| v as f32 / 100.0)
            .fold(f32::INFINITY, f32::min);
        let max_db = pooled
            .iter()
            .map(|&v| v as f32 / 100.0)
            .fold(f32::NEG_INFINITY, f32::max);

        SpectrogramGrid {
            frames,
            bins,
            bin_hz: self.bin_hz * self.nbins() as f32 / bins as f32,
            frame_s: self.hop_s(),
            duration_s: self.end_s(),
            nyquist: self.nyquist(),
            max_db,
            min_db,
            data: pooled,
        }
    }
}

/// 256 entry lookup table from the `colorgrad` inferno gradient.
pub fn inferno_lut() -> Vec<u32> {
    let grad = colorgrad::inferno();
    (0..256)
        .map(|i| {
            let [r, g, b, _] = grad.at(i as f64 / 255.0).to_rgba8();
            u32::from_be_bytes([0, r, g, b])
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use realfft::num_complex::Complex;

    use super::*;

    fn tone_spec() -> Spectrogram {
        let mut spec = Spectrogram::new();
        for frame in 0..4 {
            let mut bins = vec![Complex::new(0.01, 0.0); 5];
            bins[0] = Complex::new(1.0, 0.0); // 0 dB at bin 0
            if frame == 3 {
                bins[4] = Complex::new(1.0, 0.0); // 0 dB at bin 4
            }
            spec.push(&Spectrum {
                bins,
                bin_hz: 10.0,
                frame_index: frame,
                time_s: frame as f64 * 0.1,
            });
        }
        spec
    }

    #[test]
    fn grid_downsamples_and_preserves_peaks() {
        let grid = tone_spec().to_grid(2, 2);
        assert_eq!(grid.frames, 2);
        assert_eq!(grid.bins, 2);
        assert_eq!(grid.data.len(), 4);
        assert_eq!(grid.bin_hz, 25.0);
        assert!((grid.max_db - 0.0).abs() < 0.01);
        assert!((grid.min_db + 40.0).abs() < 0.01);
        // frame 0 -> output 0, bin 0 -> output 0
        assert_eq!(grid.data[0], 0);
        // frame 3 -> output 1, bin 4 -> output 1
        assert_eq!(grid.data[3], 0);
    }

    #[test]
    fn empty_spectrogram_makes_an_empty_grid() {
        let grid = Spectrogram::new().to_grid(100, 100);
        assert_eq!(grid.frames, 0);
        assert_eq!(grid.bins, 0);
        assert!(grid.data.is_empty());
        assert_eq!(grid.duration_s, 0.0);
    }

    #[test]
    fn flat_storage_is_frame_major_and_columns_are_slices() {
        let spec = tone_spec();
        assert_eq!(spec.frames(), 4);
        assert_eq!(spec.bins(), 5);
        assert_eq!(spec.nbins(), 5);
        assert_eq!(spec.data().len(), 4 * 5);
        for frame in 0..spec.frames() {
            assert_eq!(
                spec.column(frame),
                &spec.data()[frame * spec.bins()..(frame + 1) * spec.bins()]
            );
        }
        // bin 0 is 0 dB; the rest are -40 dB; frame 3 has a peak in bin 4.
        assert!(spec.data()[0].abs() < 1e-5);
        assert!((spec.data()[1] + 40.0).abs() < 1e-4);
        assert!(spec.data()[3 * 5 + 4].abs() < 1e-5);
        assert!((spec.column(3)[4]).abs() < 1e-5);
    }

    #[test]
    fn empty_spectrogram_has_no_bins_or_data() {
        let spec = Spectrogram::new();
        assert_eq!(spec.frames(), 0);
        assert_eq!(spec.bins(), 0);
        assert_eq!(spec.nbins(), 0);
        assert!(spec.data().is_empty());
        assert!(spec.is_empty());
    }

    #[test]
    fn smooth_time_matches_power_domain_averaging_in_flat_storage() {
        let mut spec = Spectrogram::new();
        spec.set_frame_len_s(0.1);
        for (frame, amplitude) in [(0, 1.0f32), (1, 0.01), (2, 0.01)] {
            spec.push(&Spectrum {
                bins: vec![Complex::new(amplitude, 0.0), Complex::new(amplitude, 0.0)],
                bin_hz: 10.0,
                frame_index: frame,
                time_s: frame as f64 * 0.1,
            });
        }
        assert!((spec.data()[3] + 40.0).abs() < 1e-4);
        spec.smooth_time(1.0);
        // a = 1 - exp(-hop/tau), hop = 0.1 s, tau = 1 s.
        let a = 1.0 - (-0.1f64).exp();
        let mut state = 1.0f64;
        let expected: Vec<f32> = (0..3)
            .map(|frame| {
                let x = if frame == 0 { 1.0 } else { 1e-4 };
                state += a * (x - state);
                (10.0 * state.log10()) as f32
            })
            .collect();
        for (frame, &want) in expected.iter().enumerate() {
            assert!(
                (spec.data()[frame * 2 + 1] - want).abs() < 1e-3,
                "frame {frame}: {} vs {want}",
                spec.data()[frame * 2 + 1]
            );
        }
        // Frame 0 is unchanged; later frames have not jumped to the new level yet.
        assert!(spec.data()[1].abs() < 1e-4);
        assert!(spec.data()[3] > -1.0 && spec.data()[3] < -0.1);
    }

    fn two_level_spec() -> Spectrogram {
        let mut spec = Spectrogram::new();
        spec.set_frame_len_s(0.1);
        for frame in 0..2 {
            let mut bins = vec![Complex::new(0.01, 0.0); 5]; // -40 dB
            bins[0] = Complex::new(1.0, 0.0); // 0 dB
            spec.push(&Spectrum {
                bins,
                bin_hz: 10.0,
                frame_index: frame,
                time_s: frame as f64 * 0.1,
            });
        }
        spec
    }

    fn render_options(log_freq: bool) -> RenderOptions {
        RenderOptions {
            width: 2,
            height: 5,
            log_freq,
            min_hz: 0.0,
            max_hz: None,
            dynamic_range_db: 80.0,
            db_min: None,
            db_max: None,
        }
    }

    #[test]
    fn absolute_db_bounds_override_the_relative_range() {
        let spec = two_level_spec();
        let lut = inferno_lut();
        let base = render_options(false);
        let default = spec.render(&base);
        // Bottom row is bin 0 (0 dB, the loudest → full color).
        assert_eq!(default.pixels[4 * 2], lut[255]);

        let mut shifted = base.clone();
        shifted.db_max = Some(20.0);
        let image = spec.render(&shifted);
        // top 20 dB, 80 dB span → 0 dB maps to 60/80 = 75 %.
        assert_eq!(image.pixels[4 * 2], lut[191]);
        assert_ne!(image.pixels[4 * 2], default.pixels[4 * 2]);

        let mut absolute = base.clone();
        absolute.db_min = Some(-20.0);
        absolute.db_max = Some(0.0);
        let absolute_image = spec.render(&absolute);
        // -40 dB is below the absolute bottom and stays black.
        assert_eq!(absolute_image.pixels[3 * 2], lut[0]);
        assert_eq!(absolute_image.pixels[4 * 2], lut[255]);

        let mut wider = base;
        wider.db_min = Some(-60.0);
        wider.db_max = Some(0.0);
        let image = spec.render(&wider);
        // -40 dB is 20 dB into a 60 dB span.
        assert_eq!(image.pixels[3 * 2], lut[((20.0 / 60.0) * 255.0) as usize]);
        assert_ne!(image.pixels[3 * 2], absolute_image.pixels[3 * 2]);

        let mut bottom_only = render_options(false);
        bottom_only.db_min = Some(-20.0);
        let image = spec.render(&bottom_only);
        // The top stays at the loudest bin; -40 dB is below the absolute bottom.
        assert_eq!(image.pixels[4 * 2], lut[255]);
        assert_eq!(image.pixels[3 * 2], lut[0]);
    }

    fn loud_bin_5_spec() -> Spectrogram {
        let mut spec = Spectrogram::new();
        spec.set_frame_len_s(0.1);
        for frame in 0..2 {
            let mut bins = vec![Complex::new(0.01, 0.0); 11];
            bins[5] = Complex::new(1.0, 0.0);
            spec.push(&Spectrum {
                bins,
                bin_hz: 10.0,
                frame_index: frame,
                time_s: frame as f64 * 0.1,
            });
        }
        spec
    }

    #[test]
    fn min_hz_sets_the_log_floor_for_render_and_render_view() {
        let spec = loud_bin_5_spec();
        let lut = inferno_lut();
        let mut low = render_options(true);
        low.max_hz = Some(100.0);
        low.min_hz = 10.0;
        let mut high = low.clone();
        high.min_hz = 50.0;
        let low_image = spec.render(&low);
        let high_image = spec.render(&high);
        // Bottom rows: ~12.6 Hz (quiet) with a 10 Hz floor, ~53.6 Hz
        // (the loud bin) with a 50 Hz floor.
        assert_eq!(low_image.pixels[4 * 2], lut[127]);
        assert_eq!(high_image.pixels[4 * 2], lut[255]);
        assert_ne!(low_image.pixels[4 * 2], high_image.pixels[4 * 2]);

        let view = View {
            t0: 0.0,
            t1: spec.end_s(),
            f0: 0.0,
            f1: 100.0,
            log_freq: true,
        };
        let via_options = spec.render_view_with_options(&view, 2, 5, &high);
        let via_low = spec.render_view_with_options(&view, 2, 5, &low);
        let mut explicit_view = view;
        explicit_view.f0 = 50.0;
        let explicit = spec.render_view(&explicit_view, 2, 5, spec.max_db(), high.dynamic_range_db);
        assert_eq!(via_options.pixels, explicit.pixels);
        assert_ne!(via_options.pixels[4 * 2], via_low.pixels[4 * 2]);
        // Defaults keep the top-relative color scale.
        let default = RenderOptions::default();
        assert_eq!(default.min_hz, 20.0);
        assert_eq!(default.db_min, None);
        assert_eq!(default.db_max, None);
    }
}
