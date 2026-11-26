//! Local configuration, theme, session, and script-file helpers for this adapter.
//!
//! These are pure filesystem operations keyed by the desktop config directory,
//! so command cores can be tested without a Tauri `AppHandle`.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use fft_core::orders;
use fft_core::script_runner::write_script_example;
use fft_core::session::SessionDocument;
use fft_core::settings::{AnalysisSettings, DisplaySettings, Settings};
use fft_core::theme::{theme_slug, ResolvedTheme, ThemeEntry, ThemeLoader, BUILTIN_IDS};

use crate::dto::{ConfigPaths, FileStat, ScriptExampleInfo, ScriptPathResult};
use crate::embedded_examples::EMBEDDED_EXAMPLES;

#[derive(Clone, Debug)]
pub(crate) struct FilePaths {
    config_dir: PathBuf,
}

impl FilePaths {
    pub(crate) fn new(config_dir: impl Into<PathBuf>) -> Self {
        Self {
            config_dir: config_dir.into(),
        }
    }

    pub(crate) fn themes_dir(&self) -> PathBuf {
        self.config_dir.join("themes")
    }

    pub(crate) fn settings_path(&self) -> PathBuf {
        self.config_dir.join("settings.toml")
    }

    pub(crate) fn session_path(&self) -> PathBuf {
        self.config_dir.join("session.json")
    }

    pub(crate) fn config_paths(&self) -> Result<ConfigPaths, String> {
        let themes = self.themes_dir();
        std::fs::create_dir_all(&themes).map_err(|error| error.to_string())?;
        Ok(ConfigPaths {
            config_dir: self.config_dir.to_string_lossy().into_owned(),
            settings_path: self.settings_path().to_string_lossy().into_owned(),
            session_path: self.session_path().to_string_lossy().into_owned(),
            themes_dir: themes.to_string_lossy().into_owned(),
        })
    }

    pub(crate) fn get_settings(&self) -> Result<Settings, String> {
        let path = self.settings_path();
        if !path.exists() {
            return Ok(Settings::default());
        }
        Settings::load(&path).map_err(|error| error.to_string())
    }

    pub(crate) fn save_settings(&self, settings: &Settings) -> Result<(), String> {
        settings
            .save(&self.settings_path())
            .map_err(|error| error.to_string())
    }

    pub(crate) fn set_spectrogram_defaults(
        &self,
        analysis: AnalysisSettings,
        display: DisplaySettings,
    ) -> Result<(), String> {
        let mut settings = self.get_settings()?;
        settings.analysis = analysis;
        settings.display = display;
        self.save_settings(&settings)
    }

    pub(crate) fn get_session(&self) -> Result<Option<SessionDocument>, String> {
        SessionDocument::load_json(&self.session_path()).map_err(|error| error.to_string())
    }

    pub(crate) fn save_session(&self, session: &SessionDocument) -> Result<(), String> {
        session
            .save_json(&self.session_path())
            .map_err(|error| error.to_string())
    }

    pub(crate) fn get_theme(&self, name: Option<&str>) -> Result<ResolvedTheme, String> {
        ThemeLoader::new(self.themes_dir())
            .load(name.unwrap_or("dark"))
            .map_err(|error| error.to_string())
    }

    pub(crate) fn list_themes(&self) -> Result<Vec<ThemeEntry>, String> {
        ThemeLoader::new(self.themes_dir())
            .list()
            .map_err(|error| error.to_string())
    }

    pub(crate) fn duplicate_theme(&self, source: &str, name: String) -> Result<String, String> {
        let dir = self.themes_dir();
        std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
        let loader = ThemeLoader::new(&dir);
        let theme = loader.load(source).map_err(|error| error.to_string())?;
        let mut id = theme_slug(&name);
        if BUILTIN_IDS.contains(&id.as_str()) {
            id.push_str("-user");
        }
        let path = dir.join(format!("{id}.toml"));
        if path.exists() {
            return Err(format!("a theme named {id:?} already exists"));
        }
        theme
            .to_document(name)
            .save(&path)
            .map_err(|error| error.to_string())?;
        Ok(id)
    }

