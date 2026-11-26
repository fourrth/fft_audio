//! File -> spectrogram, driven entirely by `AnalysisSettings`.
//! Used by the runtime when a signal is analysed or re-analysed with new
//! settings.
//!
//! Audio files are decoded with `symphonia`; data tables (csv, parquet, json,
//! ...) are read by the feature-gated [`crate::table`] readers and one column
//! becomes the signal.

use std::path::Path;

use crate::settings::AnalysisSettings;
use crate::sources::{FileSource, VecSource};
use crate::spectrogram::Spectrogram;
use crate::stages::{Filter, BUTTERWORTH_Q};
use crate::table::{self, TableFormat, TableInfo};
use crate::{Analyzer, AnalyzerConfig, Result, SignalSource, Spectrum};

pub struct Analysis {
    pub spectrogram: Spectrogram,
    /// Samples per second of the analysed signal.
    pub sample_rate: f64,
    /// Present when the input was a data table: what was found in it.
    pub table: Option<TableInfo>,
}

/// Open `path` as a signal: a data table when its extension says so, otherwise audio.
fn open_source(
    path: &Path,
    s: &AnalysisSettings,
) -> Result<(Box<dyn SignalSource>, Option<TableInfo>)> {
    match TableFormat::from_path(path) {
        Some(format) => {
            let signal = table::load(path, format, &s.data)?;
            let source = VecSource::new(signal.samples, signal.sample_rate);
            Ok((Box::new(source), Some(signal.info)))
        }
        None => Ok((Box::new(FileSource::open(path)?), None)),
    }
}

pub fn analyze(path: &Path, s: &AnalysisSettings) -> Result<Analysis> {
    analyze_with_progress(path, s, |_, _| true)
}

/// Like [`analyze`], but reports `(frames_done, frames_total)` after every
/// frame. Returning `false` from the callback cancels the analysis and makes
/// this return an error containing `"cancelled"`. `frames_total` is an
/// estimate based on the container's sample count (None when unknown).
pub fn analyze_with_progress(
    path: &Path,
    s: &AnalysisSettings,
    progress: impl FnMut(usize, Option<usize>) -> bool,
) -> Result<Analysis> {
    // Validate before opening the file, but run the shared implementation
    // without validating a second time.
    s.validate()?;
    let (mut source, table) = open_source(path, s)?;
    let mut analysis = run_source(&mut *source, s, progress)?;
    analysis.table = table;
    Ok(analysis)
}

/// Analyse any already-open signal source (audio decoder, in-memory samples,
/// table column, ...). `source` is consumed from its current position.
///
/// The returned [`Analysis`] has no table metadata; use [`analyze`] for path
/// inputs when that metadata matters.
pub fn analyze_source(source: &mut dyn SignalSource, s: &AnalysisSettings) -> Result<Analysis> {
    analyze_source_with_progress(source, s, |_, _| true)
}

/// [`analyze_source`] with the same progress/cancellation contract as
/// [`analyze_with_progress`]: the callback runs after every frame, and
/// returning `false` stops with an error containing `"cancelled"`.
pub fn analyze_source_with_progress(
    source: &mut dyn SignalSource,
    s: &AnalysisSettings,
    progress: impl FnMut(usize, Option<usize>) -> bool,
) -> Result<Analysis> {
    s.validate()?;
    run_source(source, s, progress)
}

/// Analyse in-memory mono samples at `sample_rate` Hz.
pub fn analyze_samples(
    samples: Vec<f32>,
    sample_rate: f64,
    s: &AnalysisSettings,
) -> Result<Analysis> {
    analyze_samples_with_progress(samples, sample_rate, s, |_, _| true)
}

/// [`analyze_samples`] with progress/cancellation support.
pub fn analyze_samples_with_progress(
    samples: Vec<f32>,
    sample_rate: f64,
    s: &AnalysisSettings,
    progress: impl FnMut(usize, Option<usize>) -> bool,
) -> Result<Analysis> {
    if !(sample_rate.is_finite() && sample_rate > 0.0) {
        return Err("sample rate must be a positive number".into());
    }
    s.validate()?;
    let mut source = VecSource::new(samples, sample_rate);
    run_source(&mut source, s, progress)
}

