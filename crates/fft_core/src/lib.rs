//! UI-agnostic core for `fft_audio`.
//!
//! Everything here is free of Tauri/HTTP dependencies: signal decoding,
//! the FFT pipeline, spectrogram computation and rendering (software
//! framebuffer), cut maths, settings/session/theme file formats, and the
//! example-script helpers. The Tauri desktop shell and the HTTP
//! adapter only translate these types into their own widgets and transport.

pub mod analyzer;
pub mod cutsession;
pub mod orders;
pub mod pipeline;
pub mod script_runner;
pub mod session;
pub mod settings;
pub mod sources;
pub mod spectrogram;
pub mod stages;
pub mod table;
pub mod testutil;
pub mod theme;
pub mod traits;

#[cfg(test)]
mod e2e_tests;

pub use analyzer::{Analyzer, AnalyzerConfig};
pub use traits::{
    Error, FrameStage, Result, SignalSource, Spectrum, SpectrumSink, SpectrumStage, StreamStage,
};
