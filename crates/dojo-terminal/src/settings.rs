//! The `[terminal]` table of `~/.config/dojo/config.toml`, with Ghostty's key names:
//!
//! ```toml
//! [terminal]
//! font-family = "Victor Mono"
//! font-size = 14
//! font-feature = ["ss01", "-calt"]
//! cursor-style = "bar"   # block, bar, underline or block_hollow; programs may choose another
//! theme = "/path/to/colours.conf"   # Ghostty colour keys; "" keeps the session's own colours
//! ```
//!
//! Left out, the font is Victor Mono's Nerd Font build with `ss01`, so programs' icons draw;
//! then plain Victor Mono, then the system monospace font, whichever is installed first; the cursor is a bar, and the colours are Dojo's current theme
//! (`$XDG_STATE_HOME/dojo/themes/current/ghostty.conf`, written by the theme switcher).
use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};
use dojo_rex::CursorStyle;
use rune::{Family, TextStyle};

pub const DEFAULT_FAMILY: &str = "VictorMono Nerd Font Mono";
/// The default family's fallbacks, before the system monospace font.
const DEFAULT_FALLBACKS: &[&str] = &["Victor Mono"];
pub const DEFAULT_FEATURES: &[&str] = &["ss01"];
pub const DEFAULT_SIZE: f32 = 14.0;

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub font: Font,
    pub cursor: CursorStyle,
    /// A Ghostty colour file applied to the session on open; None leaves its colours alone.
    pub theme: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self { font: Font::default(), cursor: CursorStyle::Bar, theme: dojo_theme() }
    }
}

/// The Ghostty colours Dojo's theme switcher writes for the current theme.
fn dojo_theme() -> Option<PathBuf> {
    let state = match std::env::var_os("XDG_STATE_HOME") {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        _ => PathBuf::from(std::env::var_os("HOME")?).join(".local/state"),
    };
    Some(state.join("dojo/themes/current/ghostty.conf"))
}

/// Ghostty's colour keys as the payload of Rex's `set_theme`. Other keys are ignored, so a
/// whole Ghostty config works too.
pub fn theme_payload(source: &str) -> serde_json::Value {
    let mut theme = serde_json::Map::new();
    let mut palette = Vec::new();
    for line in source.lines() {
        let Some((key, value)) = line.split_once('=') else { continue };
        let (key, value) = (key.trim(), value.trim());
        let hex = |value: &str| {
            let hex = value.trim_start_matches('#');
            (hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit())).then(|| hex.to_lowercase())
        };
        match key {
            "background" | "foreground" => {
                if let Some(hex) = hex(value) {
                    theme.insert(key.into(), hex.into());
                }
            }
            "cursor-color" => {
                if let Some(hex) = hex(value) {
                    theme.insert("cursor".into(), hex.into());
                }
            }
            "palette" => {
                if let Some((index, color)) = value.split_once('=') {
                    if let (Ok(index), Some(hex)) = (index.trim().parse::<u8>(), hex(color.trim())) {
                        palette.push(format!("{index}={hex}"));
                    }
                }
            }
            _ => {}
        }
    }
    if !palette.is_empty() {
        theme.insert("palette".into(), palette.into());
    }
    theme.into()
}

#[derive(Debug, Clone, PartialEq)]
pub struct Font {
    /// None is the system monospace font.
    pub family: Option<String>,
    pub size: f32,
    /// CSS `font-feature-settings`, such as `'ss01' 1, 'calt' 0`.
    pub features: String,
}

impl Default for Font {
    fn default() -> Self {
        Self { family: Some(DEFAULT_FAMILY.into()), size: DEFAULT_SIZE, features: css_features(DEFAULT_FEATURES).unwrap() }
    }
}

impl Font {
    pub fn line_height(&self) -> f32 {
        (self.size * 1.2).ceil()
    }