    /// Bundled Python examples, sorted by flattened filename.
    pub(crate) fn list_script_examples(&self) -> Vec<ScriptExampleInfo> {
        EMBEDDED_EXAMPLES
            .iter()
            .map(|example| ScriptExampleInfo {
                filename: example.filename.to_string(),
            })
            .collect()
    }

    /// Copy one bundled example into `<config>/scripting-examples/`.
    ///
    /// Only exact embedded filenames are accepted; the filename is validated
    /// again by the shared writer before it is joined to the destination.
    pub(crate) fn create_script_example(&self, filename: &str) -> Result<ScriptPathResult, String> {
        let example = EMBEDDED_EXAMPLES
            .iter()
            .find(|example| example.filename == filename)
            .ok_or_else(|| format!("unknown script example {filename:?}"))?;
        let path = write_script_example(&self.config_dir, example.filename, example.source)
            .map_err(|error| error.to_string())?;
        Ok(ScriptPathResult {
            path: path.to_string_lossy().into_owned(),
        })
    }

    /// Write a browser recording into `<config>/recordings/`.
    ///
    /// The frontend supplies a timestamped file name and its real audio
    /// extension; the name is sanitised before it is joined to the folder.
    pub(crate) fn save_recording(&self, filename: &str, bytes: &[u8]) -> Result<PathBuf, String> {
        if bytes.is_empty() {
            return Err("the recording is empty".to_string());
        }
        let dir = self.config_dir.join("recordings");
        std::fs::create_dir_all(&dir)
            .map_err(|error| format!("could not create {}: {error}", dir.display()))?;
        let name = sanitize_recording_name(filename);
        let mut path = dir.join(&name);
        let stem = name
            .rsplit_once('.')
            .map(|(stem, _)| stem)
            .unwrap_or("recording")
            .to_string();
        let extension = name
            .rsplit_once('.')
            .map(|(_, extension)| extension)
            .unwrap_or("webm")
            .to_string();
        let mut suffix = 1;
        while path.exists() {
            path = dir.join(format!("{stem}-{suffix}.{extension}"));
            suffix += 1;
        }
        std::fs::write(&path, bytes)
            .map_err(|error| format!("could not write {}: {error}", path.display()))?;
        Ok(path)
    }
}

/// Keep only the file name and characters that are safe on every desktop OS.
fn sanitize_recording_name(filename: &str) -> String {
    let base = Path::new(filename)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("recording.webm");
    let cleaned: String = base
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect();
    let cleaned = cleaned.trim_start_matches('.').to_string();
    if cleaned.is_empty() || !cleaned.contains('.') {
        "recording.webm".to_string()
    } else {
        cleaned
    }
}

pub(crate) fn read_script_file(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|error| format!("could not read {path}: {error}"))
}

pub(crate) fn write_script_file(path: &str, contents: &str) -> Result<(), String> {
    std::fs::write(path, contents).map_err(|error| format!("could not write {path}: {error}"))
}

pub(crate) fn script_stat(path: &str) -> FileStat {
    let metadata = std::fs::metadata(path);
    let mtime_ms = metadata
        .ok()
        .and_then(|meta| meta.modified().ok())
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as u64);
    FileStat {
        exists: mtime_ms.is_some(),
        mtime_ms,
    }
}

pub(crate) fn export_cuts_csv(path: &Path, cuts: &[fft_core::orders::Cut]) -> Result<(), String> {
    orders::write_csv(path, cuts).map_err(|error| error.to_string())
}

/// Render an RGB PNG with the image crate's PNG encoder.
pub(crate) fn write_png(path: &Path, image: &fft_core::spectrogram::Image) -> Result<(), String> {
    let mut rgb = Vec::with_capacity(image.pixels.len() * 3);
    for &pixel in &image.pixels {
        rgb.extend_from_slice(&[(pixel >> 16) as u8, (pixel >> 8) as u8, pixel as u8]);
    }
    let png = image::RgbImage::from_raw(image.width as u32, image.height as u32, rgb)
        .ok_or_else(|| "internal image size mismatch".to_string())?;
    png.save(path).map_err(|error| error.to_string())
}
