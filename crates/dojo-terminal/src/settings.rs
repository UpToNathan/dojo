//! The `[terminal]` table of `~/.config/dojo/config.toml`, with Ghostty's key names:
//!
//! ```toml
//! [terminal]
//! font-family = "Victor Mono"
//! font-size = 14
//! font-feature = ["ss01", "-calt"]
//! cursor-style = "bar"   # block, bar, underline or block_hollow; programs may choose another
//! ```
//!
//! Left out, the font is Victor Mono with `ss01`, or the system monospace font when Victor Mono
//! is not installed, and the cursor is a bar.
use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};
use dojo_rex::CursorStyle;
use rune::{Family, TextStyle};

pub const DEFAULT_FAMILY: &str = "Victor Mono";
pub const DEFAULT_FEATURES: &[&str] = &["ss01"];
pub const DEFAULT_SIZE: f32 = 14.0;

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub font: Font,
    pub cursor: CursorStyle,
}

impl Default for Settings {
    fn default() -> Self {
        Self { font: Font::default(), cursor: CursorStyle::Bar }
    }
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
/// Rune's proportional fallback.
fn installed(mut font: Font, known: impl Fn(&str) -> bool) -> Font {
    if let Some(family) = &font.family {
        if !known(family) {
            eprintln!("dojo-terminal: font {family:?} is not installed; using the system monospace font");
            font.family = None;
        }
    }
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
        assert_eq!(font.family.as_deref(), Some("Victor Mono"));
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
    fn bad_settings_are_refused() {
        assert!(parse("[terminal]\nfont-size = 1\n").is_err());
        assert!(parse("[terminal]\nfont-feature = [\"stylistic\"]\n").is_err());
        assert!(parse("[terminal]\ncolour = \"red\"\n").is_err());
        assert!(parse("[terminal]\ncursor-style = \"beam\"\n").is_err());
    }

    #[test]
    fn a_missing_family_falls_back_to_system_monospace() {
        assert_eq!(installed(Font::default(), |_| false).family, None);
        assert_eq!(installed(Font::default(), |_| true).family.as_deref(), Some("Victor Mono"));
    }
}
