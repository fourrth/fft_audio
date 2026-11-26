//! Streaming analysis engine: source samples → stream stages → framing →
//! frame stages → real FFT → spectrum stages → sink.
//!
//! [`AnalyzerConfig`] controls frame length, FFT size, and hop; the
//! higher-level file/table pipeline lives in [`crate::pipeline`].

use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};

use crate::traits::*;

/// Framing / resolution settings.
#[derive(Clone, Copy, Debug)]
pub struct AnalyzerConfig {
    /// Number of real samples per frame.
    pub frame_len: usize,
    /// FFT length, `>= frame_len`. Extra length is zero padded, which
    /// interpolates the spectrum (finer bin spacing, same true resolution).
    pub fft_size: usize,
    /// Samples between the starts of consecutive frames, `<= frame_len`.
    pub hop: usize,
}

impl AnalyzerConfig {
    /// 50% overlap, no zero padding.
    pub fn new(frame_len: usize) -> Self {
        Self {
            frame_len,
            fft_size: frame_len,
            hop: frame_len / 2,
        }
    }
    pub fn fft_size(mut self, n: usize) -> Self {
        self.fft_size = n;
        self
    }
    pub fn hop(mut self, n: usize) -> Self {
        self.hop = n;
        self
    }
}

struct Plan {
    fft: Arc<dyn RealToComplex<f32>>,
    input: Vec<f32>,
    output: Vec<Complex<f32>>,
    sample_rate: f64,
}

/// source -> stream stages -> framing -> frame stages -> FFT -> spectrum stages -> sink
pub struct Analyzer {
    cfg: AnalyzerConfig,
    stream: Vec<Box<dyn StreamStage>>,
    frame: Vec<Box<dyn FrameStage>>,
    spectrum: Vec<Box<dyn SpectrumStage>>,
}

impl Analyzer {
    pub fn new(cfg: AnalyzerConfig) -> Self {
        assert!(cfg.frame_len > 0, "frame_len must be > 0");
        assert!(
            cfg.hop > 0 && cfg.hop <= cfg.frame_len,
            "need 0 < hop <= frame_len"
        );
        assert!(
            cfg.fft_size >= cfg.frame_len,
            "fft_size must be >= frame_len"
        );
        Self {
            cfg,
            stream: vec![],
            frame: vec![],
            spectrum: vec![],
        }
    }

    /// Stages run in the order they are added.
    pub fn stream_stage(mut self, s: impl StreamStage + 'static) -> Self {
        self.stream.push(Box::new(s));
        self
    }
    pub fn frame_stage(mut self, s: impl FrameStage + 'static) -> Self {
        self.frame.push(Box::new(s));
        self
    }
    pub fn spectrum_stage(mut self, s: impl SpectrumStage + 'static) -> Self {
        self.spectrum.push(Box::new(s));
        self
    }