/// Shared analysis body. Callers validate the settings first.
fn run_source(
    source: &mut dyn SignalSource,
    s: &AnalysisSettings,
    mut progress: impl FnMut(usize, Option<usize>) -> bool,
) -> Result<Analysis> {
    let sr = source.sample_rate();
    let frames_total = source
        .expected_samples()
        .map(|n| expected_frames(n, s.frame, s.hop_samples()));

    let cfg = AnalyzerConfig::new(s.frame)
        .fft_size(s.frame * s.pad.max(1))
        .hop(s.hop_samples());
    let mut analyzer = Analyzer::new(cfg);

    if let Some(hz) = s.highpass {
        analyzer = analyzer.stream_stage(Filter::highpass(sr, hz, BUTTERWORTH_Q)?);
    }
    if let Some(hz) = s.lowpass {
        analyzer = analyzer.stream_stage(Filter::lowpass(sr, hz, BUTTERWORTH_Q)?);
    }
    analyzer = analyzer.frame_stage(s.window.stage());

    let mut spectrogram = Spectrogram::new();
    spectrogram.set_frame_len_s(s.frame as f64 / sr);
    let mut frames_done = 0usize;
    analyzer.run(&mut *source, |sp: &Spectrum| -> Result<()> {
        spectrogram.push(sp);
        frames_done += 1;
        if !progress(frames_done, frames_total) {
            return Err("analysis cancelled".into());
        }
        Ok(())
    })?;
    if spectrogram.is_empty() {
        return Err("no samples decoded".into());
    }
    spectrogram.smooth_time(s.smooth_s as f64);
    Ok(Analysis {
        spectrogram,
        sample_rate: sr,
        table: None,
    })
}

