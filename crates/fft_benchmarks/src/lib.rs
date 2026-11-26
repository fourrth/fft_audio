//! Shared inputs and helpers for `fft_core` benchmarks.
//!
//! The Rust and Python comparisons use the same deterministic samples, frame
//! size, hop, rectangular window, and dB scaling. Python benchmarks run direct
//! NumPy FFT code, not the app/runtime adapters, so both sides measure compute.

use fft_core::pipeline::{analyze_source, Analysis};
use fft_core::settings::{AnalysisSettings, WindowKind};
use fft_core::traits::{Result, SignalSource};

pub const SAMPLE_RATE: f64 = 48_000.0;
pub const FRAME_COUNT: usize = 64;
pub const FRAME_SIZES: &[usize] = &[1024, 4096, 16384];

/// Matched analysis settings: no padding, no filters, no window, no smoothing.
pub fn settings(frame: usize) -> AnalysisSettings {
    AnalysisSettings {
        frame,
        hop: Some(frame / 2),
        pad: 1,
        window: WindowKind::None,
        highpass: None,
        lowpass: None,
        smooth_s: 0.0,
        data: Default::default(),
    }
}

/// Deterministic multi-tone signal with a little pseudo-noise.
pub fn samples(frame: usize) -> Vec<f32> {
    let hop = frame / 2;
    let len = frame + (FRAME_COUNT - 1) * hop;
    let mut samples = Vec::with_capacity(len);
    let mut noise = 0x1234_5678u32;
    for index in 0..len {
        noise = noise.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let noise = (noise as f32 / u32::MAX as f32) * 2.0 - 1.0;
        let t = index as f32 / SAMPLE_RATE as f32;
        let value = 0.45 * (std::f32::consts::TAU * 440.0 * t).sin()
            + 0.25 * (std::f32::consts::TAU * 1_000.0 * t).sin()
            + 0.10 * (std::f32::consts::TAU * 3_000.0 * t).sin()
            + 0.03 * noise;
        samples.push(value);
    }
    samples
}

/// Borrowing `SignalSource` so the benchmark measures analysis, not input
/// cloning or file decoding.
pub struct SliceSource<'a> {
    samples: &'a [f32],
    position: usize,
}

impl<'a> SliceSource<'a> {
    pub fn new(samples: &'a [f32]) -> Self {
        Self {
            samples,
            position: 0,
        }
    }
}

impl SignalSource for SliceSource<'_> {
    fn sample_rate(&self) -> f64 {
        SAMPLE_RATE
    }

    fn read(&mut self, buffer: &mut [f32]) -> Result<usize> {
        let remaining = self.samples.len() - self.position;
        let count = remaining.min(buffer.len());
        buffer[..count].copy_from_slice(&self.samples[self.position..self.position + count]);
        self.position += count;
        Ok(count)
    }

    fn expected_samples(&self) -> Option<u64> {
        Some(self.samples.len() as u64)
    }
}

/// Run the core analyzer over in-memory samples without cloning them.
pub fn analyze(samples: &[f32], settings: &AnalysisSettings) -> Result<Analysis> {
    let mut source = SliceSource::new(samples);
    analyze_source(&mut source, settings)
}
