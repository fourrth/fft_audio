//! TOML themes: loading, `extends` resolution, WCAG contrast validation.
//!
//! A theme is a TOML document of CSS custom properties (keys without the
//! leading `--`), an 8-color colorblind-safe `plot_palette`, and
//! `status_{info,success,warn,error}` background/foreground pairs. Built-in
//! Dark and Light live in `assets/themes`; user themes live in the app config
//! `themes/` directory and may `extends` a built-in or another user theme.
//!
//! The frontend turns the resolved map into CSS variables (Stage 3 bridge).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::traits::Result;

const DARK_TOML: &str = include_str!("../assets/themes/dark.toml");
const LIGHT_TOML: &str = include_str!("../assets/themes/light.toml");

pub const BUILTIN_IDS: [&str; 2] = ["dark", "light"];
pub const STATUS_NAMES: [&str; 4] = ["info", "success", "warn", "error"];
pub const PLOT_PALETTE_LEN: usize = 8;
pub const MIN_CONTRAST: f64 = 4.5;

/// colors every resolved theme must provide (keys are CSS names without `--`).
pub const REQUIRED_COLORS: &[&str] = &[
    "bg-canvas",
    "bg-panel",
    "bg-card",
    "bg-input",
    "bg-hover",
    "bg-active",
    "border-subtle",
    "border-strong",
    "focus-ring",
    "text-primary",
    "text-secondary",
    "text-muted",
    "text-on-accent",
    "accent-primary",
    "kind-spec",
    "kind-order",
    "kind-line",
    "kind-launcher",
    "kind-settings",
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    #[default]
    Dark,
    Light,
}

/// Background/foreground pair for a status tone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusColors {
    pub bg: String,
    pub fg: String,
}

/// A raw theme file (before `extends` is resolved).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeDocument {
    /// Human-readable name (defaults to the file stem / built-in id).
    pub name: Option<String>,
    /// Theme this one starts from.
    pub extends: Option<String>,
    /// Absent = inherit from the base theme, or Dark for a standalone theme.
    pub appearance: Option<Appearance>,
    /// CSS custom-property name -> value.
    pub colors: BTreeMap<String, String>,
    /// Colorblind-safe series colors (8 by convention).
    pub plot_palette: Vec<String>,
    /// Status tones keyed by `info`, `success`, `warn`, `error`.
    pub status: BTreeMap<String, StatusColors>,
}

impl ThemeDocument {
    pub fn to_toml(&self) -> Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }

    pub fn from_toml(text: &str) -> Result<Self> {
        Ok(toml::from_str(text)?)
    }

    pub fn load(path: &Path) -> Result<Self> {
        Self::from_toml(&std::fs::read_to_string(path)?)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir)?;
            }
        }
        std::fs::write(path, self.to_toml()?)?;
        Ok(())
    }

    pub fn display_name<'a>(&'a self, fallback: &'a str) -> &'a str {
        self.name.as_deref().unwrap_or(fallback)
    }
}

/// A theme after `extends` resolution and with the built-in defaults applied.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedTheme {
    pub name: String,
    pub appearance: Appearance,
    pub colors: BTreeMap<String, String>,
    pub plot_palette: Vec<String>,
    pub status: BTreeMap<String, StatusColors>,
}

impl ResolvedTheme {
    pub fn color(&self, key: &str) -> Option<&str> {
        self.colors.get(key).map(String::as_str)
    }

    /// Problems that would break the theme or WCAG text contrast.
    pub fn validate(&self) -> Vec<String> {
        validate_theme(self)
    }

    /// Flatten into a standalone theme document (used by "duplicate theme").
    pub fn to_document(&self, name: impl Into<String>) -> ThemeDocument {
        ThemeDocument {
            name: Some(name.into()),
            extends: None,
            appearance: Some(self.appearance),
            colors: self.colors.clone(),
            plot_palette: self.plot_palette.clone(),
            status: self.status.clone(),
        }
    }