    pub fn run<S: SignalSource + ?Sized>(
        &mut self,
        source: &mut S,
        mut sink: impl SpectrumSink,
    ) -> Result<()> {
        let AnalyzerConfig {
            frame_len,
            hop,
            fft_size,
        } = self.cfg;

        let sample_rate = source.sample_rate();
        if !(sample_rate.is_finite() && sample_rate > 0.0) {
            return Err(format!("sample rate must be a positive number, got {sample_rate}").into());
        }

        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(fft_size);
        let mut plan = Plan {
            input: fft.make_input_vec(),
            output: fft.make_output_vec(),
            fft,
            sample_rate,
        };

        let mut chunk = vec![0.0f32; 4096];
        let mut pending: Vec<f32> = Vec::new();
        // Samples drained by the previous frame. They are only needed to align
        // the final partial frame to the end of the signal without padding the
        // signal with zeros inside the window.
        let mut carry: Vec<f32> = Vec::new();
        let mut seen = 0; // samples at the front of `pending` already analysed
        let mut index = 0;

        loop {
            let n = source.read(&mut chunk)?;
            if n == 0 {
                break;
            }
            let block = &mut chunk[..n];
            for stage in &mut self.stream {
                stage.process(block);
            }
            pending.extend_from_slice(block);

            while pending.len() >= frame_len {
                let start = index * hop;
                let spec = self.analyze(&mut plan, &pending[..frame_len], index, start)?;
                sink.consume(&spec)?;
                index += 1;
                carry.clear();
                carry.extend_from_slice(&pending[..hop]);
                pending.drain(..hop);
                seen = frame_len - hop;
            }
        }

        if pending.len() > seen {
            if index == 0 {
                // The whole signal is shorter than one frame. Window the
                // samples that exist and zero pad only up to the FFT size.
                let spec = self.analyze(&mut plan, &pending, 0, 0)?;
                sink.consume(&spec)?;
            } else {
                // Align the final frame to the end of the signal instead of
                // zero padding a partial frame. A zero-padded tail puts a step
                // inside the window, which makes stationary signals show a
                // spurious level/leakage change in the last frame.
                let total = index * hop + pending.len();
                let start = total.saturating_sub(frame_len);
                let need = frame_len - pending.len();
                let mut tail = Vec::with_capacity(frame_len);
                tail.extend_from_slice(&carry[carry.len().saturating_sub(need)..]);
                tail.extend_from_slice(&pending);
                debug_assert_eq!(tail.len(), frame_len, "end-aligned frame is short");
                let spec = self.analyze(&mut plan, &tail, index, start)?;
                sink.consume(&spec)?;
            }
        }
        Ok(())
    }

    fn analyze(
        &mut self,
        plan: &mut Plan,
        frame: &[f32],
        index: usize,
        start_sample: usize,
    ) -> Result<Spectrum> {
        let n = frame.len();
        plan.input[..n].copy_from_slice(frame);
        for stage in &mut self.frame {
            stage.process(&mut plan.input[..n]);
        }
        plan.input[n..].fill(0.0);

        plan.fft
            .process(&mut plan.input, &mut plan.output)
            .map_err(|e| e.to_string())?;

        let mut spec = Spectrum {
            bins: plan.output.clone(),
            bin_hz: (plan.sample_rate / self.cfg.fft_size as f64) as f32,
            frame_index: index,
            time_s: start_sample as f64 / plan.sample_rate,
        };
        for stage in &mut self.spectrum {
            stage.process(&mut spec);
        }
        Ok(spec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::WindowKind;
    use crate::sources::VecSource;

    #[test]
    fn unwindowed_bin_centered_sine_has_negligible_leakage() {
        let frame_len = 1024;
        let sample_rate = 16_000.0;
        let bin = 64; // 1000 Hz: 64 * 16000 / 1024
        let samples: Vec<f32> = (0..frame_len)
            .map(|n| {
                (2.0 * std::f64::consts::PI * bin as f64 * n as f64 / frame_len as f64).sin() as f32
            })
            .collect();

        let mut analyzer =
            Analyzer::new(AnalyzerConfig::new(frame_len)).frame_stage(WindowKind::None.stage());
        let mut source = VecSource::new(samples, sample_rate);
        let mut spectra = Vec::new();
        analyzer
            .run(&mut source, |spectrum: &Spectrum| {
                spectra.push(spectrum.clone());
                Ok(())
            })
            .unwrap();
        assert_eq!(spectra.len(), 1);
        let spectrum = &spectra[0];
        let magnitudes: Vec<f32> = spectrum.magnitudes().collect();
        let (peak_bin, peak) = magnitudes
            .iter()
            .enumerate()
            .skip(1)
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(index, value)| (index, *value))
            .unwrap();
        assert_eq!(peak_bin, bin);
        assert!(peak > 100.0);
        assert!(magnitudes[bin - 1] < peak * 1e-3);
        assert!(magnitudes[bin + 1] < peak * 1e-3);
        assert!(magnitudes[1] < peak * 1e-3);
    }
}
