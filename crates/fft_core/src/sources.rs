//! Signal sources: [`FileSource`] decodes audio with `symphonia` (plus the
//! pure-Rust Opus decoder) and downmixes it to mono, while [`VecSource`] replays
//! in-memory samples (for tables, scripts, and tests). Data tables are read by
//! [`crate::table`].

use std::fs::File;
use std::io::Cursor;
use std::path::Path;
use std::sync::OnceLock;

use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::codecs::registry::CodecRegistry;
use symphonia::core::common::Tier;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia_opus::OpusDecoder;

use crate::traits::*;

/// Symphonia's enabled codecs plus the pure-Rust Opus decoder.
///
/// Symphonia's default registry has no Opus decoder. Registering one at the
/// fallback tier keeps the built-in codecs authoritative while enabling Opus
/// in Ogg, Matroska/WebM, and ISO/MP4 containers.
fn codec_registry() -> &'static CodecRegistry {
    static REGISTRY: OnceLock<CodecRegistry> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let mut registry = CodecRegistry::new();
        symphonia::default::register_enabled_codecs(&mut registry);
        registry.register_audio_decoder_at_tier::<OpusDecoder>(Tier::Fallback);
        registry
    })
}

/// Decodes any audio file `symphonia` understands (wav, flac, mp3, ogg/vorbis,
/// aac, alac) plus Opus, and downmixes it to mono by averaging channels. Data
/// tables are read by [`crate::table`] instead.
pub struct FileSource {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    sample_rate: u32,
    /// Encoded length in samples per channel, when the container knows it.
    expected_samples: Option<u64>,
    pending: Vec<f32>, // decoded mono samples not yet handed out
    pos: usize,
    done: bool,
}

impl FileSource {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let stream = MediaSourceStream::new(Box::new(File::open(path)?), Default::default());
        Self::from_stream(stream, path.extension().and_then(|e| e.to_str()))
    }

    /// Decode an in-memory audio file. The extension hint is used only for
    /// format probing; the bytes are never written to disk. This is the entry
    /// point for browser/WASM clients.
    pub fn open_bytes(bytes: Vec<u8>, extension: Option<&str>) -> Result<Self> {
        let stream = MediaSourceStream::new(Box::new(Cursor::new(bytes)), Default::default());
        Self::from_stream(stream, extension)
    }

    fn from_stream(stream: MediaSourceStream<'static>, extension: Option<&str>) -> Result<Self> {
        let mut hint = Hint::new();
        if let Some(ext) = extension {
            hint.with_extension(ext);
        }
        let format = symphonia::default::get_probe().probe(
            &hint,
            stream,
            FormatOptions::default(),
            MetadataOptions::default(),
        )?;

        // Pick the audio track (a video track may come first in the container
        // and is not decodable here). Opus needs the container's codec params,
        // including any `dOps`/`OpusHead` extra data.
        let track = format
            .default_track(TrackType::Audio)
            .ok_or("no audio track found")?;
        let track_id = track.id;
        let expected_samples = track.num_frames;
        let track_params = track
            .codec_params
            .as_ref()
            .and_then(|params| params.audio())
            .ok_or("no audio codec parameters")?
            .clone();

        let decoder =
            codec_registry().make_audio_decoder(&track_params, &AudioDecoderOptions::default())?;
        let sample_rate = decoder
            .codec_params()
            .sample_rate
            .or(track_params.sample_rate)
            .ok_or("unknown sample rate")?;

        Ok(Self {
            format,
            decoder,
            track_id,
            sample_rate,
            expected_samples,
            pending: vec![],
            pos: 0,
            done: false,
        })
    }

    /// Decode the next packet into `pending`. `Ok(false)` means end of stream.
    fn refill(&mut self) -> Result<bool> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(Some(packet)) => packet,
                Ok(None) => return Ok(false),
                Err(SymError::ResetRequired) => return Ok(false),
                Err(error) => return Err(error.into()),
            };
            if packet.track_id != self.track_id {
                continue;
            }
            match self.decoder.decode(&packet) {
                Ok(decoded) => {
                    let channels = decoded.spec().channels().count().max(1);
                    let mut interleaved = Vec::<f32>::new();
                    decoded.copy_to_vec_interleaved(&mut interleaved);

                    self.pending.clear();
                    self.pos = 0;
                    self.pending.extend(
                        interleaved
                            .chunks(channels)
                            .map(|frame| frame.iter().sum::<f32>() / channels as f32),
                    );
                    return Ok(true);
                }
                Err(SymError::DecodeError(_)) => continue, // corrupt packet: skip it
                Err(e) => return Err(e.into()),
            }
        }
    }
}

impl SignalSource for FileSource {
    fn sample_rate(&self) -> f64 {
        self.sample_rate as f64
    }

    fn expected_samples(&self) -> Option<u64> {
        self.expected_samples
    }

    fn read(&mut self, buf: &mut [f32]) -> Result<usize> {
        let mut written = 0;
        while written < buf.len() {
            if self.pos >= self.pending.len() {
                if self.done || !self.refill()? {
                    self.done = true;
                    break;
                }
                continue;
            }
            let n = (buf.len() - written).min(self.pending.len() - self.pos);
            buf[written..written + n].copy_from_slice(&self.pending[self.pos..self.pos + n]);
            self.pos += n;
            written += n;
        }
        Ok(written)
    }
}

/// In-memory mono source: what a table column becomes, and handy for tests
/// and as a minimal example.
pub struct VecSource {
    samples: Vec<f32>,
    pos: usize,
    sample_rate: f64,
}

impl VecSource {
    pub fn new(samples: Vec<f32>, sample_rate: f64) -> Self {
        Self {
            samples,
            pos: 0,
            sample_rate,
        }
    }
}

impl SignalSource for VecSource {
    fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    fn expected_samples(&self) -> Option<u64> {
        Some(self.samples.len() as u64)
    }

    fn read(&mut self, buf: &mut [f32]) -> Result<usize> {
        let n = buf.len().min(self.samples.len() - self.pos);
        buf[..n].copy_from_slice(&self.samples[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}