    pub fn preview(&self) -> Option<ThemePreview> {
        Some(ThemePreview {
            bg: self.color("bg-canvas")?.to_string(),
            panel: self.color("bg-panel")?.to_string(),
            card: self.color("bg-card")?.to_string(),
            text: self.color("text-primary")?.to_string(),
            accent: self.color("accent-primary")?.to_string(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemePreview {
    pub bg: String,
    pub panel: String,
    pub card: String,
    pub text: String,
    pub accent: String,
}

/// One entry in the theme picker.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeEntry {
    pub id: String,
    pub name: String,
    pub builtin: bool,
    pub appearance: Option<Appearance>,
    /// Swatches for the theme picker.
    pub preview: Option<ThemePreview>,
    /// Set when the file could not be parsed; the file is still listed.
    pub parse_error: Option<String>,
}

/// Loads built-in themes plus `*.toml` from a user theme directory.
#[derive(Clone, Debug)]
pub struct ThemeLoader {
    user_dir: PathBuf,
}

impl ThemeLoader {
    pub fn new(user_dir: impl Into<PathBuf>) -> Self {
        Self {
            user_dir: user_dir.into(),
        }
    }

    pub fn user_dir(&self) -> &Path {
        &self.user_dir
    }

    /// Built-in themes first, then user themes sorted by id.
    pub fn list(&self) -> Result<Vec<ThemeEntry>> {
        let mut entries = Vec::new();
        for id in BUILTIN_IDS {
            let doc = builtin_document(id)?;
            entries.push(ThemeEntry {
                id: id.to_string(),
                name: doc.display_name(id).to_string(),
                builtin: true,
                appearance: Some(doc.appearance.unwrap_or_default()),
                preview: self.load(id).ok().and_then(|theme| theme.preview()),
                parse_error: None,
            });
        }

        let dir = match std::fs::read_dir(&self.user_dir) {
            Ok(dir) => dir,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(entries),
            Err(e) => return Err(e.into()),
        };
        let mut user = Vec::new();
        for entry in dir {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if BUILTIN_IDS.contains(&id) {
                continue; // built-ins are reserved
            }
            match ThemeDocument::load(&path) {
                Ok(doc) => user.push(ThemeEntry {
                    id: id.to_string(),
                    name: doc.display_name(id).to_string(),
                    builtin: false,
                    appearance: doc.appearance,
                    preview: self.load(id).ok().and_then(|theme| theme.preview()),
                    parse_error: None,
                }),
                Err(e) => user.push(ThemeEntry {
                    id: id.to_string(),
                    name: id.to_string(),
                    builtin: false,
                    appearance: None,
                    preview: None,
                    parse_error: Some(e.to_string()),
                }),
            }
        }
        user.sort_by(|a, b| a.id.cmp(&b.id));
        entries.extend(user);
        Ok(entries)
    }

    /// Resolve a theme id (`dark`, `light`, or a user file stem).
    pub fn load(&self, id: &str) -> Result<ResolvedTheme> {
        let mut stack = Vec::new();
        self.load_with_stack(id, &mut stack)
    }

    fn load_with_stack(&self, id: &str, stack: &mut Vec<String>) -> Result<ResolvedTheme> {
        if stack.iter().any(|s| s == id) {
            let mut chain = stack.join(" -> ");
            chain.push_str(" -> ");
            chain.push_str(id);
            return Err(format!("theme extends cycle: {chain}").into());
        }
        if stack.len() >= 16 {
            return Err("theme extends chain is too deep (max 16)".into());
        }
        stack.push(id.to_string());

        let doc = self.document(id)?;
        let base = match doc.extends.as_deref() {
            Some(parent) => self.load_with_stack(parent, stack)?,
            None => builtin_theme(doc.appearance.unwrap_or_default())?,
        };

        let appearance = doc.appearance.unwrap_or(base.appearance);
        let mut colors = base.colors;
        colors.extend(doc.colors);
        let mut status = base.status;
        status.extend(doc.status);
        let plot_palette = if doc.plot_palette.is_empty() {
            base.plot_palette
        } else {
            doc.plot_palette
        };
        let name = doc.name.unwrap_or(base.name);
        stack.pop();

        Ok(ResolvedTheme {
            name,
            appearance,
            colors,
            plot_palette,
            status,
        })
    }

    fn document(&self, id: &str) -> Result<ThemeDocument> {
        match id {
            "dark" => ThemeDocument::from_toml(DARK_TOML),
            "light" => ThemeDocument::from_toml(LIGHT_TOML),
            _ => {
                if id.trim().is_empty()
                    || id.contains('/')
                    || id.contains('\\')
                    || id.contains("..")
                {
                    return Err(format!("invalid theme id {id:?}").into());
                }
                let path = self.user_dir.join(format!("{id}.toml"));
                ThemeDocument::load(&path)
                    .map_err(|e| format!("theme {id:?} ({}): {e}", path.display()).into())
            }
        }
    }
}

fn builtin_document(id: &str) -> Result<ThemeDocument> {
    match id {
        "dark" => ThemeDocument::from_toml(DARK_TOML),
        "light" => ThemeDocument::from_toml(LIGHT_TOML),
        _ => Err(format!("unknown built-in theme {id:?}").into()),
    }
}

/// File-stem-safe id for a user theme name.
pub fn theme_slug(name: &str) -> String {
    let mut slug = String::new();
    let mut dash = false;
    for ch in name.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            dash = false;
        } else if !dash && !slug.is_empty() {
            slug.push('-');
            dash = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        "theme".into()
    } else {
        slug
    }
}

/// A built-in theme with no user overrides.
pub fn builtin_theme(appearance: Appearance) -> Result<ResolvedTheme> {
    let id = match appearance {
        Appearance::Dark => "dark",
        Appearance::Light => "light",
    };
    let doc = builtin_document(id)?;
    Ok(ResolvedTheme {
        name: doc.display_name(id).to_string(),
        appearance,
        colors: doc.colors,
        plot_palette: doc.plot_palette,
        status: doc.status,
    })
}

/// Validate required tokens, palette/status colors and WCAG text contrast.
pub fn validate_theme(theme: &ResolvedTheme) -> Vec<String> {
    let mut errors = Vec::new();

    for key in REQUIRED_COLORS {
        match theme.color(key) {
            None => errors.push(format!("missing color token {key}")),
            Some(value) if parse_color(value).is_none() => {
                errors.push(format!("{key}: cannot parse color {value:?}"));
            }
            Some(_) => {}
        }
    }

    if theme.plot_palette.len() < PLOT_PALETTE_LEN {
        errors.push(format!(
            "plot_palette needs {PLOT_PALETTE_LEN} colors, got {}",
            theme.plot_palette.len()
        ));
    }
    for (i, value) in theme.plot_palette.iter().enumerate() {
        if parse_color(value).is_none() {
            errors.push(format!("plot_palette[{i}]: cannot parse color {value:?}"));
        }
    }

    for name in STATUS_NAMES {
        let Some(colors) = theme.status.get(name) else {
            errors.push(format!("missing status.{name}"));
            continue;
        };
        let surface = theme.color("bg-panel").unwrap_or("#000000");
        match contrast_ratio_on(&colors.fg, &colors.bg, surface) {
            Some(ratio) if ratio >= MIN_CONTRAST => {}
            Some(ratio) => errors.push(format!(
                "status.{name}: contrast {ratio:.2}:1 < {MIN_CONTRAST}:1"
            )),
            None => errors.push(format!("status.{name}: cannot parse its colors")),
        }
    }

    for surface in ["bg-canvas", "bg-panel"] {
        for text in ["text-primary", "text-secondary", "text-muted"] {
            check_pair(&mut errors, theme, text, surface, surface);
        }
    }
    check_pair(
        &mut errors,
        theme,
        "text-on-accent",
        "accent-primary",
        "accent-primary",
    );

    errors
}

fn check_pair(errors: &mut Vec<String>, theme: &ResolvedTheme, fg: &str, bg: &str, surface: &str) {
    let (Some(fg_value), Some(bg_value), Some(surface_value)) =
        (theme.color(fg), theme.color(bg), theme.color(surface))
    else {
        return; // missing tokens are reported separately
    };
    match contrast_ratio_on(fg_value, bg_value, surface_value) {
        Some(ratio) if ratio >= MIN_CONTRAST => {}
        Some(ratio) => errors.push(format!(
            "{fg} on {bg}: contrast {ratio:.2}:1 < {MIN_CONTRAST}:1"
        )),
        None => errors.push(format!("{fg} on {bg}: cannot parse colors")),
    }
}

// ---------------------------------------------------------------- colors ---

/// Straight RGBA color used by the theme validator.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

impl Rgba {
    fn rgb8(self) -> [u8; 3] {
        [
            self.r.round().clamp(0.0, 255.0) as u8,
            self.g.round().clamp(0.0, 255.0) as u8,
            self.b.round().clamp(0.0, 255.0) as u8,
        ]
    }

    /// Source-over compositing. `backdrop` is normally opaque.
    fn over(self, backdrop: Rgba) -> Rgba {
        let a = self.a + backdrop.a * (1.0 - self.a);
        if a <= f64::EPSILON {
            return Rgba {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            };
        }
        Rgba {
            r: (self.r * self.a + backdrop.r * backdrop.a * (1.0 - self.a)) / a,
            g: (self.g * self.a + backdrop.g * backdrop.a * (1.0 - self.a)) / a,
            b: (self.b * self.a + backdrop.b * backdrop.a * (1.0 - self.a)) / a,
            a,
        }
    }
}

/// Parse `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`, `rgb(...)` or `rgba(...)`.
pub fn parse_color(text: &str) -> Option<Rgba> {
    let text = text.trim();
    if let Some(hex) = text.strip_prefix('#') {
        return parse_hex(hex);
    }
    let lower = text.to_ascii_lowercase();
    let inner = lower
        .strip_prefix("rgba(")
        .or_else(|| lower.strip_prefix("rgb("))?
        .strip_suffix(')')?;
    let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
    if parts.len() != 3 && parts.len() != 4 {
        return None;
    }
    let channel = |s: &str| parse_channel(s, 255.0);
    Some(Rgba {
        r: channel(parts[0])?,
        g: channel(parts[1])?,
        b: channel(parts[2])?,
        a: if parts.len() == 4 {
            parse_alpha(parts[3])?
        } else {
            1.0
        },
    })
}

fn parse_hex(hex: &str) -> Option<Rgba> {
    if !hex.is_ascii() {
        return None;
    }
    let nibble = |c: char| c.to_digit(16).map(|v| v as f64 * 17.0);
    let byte = |s: &str| u8::from_str_radix(s, 16).ok().map(f64::from);
    let (r, g, b, a) = match hex.len() {
        3 => {
            let mut chars = hex.chars();
            (
                nibble(chars.next()?)?,
                nibble(chars.next()?)?,
                nibble(chars.next()?)?,
                1.0,
            )
        }
        4 => {
            let mut chars = hex.chars();
            (
                nibble(chars.next()?)?,
                nibble(chars.next()?)?,
                nibble(chars.next()?)?,
                nibble(chars.next()?)? / 255.0,
            )
        }
        6 => (byte(&hex[0..2])?, byte(&hex[2..4])?, byte(&hex[4..6])?, 1.0),
        8 => (
            byte(&hex[0..2])?,
            byte(&hex[2..4])?,
            byte(&hex[4..6])?,
            byte(&hex[6..8])? / 255.0,
        ),
        _ => return None,
    };
    Some(Rgba { r, g, b, a })
}

fn parse_channel(text: &str, max: f64) -> Option<f64> {
    if let Some(percent) = text.strip_suffix('%') {
        let value = percent.trim().parse::<f64>().ok()?;
        return Some((value / 100.0 * max).clamp(0.0, max));
    }
    Some(text.parse::<f64>().ok()?.clamp(0.0, max))
}

fn parse_alpha(text: &str) -> Option<f64> {
    if let Some(percent) = text.strip_suffix('%') {
        let value = percent.trim().parse::<f64>().ok()?;
        return Some((value / 100.0).clamp(0.0, 1.0));
    }
    Some(text.parse::<f64>().ok()?.clamp(0.0, 1.0))
}

/// WCAG 2.x relative luminance of an opaque sRGB color.
pub fn relative_luminance(rgb: [u8; 3]) -> f64 {
    fn linear(channel: u8) -> f64 {
        let c = channel as f64 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * linear(rgb[0]) + 0.7152 * linear(rgb[1]) + 0.0722 * linear(rgb[2])
}

/// WCAG contrast ratio between two opaque sRGB colors (>= 1).
pub fn contrast_ratio(a: [u8; 3], b: [u8; 3]) -> f64 {
    let (la, lb) = (relative_luminance(a), relative_luminance(b));
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// Contrast of `fg` on `bg`, where a translucent `bg` is composited over
/// `surface`. Returns None when a value cannot be parsed.
pub fn contrast_ratio_on(fg: &str, bg: &str, surface: &str) -> Option<f64> {
    let surface = parse_color(surface)?;
    let bg = parse_color(bg)?;
    let fg = parse_color(fg)?;
    let composited_bg = bg.over(surface);
    let composited_fg = fg.over(composited_bg);
    Some(contrast_ratio(composited_fg.rgb8(), composited_bg.rgb8()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fft_theme_{}_{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn builtin_themes_are_complete_and_pass_wcag() {
        for appearance in [Appearance::Dark, Appearance::Light] {
            let theme = builtin_theme(appearance).unwrap();
            let errors = theme.validate();
            assert!(errors.is_empty(), "{appearance:?}: {errors:#?}");
            assert_eq!(theme.plot_palette.len(), PLOT_PALETTE_LEN);
            for name in STATUS_NAMES {
                assert!(theme.status.contains_key(name), "missing status {name}");
            }
            for key in REQUIRED_COLORS {
                assert!(theme.color(key).is_some(), "missing {key}");
            }
        }
    }

    #[test]
    fn extends_merges_child_over_parent() {
        let dir = temp_dir("extends");
        std::fs::write(
            dir.join("ocean.toml"),
            r##"
name = "Ocean"
extends = "dark"
[colors]
accent-primary = "#00e5ff"
[status.info]
bg = "rgba(0, 229, 255, 0.14)"
fg = "#8ef6ff"
"##,
        )
        .unwrap();
        let loader = ThemeLoader::new(&dir);
        let theme = loader.load("ocean").unwrap();
        assert_eq!(theme.name, "Ocean");
        assert_eq!(theme.appearance, Appearance::Dark);
        assert_eq!(theme.color("accent-primary"), Some("#00e5ff"));
        assert_eq!(
            theme.color("bg-panel"),
            builtin_theme(Appearance::Dark).unwrap().color("bg-panel")
        );
        assert_eq!(theme.status["info"].fg, "#8ef6ff");
        assert_eq!(
            theme.status["warn"].fg,
            builtin_theme(Appearance::Dark).unwrap().status["warn"].fg
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cycles_and_unknown_themes_are_errors() {
        let dir = temp_dir("cycles");
        std::fs::write(dir.join("a.toml"), "extends = \"b\"\n").unwrap();
        std::fs::write(dir.join("b.toml"), "extends = \"a\"\n").unwrap();
        let loader = ThemeLoader::new(&dir);
        let error = loader.load("a").unwrap_err().to_string();
        assert!(error.contains("cycle"), "{error}");
        assert!(loader.load("missing").is_err());
        assert!(loader.load("../escape").is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn list_includes_builtins_and_user_themes_and_reports_parse_errors() {
        let dir = temp_dir("list");
        std::fs::write(dir.join("mine.toml"), "name = \"Mine\"\n").unwrap();
        std::fs::write(dir.join("broken.toml"), "not = [valid\n").unwrap();
        let loader = ThemeLoader::new(&dir);
        let list = loader.list().unwrap();
        let ids: Vec<&str> = list.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["dark", "light", "broken", "mine"]);
        assert!(list[0].builtin && list[1].builtin);
        assert!(list[0].preview.is_some());
        assert!(list[2].parse_error.is_some());
        assert_eq!(list[3].name, "Mine");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn resolved_theme_flattens_to_a_standalone_document() {
        let theme = builtin_theme(Appearance::Dark).unwrap();
        let doc = theme.to_document("Copy of Dark");
        assert_eq!(doc.name.as_deref(), Some("Copy of Dark"));
        assert!(doc.extends.is_none());
        assert_eq!(doc.appearance, Some(Appearance::Dark));
        assert_eq!(doc.colors, theme.colors);
        assert_eq!(doc.plot_palette.len(), PLOT_PALETTE_LEN);
        assert_eq!(theme_slug("My Ocean Theme!"), "my-ocean-theme");
        assert_eq!(theme_slug("!!!"), "theme");
    }

    #[test]
    fn color_parser_handles_hex_and_functional_notation() {
        assert_eq!(
            parse_color("#00e5ff"),
            Some(Rgba {
                r: 0.0,
                g: 229.0,
                b: 255.0,
                a: 1.0
            })
        );
        assert_eq!(parse_color("#abc").unwrap().rgb8(), [170, 187, 204]);
        let rgba = parse_color("rgba(78, 161, 255, 0.14)").unwrap();
        assert_eq!(rgba.rgb8(), [78, 161, 255]);
        assert!((rgba.a - 0.14).abs() < 1e-12);
        assert!(parse_color("not-a-color").is_none());
    }

    #[test]
    fn compositing_a_translucent_status_background_keeps_text_readable() {
        let on_dark = contrast_ratio_on("#7cc0ff", "rgba(78, 161, 255, 0.14)", "#151921").unwrap();
        assert!(on_dark >= MIN_CONTRAST, "{on_dark}");
        assert!(contrast_ratio([0, 0, 0], [255, 255, 255]) >= 21.0);
    }
}
