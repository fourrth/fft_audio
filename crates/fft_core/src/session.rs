//! UI-agnostic session documents: open displays plus their dock layout.
//!
//! `SessionDocument` is the JSON document used by the Tauri shell and headless
//! WebUI (`session.json`). It uses plain data, so display state and layout stay
//! portable and the core never depends on a UI docking engine.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::traits::Result;

/// Version written into `session.json`.
pub const SESSION_VERSION: u32 = 1;

// ------------------------------------------------------------ session.json ---

/// One display as stored in `session.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JsonSessionDisplay {
    pub id: u64,
    pub parent: Option<u64>,
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub state: serde_json::Value,
}

fn session_version() -> u32 {
    SESSION_VERSION
}

fn session_next_id() -> u64 {
    1
}

/// The JSON session document used by the Tauri/WebUI shell.
///
/// `layout` is the docking library's own serialized layout (Dockview), stored
/// opaquely so the core never depends on a UI docking engine.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionDocument {
    #[serde(default = "session_version")]
    pub version: u32,
    #[serde(default = "session_next_id")]
    pub next_id: u64,
    #[serde(default)]
    pub displays: Vec<JsonSessionDisplay>,
    #[serde(default)]
    pub recent_files: Vec<String>,
    /// User-saved Dockview layouts, keyed by preset name.
    #[serde(default)]
    pub layout_presets: std::collections::BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub layout: serde_json::Value,
}

impl Default for SessionDocument {
    fn default() -> Self {
        Self {
            version: SESSION_VERSION,
            next_id: 1,
            displays: Vec::new(),
            recent_files: Vec::new(),
            layout_presets: Default::default(),
            layout: serde_json::Value::Null,
        }
    }
}

impl SessionDocument {
    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    pub fn from_json(text: &str) -> Result<Self> {
        Ok(serde_json::from_str(text)?)
    }

    pub fn load_json(path: &Path) -> Result<Option<Self>> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(Some(Self::from_json(&text)?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save_json(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir)?;
            }
        }
        std::fs::write(path, self.to_json()?)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_session_round_trips_an_opaque_dockview_layout() {
        let session = SessionDocument {
            version: SESSION_VERSION,
            next_id: 3,
            displays: vec![JsonSessionDisplay {
                id: 1,
                parent: None,
                kind: "launcher".into(),
                name: "Launcher".into(),
                state: serde_json::json!({ "theme": "dark" }),
            }],
            recent_files: vec!["clip.wav".into()],
            layout_presets: [(
                "Focus".to_string(),
                serde_json::json!({ "grid": { "root": null } }),
            )]
            .into_iter()
            .collect(),
            layout: serde_json::json!({
                "grid": { "root": { "type": "branch", "data": [] } },
                "panels": { "panel-1": { "id": "panel-1", "title": "Launcher" } }
            }),
        };
        let text = session.to_json().unwrap();
        let back = SessionDocument::from_json(&text).unwrap();
        assert_eq!(session, back);
        assert!(text.contains("\"nextId\""));
        assert!(text.contains("\"layout\""));
    }

    #[test]
    fn json_session_file_load_save_and_missing() {
        let path = std::env::temp_dir().join(format!("fft_session_{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        assert!(SessionDocument::load_json(&path).unwrap().is_none());

        let session = SessionDocument::default();
        session.save_json(&path).unwrap();
        let back = SessionDocument::load_json(&path).unwrap().unwrap();
        assert_eq!(back, session);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn partial_json_session_gets_safe_defaults() {
        let session = SessionDocument::from_json("{}").unwrap();
        assert_eq!(session.version, SESSION_VERSION);
        assert_eq!(session.next_id, 1);
        assert!(session.displays.is_empty());
        assert!(session.recent_files.is_empty());
        assert!(session.layout_presets.is_empty());
        assert_eq!(session.layout, serde_json::Value::Null);
    }
}
