//! Ready-made stages, thin wrappers around existing crates.
//! Each one is a few lines; copy them as templates for your own.

use apodize::{blackman_iter, hamming_iter, hanning_iter, nuttall_iter, triangular_iter};
use biquad::{Biquad as _, Coefficients, DirectForm2Transposed, Hertz, Type};
use realfft::num_complex::Complex;

use crate::traits::*;

// ---------------------------------------------------------------- windows ---

/// Applies a window to every frame. Coefficients are cached per frame length.
/// Custom windows: `Window::new(|n| -> Vec<f32> { ... })`.
pub struct Window {
    make: Box<dyn Fn(usize) -> Vec<f32>>,
    cache: Vec<f32>,
}

impl Window {
    pub fn new(make: impl Fn(usize) -> Vec<f32> + 'static) -> Self {
        Self {
            make: Box::new(make),
            cache: Vec::new(),
        }
    }
    pub fn rectangular() -> Self {
        Self::new(|n| vec![1.0; n])
    }
    pub fn hann() -> Self {
        Self::new(|n| hanning_iter(n).map(|w| w as f32).collect())
    }
    pub fn hamming() -> Self {
        Self::new(|n| hamming_iter(n).map(|w| w as f32).collect())
    }
    pub fn blackman() -> Self {
        Self::new(|n| blackman_iter(n).map(|w| w as f32).collect())
    }
    pub fn nuttall() -> Self {
        Self::new(|n| nuttall_iter(n).map(|w| w as f32).collect())
    }
    pub fn triangular() -> Self {
        Self::new(|n| triangular_iter(n).map(|w| w as f32).collect())
    }
}

impl FrameStage for Window {
    fn process(&mut self, frame: &mut [f32]) {
        if self.cache.len() != frame.len() {
            self.cache = (self.make)(frame.len());
        }
        for (x, w) in frame.iter_mut().zip(&self.cache) {
            *x *= w;
        }
    }
}

// ---------------------------------------------------------------- filters ---

pub const BUTTERWORTH_Q: f32 = biquad::Q_BUTTERWORTH_F32;

/// Second order IIR filter on the sample stream (wraps the `biquad` crate).
/// Cascade several for steeper slopes.
pub struct Filter(DirectForm2Transposed<f32>);

impl Filter {
    fn build(kind: Type<f32>, sample_rate: f64, freq_hz: f32, q: f32) -> Result<Self> {
        let hz = |v: f32| Hertz::<f32>::from_hz(v).map_err(|e| format!("bad frequency: {e:?}"));
        let coeffs =
            Coefficients::<f32>::from_params(kind, hz(sample_rate as f32)?, hz(freq_hz)?, q)
                .map_err(|e| format!("invalid filter parameters: {e:?}"))?;
        Ok(Self(DirectForm2Transposed::<f32>::new(coeffs)))
    }

    pub fn lowpass(sample_rate: f64, cutoff_hz: f32, q: f32) -> Result<Self> {
        Self::build(Type::LowPass, sample_rate, cutoff_hz, q)
    }
    pub fn highpass(sample_rate: f64, cutoff_hz: f32, q: f32) -> Result<Self> {
        Self::build(Type::HighPass, sample_rate, cutoff_hz, q)
    }
    pub fn bandpass(sample_rate: f64, center_hz: f32, q: f32) -> Result<Self> {
        Self::build(Type::BandPass, sample_rate, center_hz, q)
    }
    pub fn notch(sample_rate: f64, center_hz: f32, q: f32) -> Result<Self> {
        Self::build(Type::Notch, sample_rate, center_hz, q)
    }
}

impl StreamStage for Filter {
    fn process(&mut self, samples: &mut [f32]) {
        for s in samples {
            *s = self.0.run(*s);
        }
    }
}

// --------------------------------------------------------- spectrum stages ---

/// Multiply all bins by a constant.
pub struct Scale(pub f32);
impl SpectrumStage for Scale {
    fn process(&mut self, spec: &mut Spectrum) {
        for b in &mut spec.bins {
            *b *= self.0;
        }
    }
}

/// Zero every bin outside `[min_hz, max_hz]`.
pub struct BandLimit {
    pub min_hz: f32,
    pub max_hz: f32,
}
impl SpectrumStage for BandLimit {
    fn process(&mut self, spec: &mut Spectrum) {
        let bin_hz = spec.bin_hz;
        for (i, b) in spec.bins.iter_mut().enumerate() {
            let f = i as f32 * bin_hz;
            if f < self.min_hz || f > self.max_hz {
                *b = Complex::new(0.0, 0.0);
            }
        }
    }
}

/// Reduce frequency resolution by merging `factor` neighbouring bins into one.
/// The merged bin holds the RMS magnitude (phase is discarded).
pub struct MergeBins(pub usize);
impl SpectrumStage for MergeBins {
    fn process(&mut self, spec: &mut Spectrum) {
        let f = self.0.max(1);
        spec.bins = spec
            .bins
            .chunks(f)
            .map(|c| {
                let power = c.iter().map(|z| z.norm_sqr()).sum::<f32>() / c.len() as f32;
                Complex::new(power.sqrt(), 0.0)
            })
            .collect();
        spec.bin_hz *= f as f32;
    }
}