/// Approximate number of FFT frames for `samples` input samples.
fn expected_frames(samples: u64, frame: usize, hop: usize) -> usize {
    let frame = frame.max(1) as u64;
    let hop = hop.max(1) as u64;
    if samples == 0 {
        0
    } else if samples <= frame {
        1
    } else {
        // The final frame is end-aligned, so any remainder starts one more
        // frame even when it is shorter than a full hop.
        (1 + (samples - frame).div_ceil(hop)) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{tones, write_wav};

    fn settings() -> AnalysisSettings {
        AnalysisSettings {
            frame: 512,
            hop: Some(128),
            pad: 2,
            ..Default::default()
        }
    }

    #[test]
    fn path_source_and_samples_agree_on_shape() {
        let samples = tones(16_000, 0.5, &[(440.0, 0.5)]);
        let path = write_wav("pipeline_equiv", 16_000, &samples);
        let from_path = analyze(&path, &settings()).expect("path analysis");

        let mut source = VecSource::new(samples.clone(), 16_000.0);
        let from_source = analyze_source(&mut source, &settings()).expect("source analysis");
        let from_samples =
            analyze_samples(samples.clone(), 16_000.0, &settings()).expect("sample analysis");

        for other in [&from_source, &from_samples] {
            assert_eq!(other.sample_rate, from_path.sample_rate);
            assert_eq!(other.spectrogram.frames(), from_path.spectrogram.frames());
            assert_eq!(other.spectrogram.bins(), from_path.spectrogram.bins());
            assert_eq!(other.spectrogram.bin_hz(), from_path.spectrogram.bin_hz());
            assert_eq!(other.spectrogram.hop_s(), from_path.spectrogram.hop_s());
        }
        // The raw sample APIs see byte-identical input.
        assert_eq!(
            from_source.spectrogram.data(),
            from_samples.spectrogram.data()
        );
        // The WAV path is quantised to i16, so levels are close but not identical.
        let path_db = from_path.spectrogram.data();
        let sample_db = from_samples.spectrogram.data();
        for (a, b) in path_db.iter().zip(sample_db) {
            if *b > -30.0 {
                assert!((a - b).abs() < 0.2, "path {a} dB vs samples {b} dB");
            }
        }
        assert!((from_path.spectrogram.max_db() - from_samples.spectrogram.max_db()).abs() < 0.1);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn sample_progress_reports_every_frame_and_cancels() {
        let samples = tones(16_000, 0.4, &[(440.0, 0.5)]);
        let expected_total = Some(expected_frames(
            samples.len() as u64,
            settings().frame,
            settings().hop_samples(),
        ));
        let mut calls = Vec::new();
        let analysis =
            analyze_samples_with_progress(samples.clone(), 16_000.0, &settings(), |done, total| {
                calls.push((done, total));
                true
            })
            .expect("analysis");
        assert_eq!(calls.len(), analysis.spectrogram.frames());
        assert_eq!(calls.first().unwrap().0, 1);
        assert_eq!(calls.last().unwrap().0, analysis.spectrogram.frames());
        assert_eq!(calls.last().unwrap().1, expected_total);

        let err =
            match analyze_samples_with_progress(samples, 16_000.0, &settings(), |done, _| done < 2)
            {
                Ok(_) => panic!("the second frame must cancel the analysis"),
                Err(e) => e,
            };
        assert!(err.to_string().contains("cancelled"));
    }

    #[test]
    fn sample_api_rejects_an_invalid_rate_and_empty_signal() {
        let err = match analyze_samples(vec![0.0; 100], 0.0, &settings()) {
            Ok(_) => panic!("a zero sample rate must be rejected"),
            Err(e) => e,
        };
        assert!(err.to_string().contains("sample rate"));

        let err = match analyze_samples(Vec::new(), 16_000.0, &settings()) {
            Ok(_) => panic!("an empty signal must be rejected"),
            Err(e) => e,
        };
        assert!(err.to_string().contains("no samples"));
    }

    /// A 1 s, 2500 Hz four-tone file chosen so the default 2048-sample frame
    /// leaves a partial final frame. A zero-padded tail puts a step inside the
    /// window, which shows up as a frame-to-frame level/leakage jump. The tail
    /// must be aligned to the end of the signal instead.
    #[test]
    fn stationary_tones_keep_a_uniform_spectrum_in_a_partial_tail_frame() {
        let sample_rate = 2500.0;
        let samples: Vec<f32> = (0..2500)
            .map(|index| {
                let t = index as f64 / sample_rate;
                (1.5 * (2.0 * std::f64::consts::PI * 75.0 * t + 0.2).sin()
                    + 0.8 * (2.0 * std::f64::consts::PI * 240.0 * t - 0.7).sin()
                    + 1.2 * (2.0 * std::f64::consts::PI * 520.0 * t + 1.1).sin()
                    + 0.5 * (2.0 * std::f64::consts::PI * 850.0 * t).sin()) as f32
            })
            .collect();

        let analysis = analyze_samples(samples, sample_rate, &AnalysisSettings::default()).unwrap();
        let spec = &analysis.spectrogram;
        assert_eq!(spec.frames(), 2, "the partial tail frame is analysed");
        assert!(
            (spec.end_s() - 1.0).abs() < 1e-9,
            "the last frame must cover the end of the file, got {} s",
            spec.end_s()
        );

        for (frequency, amplitude) in [(75.0, 1.5), (240.0, 0.8), (520.0, 1.2), (850.0, 0.5)] {
            let bin = (frequency / spec.bin_hz() as f64).round() as usize;
            let first = spec.column(0)[bin];
            let last = spec.column(1)[bin];
            assert!(
                (first - last).abs() < 0.1,
                "{frequency} Hz ({amplitude} amplitude) changed from {first:.2} to {last:.2} dB"
            );
        }

        // Leakage in the visible 100 dB color range must not jump between the
        // full frame and the end-aligned tail frame.
        let floor = spec.max_db() - 100.0;
        let (worst_bin, worst) = (0..spec.bins())
            .filter_map(|bin| {
                let a = spec.column(0)[bin];
                let b = spec.column(1)[bin];
                (a.max(b) > floor).then_some((bin, (a - b).abs()))
            })
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap_or((0, 0.0));
        assert!(
            worst < 0.5,
            "spectral leakage jumps by {worst:.2} dB at bin {worst_bin} ({} Hz)",
            worst_bin as f32 * spec.bin_hz()
        );
    }
}
