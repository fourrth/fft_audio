//! All extension points live here. Implement these for your own types.

use realfft::num_complex::Complex;

pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T> = std::result::Result<T, Error>;

/// Where samples come from: an audio file (WAV, FLAC, ...), a column of a data
/// table (CSV, Parquet, ...), a microphone, the network, ...
///
/// Implementors are responsible for decoding and for downmixing to mono.
pub trait SignalSource {
    /// Samples per second. Not necessarily an integer: a table logged once a
    /// minute has a sample rate of 1/60 Hz.
    fn sample_rate(&self) -> f64;

    /// Fill `buf` with mono samples (nominally in -1.0..=1.0).
    /// Returns the number of samples written; `0` means end of stream.
    fn read(&mut self, buf: &mut [f32]) -> Result<usize>;

    /// Total mono samples, when known (used for analysis progress / ETA).
    fn expected_samples(&self) -> Option<u64> {
        None
    }
}

/// Stateful processing of the continuous sample stream, before it is cut into
/// frames. Good for filters, DC removal, gain, resampling-like effects.
pub trait StreamStage {
    fn process(&mut self, samples: &mut [f32]);
}

/// Processing of a single time-domain frame, right before the FFT.
/// Good for window functions, per-frame normalisation, etc.
pub trait FrameStage {
    fn process(&mut self, frame: &mut [f32]);
}

/// Processing of the spectrum right after the FFT.
/// Good for scaling, band limiting, changing frequency resolution, smoothing...
pub trait SpectrumStage {
    fn process(&mut self, spectrum: &mut Spectrum);
}

/// Receives every computed spectrum (print it, plot it, write it to disk, ...).
pub trait SpectrumSink {
    fn consume(&mut self, spectrum: &Spectrum) -> Result<()>;
}

// Closures work as stages / sinks, so quick experiments are one-liners.
// (Annotate the closure parameter type, e.g. `|s: &mut [f32]| ...`.)
impl<F: FnMut(&mut [f32])> StreamStage for F {
    fn process(&mut self, samples: &mut [f32]) {
        self(samples)
    }
}
impl<F: FnMut(&mut [f32])> FrameStage for F {
    fn process(&mut self, frame: &mut [f32]) {
        self(frame)
    }
}
impl<F: FnMut(&mut Spectrum)> SpectrumStage for F {
    fn process(&mut self, spectrum: &mut Spectrum) {
        self(spectrum)
    }
}
impl<F: FnMut(&Spectrum) -> Result<()>> SpectrumSink for F {
    fn consume(&mut self, spectrum: &Spectrum) -> Result<()> {
        self(spectrum)
    }
}

/// One FFT result (bins 0..=N/2 for an N point real FFT).
#[derive(Clone, Debug)]
pub struct Spectrum {
    pub bins: Vec<Complex<f32>>,
    /// Frequency distance between neighbouring bins. Stages that change the
    /// resolution must update this.
    pub bin_hz: f32,
    pub frame_index: usize,
    /// Start time of the frame in seconds.
    pub time_s: f64,
}

impl Spectrum {
    pub fn freq(&self, bin: usize) -> f32 {
        bin as f32 * self.bin_hz
    }

    pub fn magnitude(&self, bin: usize) -> f32 {
        self.bins[bin].norm()
    }

    pub fn magnitudes(&self) -> impl Iterator<Item = f32> + '_ {
        self.bins.iter().map(|c| c.norm())
    }

    /// `(frequency_hz, magnitude)` of the strongest bin, ignoring DC.
    pub fn peak(&self) -> Option<(f32, f32)> {
        self.magnitudes()
            .enumerate()
            .skip(1)
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, m)| (self.freq(i), m))
    }
}
