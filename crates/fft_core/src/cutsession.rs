//! UI-free cut sessions.
//!
//! A display owns the settings and the resulting cuts; both Order Cuts and
//! Line Cuts compute exactly these values. Keeping the session here means the
//! Tauri commands and the headless HTTP handlers share the same maths and CSV
//! output.

use std::path::Path;

use crate::orders::{self, Cut, LineSpec, RpmProfile};
use crate::settings::{LineCutSettings, OrderCutSettings, RpmSource};
use crate::spectrogram::Spectrogram;
use crate::traits::Result;

/// Order/frequency cuts derived from an order-cut settings block.
#[derive(Clone, Debug)]
pub struct OrderCutSession {
    settings: OrderCutSettings,
    cuts: Vec<Cut>,
}

impl OrderCutSession {
    pub fn new(settings: OrderCutSettings) -> Self {
        Self {
            settings,
            cuts: Vec::new(),
        }
    }

    pub fn settings(&self) -> &OrderCutSettings {
        &self.settings
    }

    pub fn settings_mut(&mut self) -> &mut OrderCutSettings {
        &mut self.settings
    }

    pub fn cuts(&self) -> &[Cut] {
        &self.cuts
    }

    /// Resolve the RPM profile for `spec` (constant, ramp over its duration,
    /// or a `time_s,rpm` CSV file).
    pub fn rpm_profile_for(&self, spec: &Spectrogram) -> Result<RpmProfile> {
        let r = &self.settings.rpm;
        match r.source {
            RpmSource::Constant => Ok(RpmProfile::constant(r.constant)),
            RpmSource::Ramp => Ok(RpmProfile::ramp(0.0, spec.end_s(), r.start, r.end)),
            RpmSource::Csv => {
                let path = r
                    .csv
                    .as_ref()
                    .ok_or("RPM source is CSV but no file is configured")?;
                RpmProfile::from_csv(path)
            }
        }
    }

    /// Compute all order and fixed-frequency cuts.
    pub fn compute(&mut self, spec: &Spectrogram) -> Result<()> {
        let rpm = self.rpm_profile_for(spec)?;
        self.compute_with_profile(spec, &rpm);
        Ok(())
    }

    /// Compute with an explicitly supplied profile (the UI may cache one).
    pub fn compute_with_profile(&mut self, spec: &Spectrogram, rpm: &RpmProfile) {
        let mut cuts = Vec::new();
        for order in orders::parse_orders(&self.settings.orders) {
            let width = match self.settings.order_width_hz {
                Some(hz) => orders::CutWidth::Hz(hz),
                None => orders::CutWidth::Orders(self.settings.order_width),
            };
            cuts.push(orders::order_cut_with_width(spec, rpm, order, width));
        }
        for frequency in orders::parse_freqs(&self.settings.freqs) {
            cuts.push(orders::freq_cut(
                spec,
                rpm,
                frequency,
                self.settings.freq_width_hz,
            ));
        }
        for cut in &mut cuts {
            orders::smooth_cut(cut, self.settings.smooth_s);
        }
        self.cuts = cuts;
    }

    pub fn export_csv(&self, path: &Path) -> Result<()> {
        orders::write_csv(path, &self.cuts)
    }
}

/// Line cuts drawn on a spectrogram.
#[derive(Clone, Debug)]
pub struct LineCutSession {
    settings: LineCutSettings,
    lines: Vec<LineSpec>,
    cuts: Vec<Cut>,
}

impl LineCutSession {
    pub fn new(settings: LineCutSettings) -> Self {
        Self {
            settings,
            lines: Vec::new(),
            cuts: Vec::new(),
        }
    }

    pub fn settings(&self) -> &LineCutSettings {
        &self.settings
    }

    pub fn settings_mut(&mut self) -> &mut LineCutSettings {
        &mut self.settings
    }

    pub fn lines(&self) -> &[LineSpec] {
        &self.lines
    }

    pub fn lines_mut(&mut self) -> &mut Vec<LineSpec> {
        &mut self.lines
    }

    pub fn set_lines(&mut self, lines: Vec<LineSpec>) {
        self.lines = lines;
    }

    pub fn add_line(&mut self, line: LineSpec) {
        self.lines.push(line);
    }

    pub fn clear_lines(&mut self) {
        self.lines.clear();
        self.cuts.clear();
    }

    pub fn cuts(&self) -> &[Cut] {
        &self.cuts
    }

    /// Line cuts are never smoothed along time: a vertical line has no time
    /// extent to smooth. They show the parent's smoothing as-is.
    pub fn compute(&mut self, spec: &Spectrogram) {
        let rpm = RpmProfile::constant(0.0);
        self.cuts = self
            .lines
            .iter()
            .enumerate()
            .map(|(k, line)| {
                orders::line_cut_with_samples(
                    spec,
                    &rpm,
                    line,
                    self.settings.line_width_hz,
                    k + 1,
                    self.settings.samples,
                )
            })
            .collect();
    }

