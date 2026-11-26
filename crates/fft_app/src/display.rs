//! Display records, ids, and typed display-open options.
//!
//! Displays are app-level: core DSP and the runtime never know about them.
//! Backend/script displays use the reserved id range starting at
//! [`SCRIPTED_DISPLAY_ID_BASE`]; frontend user displays stay below it.
//!
//! [`DisplayInfo`] is the wire shape shared with HTTP/Tauri. Owner and the
//! concrete analysis/signal/cut handles stay in [`DisplayRecord`] and are
//! never added to the wire type.

use std::sync::atomic::{AtomicU64, Ordering};

use fft_core::session::{JsonSessionDisplay, SessionDocument};
use fft_core::settings::{DisplaySettings, LineCutSettings, OrderCutSettings};
use fft_core::table::PlotLayout;
use fft_runtime::{AnalysisId, CutId, DisplayId, RpmProfileSpec, SignalId};
use serde::{Deserialize, Serialize};

/// First display id reserved for backend/script-created displays.
pub const SCRIPTED_DISPLAY_ID_BASE: u64 = 1_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayKind {
    Spectrogram,
    OrderCuts,
    LineCuts,
    Plot,
}

impl DisplayKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            DisplayKind::Spectrogram => "spectrogram",
            DisplayKind::OrderCuts => "order_cuts",
            DisplayKind::LineCuts => "line_cuts",
            DisplayKind::Plot => "plot",
        }
    }
}

/// Who created a display. Internal only; deliberately not part of
/// [`DisplayInfo`]'s wire shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayOwner {
    User,
    Backend,
}

impl DisplayOwner {
    pub const fn as_str(self) -> &'static str {
        match self {
            DisplayOwner::User => "user",
            DisplayOwner::Backend => "backend",
        }
    }
}

/// One display record. `state` is app-specific display JSON (analysis id,
/// cut id, settings, ...), deliberately not core DSP data.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayInfo {
    pub id: DisplayId,
    pub parent: Option<DisplayId>,
    pub kind: DisplayKind,
    pub name: String,
    pub state: serde_json::Value,
    pub visible: bool,
    /// In-memory/script displays are ephemeral and must not be persisted or
    /// added to the undo stack by the frontend.
    #[serde(default)]
    pub ephemeral: bool,
}

impl DisplayInfo {
    /// Convert to the core `session.json` display shape.
    ///
    /// The caller is responsible for filtering ephemeral displays first; see
    /// [`crate::DisplayRegistry::session_displays`].
    pub fn to_json_session_display(&self) -> JsonSessionDisplay {
        JsonSessionDisplay {
            id: self.id.get(),
            parent: self.parent.map(DisplayId::get),
            kind: self.kind.as_str().to_string(),
            name: self.name.clone(),
            state: self.state.clone(),
        }
    }
}

/// Full registry record. `info` is the transport-facing metadata; the typed
/// handles are used for attach and lifecycle close.
#[derive(Clone, Debug, PartialEq)]
pub struct DisplayRecord {
    pub info: DisplayInfo,
    pub analysis: Option<AnalysisId>,
    pub signal: Option<SignalId>,
    pub cut: Option<CutId>,
    pub owner: DisplayOwner,
}

/// Monotonic allocator for backend-created display ids.
///
/// The allocator never hands out an id below [`SCRIPTED_DISPLAY_ID_BASE`] and
/// seeds itself above every id already present in the persisted session, so a
/// restart cannot reuse an id after close.
#[derive(Debug)]
pub struct DisplayIdAllocator {
    next: AtomicU64,
}

impl DisplayIdAllocator {
    pub fn new(persisted: Option<&SessionDocument>) -> Self {
        let highest = persisted
            .map(|session| {
                session
                    .displays
                    .iter()
                    .map(|display| display.id)
                    .max()
                    .unwrap_or(0)
            })
            .unwrap_or(0);
        let next = SCRIPTED_DISPLAY_ID_BASE.max(highest.saturating_add(1));
        Self {
            next: AtomicU64::new(next),
        }
    }

    /// Allocate the next backend display id. Ids are never reused.
    pub fn allocate(&self) -> DisplayId {
        DisplayId(self.next.fetch_add(1, Ordering::SeqCst))
    }
}

impl Default for DisplayIdAllocator {
    fn default() -> Self {
        Self::new(None)
    }
}

/// Options for opening a spectrogram display.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SpectrogramDisplayOptions {
    /// Caller-supplied display name; takes precedence over the analysis name.
    pub name: Option<String>,
    /// Display-side view settings (log axis, max frequency, dB range).
    pub settings: DisplaySettings,
    pub visible: bool,
    /// In-memory displays are not persisted by the frontend session.
    #[serde(default)]
    pub ephemeral: bool,
}