    /// Rune keeps families and features as `&'static str`; a font is built once per process.
    pub fn style(&self) -> TextStyle {
        let family = match &self.family {
            Some(name) => Family::Named(Box::leak(name.clone().into_boxed_str())),
            None => Family::SystemMono,
        };
        TextStyle {
            family,
            line_height: self.line_height(),
            features: Box::leak(self.features.clone().into_boxed_str()),
            ..TextStyle::new(self.size, 400.0)
        }
    }
}

/// Reads the settings from the config file; a missing file or table means the defaults.
pub fn load() -> Result<Settings> {
    let path = config_path()?;
    let mut settings = match std::fs::read_to_string(&path) {
        Ok(source) => parse(&source).with_context(|| format!("reading {}", path.display()))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Settings::default(),
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    settings.font = installed(settings.font, family_installed);
    Ok(settings)
}

fn config_path() -> Result<PathBuf> {
    let home = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        _ => PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?).join(".config"),
    };
    Ok(home.join("dojo/config.toml"))
}

pub fn parse(source: &str) -> Result<Settings> {
    let config: toml::Table = source.parse()?;
    let mut settings = Settings::default();
    let Some(terminal) = config.get("terminal") else { return Ok(settings) };
    let font = &mut settings.font;
    let terminal = terminal.as_table().context("[terminal] must be a table")?;
    for (key, value) in terminal {
        match key.as_str() {
            "font-family" => {
                let name = value.as_str().context("font-family must be a string")?.trim();
                font.family = (!name.is_empty()).then(|| name.to_owned());
            }
            "font-size" => {
                let size = value.as_float().or_else(|| value.as_integer().map(|v| v as f64)).context("font-size must be a number")?;
                if !(4.0..=200.0).contains(&size) {
                    bail!("font-size must be between 4 and 200");
                }
                font.size = size as f32;
            }
            "font-feature" => {
                let list: Vec<&str> = match value {
                    toml::Value::String(one) => vec![one.as_str()],
                    toml::Value::Array(many) => many
                        .iter()
                        .map(|item| item.as_str().context("font-feature entries must be strings"))
                        .collect::<Result<_>>()?,
                    _ => bail!("font-feature must be a string or a list of strings"),
                };
                font.features = css_features(&list)?;
            }
            "theme" => {
                let path = value.as_str().context("theme must be a string")?.trim();
                settings.theme = (!path.is_empty()).then(|| PathBuf::from(path));
            }
            "cursor-style" => {
                settings.cursor = match value.as_str().context("cursor-style must be a string")? {
                    "block" => CursorStyle::Block,
                    "bar" => CursorStyle::Bar,
                    "underline" => CursorStyle::Underline,
                    "block_hollow" => CursorStyle::HollowBlock,
                    other => bail!("cursor-style {other:?} is not block, bar, underline or block_hollow"),
                };
            }
            other => bail!("unknown [terminal] setting {other:?}"),
        }
    }
    Ok(settings)
}

/// Ghostty's feature syntax (`ss01`, `+liga`, `-calt`, `cv01=2`) as CSS feature settings.
pub fn css_features(list: &[&str]) -> Result<String> {
    let mut out = Vec::new();
    for entry in list {
        let entry = entry.trim();
        let (tag, value) = if let Some(tag) = entry.strip_prefix('-') {
            (tag, 0)
        } else if let Some((tag, value)) = entry.split_once('=') {
            (tag.trim(), value.trim().parse::<u32>().with_context(|| format!("font-feature {entry:?}"))?)
        } else {
            (entry.strip_prefix('+').unwrap_or(entry), 1)
        };
        if tag.len() != 4 || !tag.chars().all(|c| c.is_ascii_alphanumeric()) {
            bail!("font-feature {entry:?} needs a four-character tag such as ss01");
        }
        out.push(format!("'{tag}' {value}"));
    }
    Ok(out.join(", "))
}