    pub fn export_csv(&self, path: &Path) -> Result<()> {
        orders::write_csv(path, &self.cuts)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use realfft::num_complex::Complex;

    use super::*;
    use crate::settings::{LineAxis, RpmSettings};
    use crate::traits::Spectrum;

    /// A one-bin tone: bin 100 of a 1000 Hz / 1 Hz-per-bin analysis.
    fn tone_spec(seconds: f64, hz: f32) -> Spectrogram {
        let bins = 501;
        let bin_hz = 1.0;
        let frames = (seconds * 10.0).round() as usize;
        let mut spec = Spectrogram::new();
        for i in 0..frames {
            let mut bins_vec = vec![Complex::new(1e-6, 0.0); bins];
            bins_vec[hz as usize] = Complex::new(1.0, 0.0);
            spec.push(&Spectrum {
                bins: bins_vec,
                bin_hz,
                frame_index: i,
                // 10 frames per second: one frame every 0.1 s.
                time_s: i as f64 * 0.1,
            });
        }
        spec
    }

    fn order_settings(rpm: f64) -> OrderCutSettings {
        OrderCutSettings {
            orders: "1".into(),
            order_width: 0.2,
            order_width_hz: None,
            freqs: String::new(),
            freq_width_hz: 2.0,
            axis: crate::settings::CutAxis::Rpm,
            smooth_s: 0.0,
            peak_snap: false,
            peak_min_db: 0.0,
            rpm: RpmSettings {
                source: RpmSource::Constant,
                constant: rpm,
                ..Default::default()
            },
        }
    }

    #[test]
    fn order_cut_session_follows_the_order() {
        let spec = tone_spec(1.0, 100.0);
        let mut session = OrderCutSession::new(order_settings(6000.0)); // order 1 = 100 Hz
        session.compute(&spec).unwrap();
        assert_eq!(session.cuts().len(), 1);
        let cut = &session.cuts()[0];
        assert!(!cut.points.is_empty());
        let peak = cut
            .points
            .iter()
            .max_by(|a, b| a.db.total_cmp(&b.db))
            .unwrap();
        assert!(
            (peak.freq_hz - 100.0).abs() < 5.0,
            "expected the 100 Hz tone, got {} Hz",
            peak.freq_hz
        );
    }

    #[test]
    fn ramp_profile_spans_the_spectrogram_duration() {
        let spec = tone_spec(2.0, 100.0);
        let mut settings = order_settings(0.0);
        settings.rpm = RpmSettings {
            source: RpmSource::Ramp,
            start: 600.0,
            end: 6000.0,
            ..Default::default()
        };
        let mut session = OrderCutSession::new(settings);
        let profile = session.rpm_profile_for(&spec).unwrap();
        assert!((profile.rpm_at(0.0) - 600.0).abs() < 1e-6);
        assert!((profile.rpm_at(spec.end_s()) - 6000.0).abs() < 1e-6);
        session.compute(&spec).unwrap();
        let cut = &session.cuts()[0];
        assert!((cut.points.first().unwrap().rpm - 600.0).abs() < 1.0);
        assert!((cut.points.last().unwrap().rpm - 6000.0).abs() < 1.0);
    }

    #[test]
    fn missing_rpm_csv_reports_an_error() {
        let spec = tone_spec(1.0, 100.0);
        let mut settings = order_settings(0.0);
        settings.rpm.source = RpmSource::Csv;
        settings.rpm.csv = None;
        let mut session = OrderCutSession::new(settings);
        assert!(session.compute(&spec).is_err());
    }

    #[test]
    fn line_cut_session_exports_the_shared_csv_format() {
        let spec = tone_spec(1.0, 100.0);
        let mut session = LineCutSession::new(LineCutSettings {
            line_axis: LineAxis::Time,
            ..Default::default()
        });
        session.add_line(LineSpec {
            t0: 0.2,
            f0: 50.0,
            t1: 0.2,
            f1: 150.0,
            log_f: false,
        });
        session.compute(&spec);
        assert_eq!(session.cuts().len(), 1);
        assert!(!session.cuts()[0].points.is_empty());

        let path =
            std::env::temp_dir().join(format!("fft_core_linecuts_{}.csv", std::process::id()));
        session.export_csv(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("kind,value,time_s,rpm,freq_hz,db"));
        let _ = std::fs::remove_file(PathBuf::from(path));
    }

    #[test]
    fn clearing_lines_clears_cuts_too() {
        let spec = tone_spec(1.0, 100.0);
        let mut session = LineCutSession::new(LineCutSettings::default());
        session.add_line(LineSpec {
            t0: 0.1,
            f0: 100.0,
            t1: 0.9,
            f1: 100.0,
            log_f: false,
        });
        session.compute(&spec);
        assert_eq!(session.cuts().len(), 1);
        session.clear_lines();
        assert!(session.lines().is_empty());
        assert!(session.cuts().is_empty());
    }

    #[test]
    fn order_cut_session_uses_hz_width_when_configured() {
        let spec = tone_spec(1.0, 100.0);
        let mut settings = order_settings(6000.0); // order 1 = 100 Hz
        settings.order_width_hz = Some(3.0);
        let mut session = OrderCutSession::new(settings);
        session.compute(&spec).unwrap();
        let rpm = session.rpm_profile_for(&spec).unwrap();
        let expected = orders::order_cut_with_width(&spec, &rpm, 1.0, orders::CutWidth::Hz(3.0));
        assert_eq!(session.cuts(), &[expected]);
    }

    #[test]
    fn line_cut_session_honours_exact_sample_count() {
        let spec = tone_spec(1.0, 100.0);
        let mut session = LineCutSession::new(LineCutSettings {
            samples: Some(2),
            ..Default::default()
        });
        session.add_line(LineSpec {
            t0: 0.5,
            f0: 50.0,
            t1: 0.5,
            f1: 150.0,
            log_f: false,
        });
        session.compute(&spec);
        assert_eq!(session.cuts()[0].points.len(), 2);
        assert_eq!(session.cuts()[0].points[0].freq_hz, 50.0);
        assert_eq!(session.cuts()[0].points[1].freq_hz, 150.0);
    }
}
