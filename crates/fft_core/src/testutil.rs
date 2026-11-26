//! Synthetic audio for tests (compiled only for `cargo test`).

use std::path::PathBuf;

/// Write mono 16-bit PCM WAV and return its path (in the temp dir).
pub fn write_wav(name: &str, sample_rate: u32, samples: &[f32]) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "fft_audio_test_{}_{}.wav",
        std::process::id(),
        name
    ));
    let data_len = (samples.len() * 2) as u32;
    let mut b: Vec<u8> = Vec::with_capacity(44 + data_len as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    b.extend_from_slice(&1u16.to_le_bytes()); // PCM
    b.extend_from_slice(&1u16.to_le_bytes()); // mono
    b.extend_from_slice(&sample_rate.to_le_bytes());
    b.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    b.extend_from_slice(&2u16.to_le_bytes()); // block align
    b.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for &s in samples {
        b.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    std::fs::write(&path, b).expect("write test wav");
    path
}

/// Sum of steady sine tones `(frequency_hz, amplitude)`.
pub fn tones(sample_rate: u32, secs: f64, tones: &[(f64, f32)]) -> Vec<f32> {
    let n = (sample_rate as f64 * secs) as usize;
    (0..n)
        .map(|i| {
            let t = i as f64 / sample_rate as f64;
            tones
                .iter()
                .map(|&(f, a)| a * (2.0 * std::f64::consts::PI * f * t).sin() as f32)
                .sum()
        })
        .collect()
}

/// Linear chirp from `f0` to `f1` Hz over `secs`.
pub fn chirp(sample_rate: u32, secs: f64, f0: f64, f1: f64, amp: f32) -> Vec<f32> {
    let n = (sample_rate as f64 * secs) as usize;
    let k = (f1 - f0) / secs;
    (0..n)
        .map(|i| {
            let t = i as f64 / sample_rate as f64;
            amp * (2.0 * std::f64::consts::PI * (f0 * t + 0.5 * k * t * t)).sin() as f32
        })
        .collect()
}