/// A family fontconfig does not know falls back to the system monospace font rather than to
/// Rune's proportional fallback; the default family tries its own fallbacks first.
fn installed(mut font: Font, known: impl Fn(&str) -> bool) -> Font {
    let Some(family) = font.family.clone() else { return font };
    if known(&family) {
        return font;
    }
    let fallbacks: &[&str] = if family == DEFAULT_FAMILY { DEFAULT_FALLBACKS } else { &[] };
    font.family = fallbacks.iter().find(|name| known(name)).map(|name| name.to_string());
    eprintln!(
        "dojo-terminal: font {family:?} is not installed; using {}",
        font.family.as_deref().unwrap_or("the system monospace font")
    );
    font
}

fn family_installed(family: &str) -> bool {
    std::process::Command::new("fc-list")
        .args(["-q", &format!(":family={family}")])
        .status()
        .map(|status| status.success())
        // Without fontconfig's tools, trust the setting.
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_victor_mono_with_ss01() {
        let settings = parse("").unwrap();
        assert_eq!(settings.cursor, CursorStyle::Bar);
        let font = settings.font;
        assert_eq!(font.family.as_deref(), Some("VictorMono Nerd Font Mono"));
        assert_eq!(font.features, "'ss01' 1");
        assert_eq!(font.size, 14.0);
        assert_eq!(parse("[theme]\nx = 1\n").unwrap().font, font);
    }

    #[test]
    fn ghostty_keys_override_it() {
        let font = parse("[terminal]\nfont-family = \"JetBrains Mono\"\nfont-size = 12.5\nfont-feature = [\"-calt\", \"+liga\", \"cv01=2\"]\n").unwrap().font;
        assert_eq!(font.family.as_deref(), Some("JetBrains Mono"));
        assert_eq!(font.size, 12.5);
        assert_eq!(font.features, "'calt' 0, 'liga' 1, 'cv01' 2");
        assert_eq!(parse("[terminal]\nfont-family = \"\"\n").unwrap().font.family, None);
        assert_eq!(parse("[terminal]\nfont-feature = \"ss02\"\n").unwrap().font.features, "'ss02' 1");
        assert_eq!(parse("[terminal]\ncursor-style = \"block\"\n").unwrap().cursor, CursorStyle::Block);
    }

    #[test]
    fn ghostty_colour_keys_become_a_rex_theme() {
        let theme = theme_payload("# Generated\nbackground = #24283B\nforeground = c0caf5\ncursor-color = #7aa2f7\npalette = 1=#f7768e\npalette = 15=#c0caf5\nfont-size = 12\nbackground = nope\n");
        assert_eq!(
            theme,
            serde_json::json!({"background": "24283b", "foreground": "c0caf5", "cursor": "7aa2f7", "palette": ["1=f7768e", "15=c0caf5"]})
        );
        assert_eq!(parse("[terminal]\ntheme = \"\"\n").unwrap().theme, None);
        assert!(parse("").unwrap().theme.unwrap().ends_with("dojo/themes/current/ghostty.conf"));
    }

    #[test]
    fn bad_settings_are_refused() {
        assert!(parse("[terminal]\nfont-size = 1\n").is_err());
        assert!(parse("[terminal]\nfont-feature = [\"stylistic\"]\n").is_err());
        assert!(parse("[terminal]\ncolour = \"red\"\n").is_err());
        assert!(parse("[terminal]\ncursor-style = \"beam\"\n").is_err());
    }

    #[test]
    fn a_missing_family_falls_back_to_system_monospace() {
        assert_eq!(installed(Font::default(), |_| false).family, None);
        assert_eq!(installed(Font::default(), |_| true).family.as_deref(), Some("VictorMono Nerd Font Mono"));
        assert_eq!(installed(Font::default(), |name| name == "Victor Mono").family.as_deref(), Some("Victor Mono"));
        let other = Font { family: Some("Iosevka".into()), ..Font::default() };
        assert_eq!(installed(other, |name| name == "Victor Mono").family, None, "only the default falls back to Victor Mono");
    }
}