impl Default for SpectrogramDisplayOptions {
    fn default() -> Self {
        Self {
            name: None,
            settings: DisplaySettings::default(),
            visible: true,
            ephemeral: false,
        }
    }
}

impl SpectrogramDisplayOptions {
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn with_ephemeral(mut self, ephemeral: bool) -> Self {
        self.ephemeral = ephemeral;
        self
    }
}

/// Cut-display settings. The variant must match the `show_*` call.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "settings", rename_all = "snake_case")]
pub enum CutDisplaySettings {
    Order(OrderCutSettings),
    Line(LineCutSettings),
}

/// Options for opening an Order Cuts or Line Cuts child display.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CutDisplayOptions {
    pub name: Option<String>,
    pub settings: CutDisplaySettings,
    pub visible: bool,
    /// Optional non-constant RPM source used by the cut request. Kept with the
    /// display so variable-fundamental graphs can draw the same profile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rpm: Option<RpmProfileSpec>,
    /// In-memory displays are not persisted by the frontend session.
    #[serde(default)]
    pub ephemeral: bool,
}

impl CutDisplayOptions {
    pub fn order(settings: OrderCutSettings) -> Self {
        Self {
            name: None,
            settings: CutDisplaySettings::Order(settings),
            visible: true,
            rpm: None,
            ephemeral: false,
        }
    }

    pub fn line(settings: LineCutSettings) -> Self {
        Self {
            name: None,
            settings: CutDisplaySettings::Line(settings),
            visible: true,
            rpm: None,
            ephemeral: false,
        }
    }

    pub fn with_rpm(mut self, rpm: RpmProfileSpec) -> Self {
        self.rpm = Some(rpm);
        self
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn with_visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }

    pub fn with_ephemeral(mut self, ephemeral: bool) -> Self {
        self.ephemeral = ephemeral;
        self
    }
}

/// Options for opening a standalone generic Plot display.
///
/// The display keeps the table path and column layout in its state; the
/// transport-side `plot.load` command maps the file onto x/y series.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PlotDisplayOptions {
    /// Caller-supplied display name; defaults to the table file name.
    pub name: Option<String>,
    /// Table file to plot.
    pub path: String,
    /// How columns map onto lines.
    pub layout: PlotLayout,
    pub visible: bool,
    /// In-memory displays are not persisted by the frontend session.
    pub ephemeral: bool,
}

impl Default for PlotDisplayOptions {
    fn default() -> Self {
        Self {
            name: None,
            path: String::new(),
            layout: PlotLayout::Columns,
            visible: true,
            ephemeral: false,
        }
    }
}

impl PlotDisplayOptions {
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            ..Self::default()
        }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn with_layout(mut self, layout: PlotLayout) -> Self {
        self.layout = layout;
        self
    }

    pub fn with_visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }

    pub fn with_ephemeral(mut self, ephemeral: bool) -> Self {
        self.ephemeral = ephemeral;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fft_core::session::{JsonSessionDisplay, SESSION_VERSION};

    #[test]
    fn display_info_serializes_camel_case_and_snake_case_enums() {
        let info = DisplayInfo {
            id: DisplayId(SCRIPTED_DISPLAY_ID_BASE),
            parent: None,
            kind: DisplayKind::OrderCuts,
            name: "orders".into(),
            state: serde_json::json!({ "cutId": 4 }),
            visible: true,
            ephemeral: false,
        };
        let value = serde_json::to_value(&info).unwrap();
        assert_eq!(value["id"], SCRIPTED_DISPLAY_ID_BASE);
        assert_eq!(value["kind"], "order_cuts");
        assert_eq!(value["name"], "orders");
        assert_eq!(value["state"]["cutId"], 4);
        assert_eq!(value["ephemeral"], false);
        assert!(value.get("parent").is_some());
        assert!(
            value.get("owner").is_none(),
            "owner is registry-internal and must stay off the wire"
        );
        let back: DisplayInfo = serde_json::from_value(value).unwrap();
        assert_eq!(back, info);
    }

    #[test]
    fn display_info_defaults_ephemeral_for_old_payloads() {
        let value = serde_json::json!({
            "id": 7,
            "parent": null,
            "kind": "spectrogram",
            "name": "old",
            "state": {},
            "visible": true
        });
        let info: DisplayInfo = serde_json::from_value(value).unwrap();
        assert!(!info.ephemeral);
    }

    #[test]
    fn cut_options_keep_settings_typed() {
        let options = CutDisplayOptions::order(OrderCutSettings::default())
            .with_name("harmonics")
            .with_visible(false);
        assert!(matches!(options.settings, CutDisplaySettings::Order(_)));
        assert!(!options.visible);
        let json = serde_json::to_value(&options).unwrap();
        assert_eq!(json["settings"]["kind"], "order");
        assert_eq!(json["name"], "harmonics");
        assert!(json.get("rpm").is_none());
    }

    #[test]
    fn cut_options_serialize_an_optional_rpm_profile() {
        let options = CutDisplayOptions::order(OrderCutSettings::default()).with_rpm(
            fft_runtime::RpmProfileSpec::Points(vec![fft_runtime::RpmPoint {
                time_s: 0.0,
                rpm: 6_000.0,
            }]),
        );
        let json = serde_json::to_value(&options).unwrap();
        assert_eq!(json["rpm"]["kind"], "points");
        assert_eq!(json["rpm"]["value"][0]["timeS"], 0.0);
        let back: CutDisplayOptions = serde_json::from_value(json).unwrap();
        assert_eq!(back, options);
    }

    #[test]
    fn plot_options_round_trip_path_layout_and_defaults() {
        let options = PlotDisplayOptions::new("series.csv")
            .with_layout(PlotLayout::Pairs)
            .with_name("pairs")
            .with_visible(false);
        let json = serde_json::to_value(&options).unwrap();
        assert_eq!(json["path"], "series.csv");
        assert_eq!(json["layout"], "pairs");
        assert_eq!(json["name"], "pairs");
        assert_eq!(json["visible"], false);
        let back: PlotDisplayOptions = serde_json::from_value(json).unwrap();
        assert_eq!(back, options);
        let default = PlotDisplayOptions::default();
        assert_eq!(default.layout, PlotLayout::Columns);
        assert!(default.visible);
        assert!(!default.ephemeral);
    }

    #[test]
    fn spectrogram_options_default_to_visible() {
        let options = SpectrogramDisplayOptions::default();
        assert!(options.visible);
        assert!(options.name.is_none());
        assert_eq!(options.settings, DisplaySettings::default());
    }

    #[test]
    fn display_owner_is_snake_case() {
        assert_eq!(
            serde_json::to_string(&DisplayOwner::User).unwrap(),
            r#""user""#
        );
        assert_eq!(
            serde_json::to_string(&DisplayOwner::Backend).unwrap(),
            r#""backend""#
        );
    }

    #[test]
    fn allocator_starts_at_reserved_base_and_never_reuses() {
        let allocator = DisplayIdAllocator::new(None);
        assert_eq!(
            allocator.allocate(),
            DisplayId(SCRIPTED_DISPLAY_ID_BASE),
            "fresh allocator starts in the reserved range"
        );
        assert_eq!(
            allocator.allocate(),
            DisplayId(SCRIPTED_DISPLAY_ID_BASE + 1)
        );
    }

    #[test]
    fn allocator_seeds_from_all_persisted_display_ids() {
        let session = SessionDocument {
            version: SESSION_VERSION,
            next_id: 2,
            displays: vec![
                JsonSessionDisplay {
                    id: 1,
                    parent: None,
                    kind: "spectrogram".into(),
                    name: "frontend".into(),
                    state: serde_json::json!({}),
                },
                JsonSessionDisplay {
                    id: SCRIPTED_DISPLAY_ID_BASE + 5,
                    parent: Some(1),
                    kind: "order_cuts".into(),
                    name: "script".into(),
                    state: serde_json::json!({}),
                },
            ],
            ..SessionDocument::default()
        };
        let allocator = DisplayIdAllocator::new(Some(&session));
        assert_eq!(
            allocator.allocate(),
            DisplayId(SCRIPTED_DISPLAY_ID_BASE + 6)
        );
        assert_eq!(
            allocator.allocate(),
            DisplayId(SCRIPTED_DISPLAY_ID_BASE + 7)
        );
    }

    #[test]
    fn allocator_respects_a_higher_persisted_id() {
        let session = SessionDocument {
            displays: vec![JsonSessionDisplay {
                id: 2_000_000,
                parent: None,
                kind: "spectrogram".into(),
                name: "high".into(),
                state: serde_json::json!({}),
            }],
            ..SessionDocument::default()
        };
        let allocator = DisplayIdAllocator::new(Some(&session));
        assert_eq!(allocator.allocate(), DisplayId(2_000_001));
    }

    #[test]
    fn display_info_converts_to_json_session_displays() {
        let info = DisplayInfo {
            id: DisplayId(SCRIPTED_DISPLAY_ID_BASE + 2),
            parent: Some(DisplayId(1)),
            kind: DisplayKind::LineCuts,
            name: "lines".into(),
            state: serde_json::json!({ "analysisId": 3, "cutId": 4 }),
            visible: false,
            ephemeral: false,
        };

        let json = info.to_json_session_display();
        assert_eq!(json.id, SCRIPTED_DISPLAY_ID_BASE + 2);
        assert_eq!(json.parent, Some(1));
        assert_eq!(json.kind, "line_cuts");
        assert_eq!(json.name, "lines");
        assert_eq!(json.state, info.state);
    }
}
