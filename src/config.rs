use std::fs;
use std::path::PathBuf;

use anyhow::{Context as _, bail};
use serde::{Deserialize, Deserializer};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color(u32);

impl Color {
    pub fn rgb(self) -> gpui::Rgba {
        gpui::rgb(self.0)
    }

    pub fn hex(self) -> String {
        format!("#{:06x}", self.0)
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        let digits = value.strip_prefix('#').filter(|digits| {
            digits.len() == 6 && digits.bytes().all(|byte| byte.is_ascii_hexdigit())
        });
        let digits = digits.ok_or_else(|| {
            serde::de::Error::custom(format!("expected a color in #RRGGBB format, got {value:?}"))
        })?;
        u32::from_str_radix(digits, 16)
            .map(Self)
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThemeColors {
    pub primary: Color,
    pub background: Color,
    pub foreground: Color,
    pub background_highlight: Color,
    pub border: Color,
    pub foreground_muted: Color,
    pub primary_foreground: Color,
    pub selection_foreground: Color,
    pub surface: Color,
    pub surface_raised: Color,
}

impl Default for ThemeColors {
    fn default() -> Self {
        Self {
            primary: Color(0xcba6f7),
            background: Color(0x1e1e2e),
            foreground: Color(0xcdd6f4),
            background_highlight: Color(0x45475a),
            border: Color(0x585b70),
            foreground_muted: Color(0x6c7086),
            primary_foreground: Color(0x000000),
            selection_foreground: Color(0xcdd6f4),
            surface: Color(0x1e1e2e),
            surface_raised: Color(0x45475a),
        }
    }
}

impl<'de> Deserialize<'de> for ThemeColors {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Default, Deserialize)]
        #[serde(default, deny_unknown_fields)]
        struct Input {
            primary: Option<Color>,
            background: Option<Color>,
            foreground: Option<Color>,
            background_highlight: Option<Color>,
            border: Option<Color>,
            foreground_muted: Option<Color>,
            primary_foreground: Option<Color>,
            selection_foreground: Option<Color>,
            surface: Option<Color>,
            surface_raised: Option<Color>,
        }
        let input = Input::deserialize(deserializer)?;
        let defaults = ThemeColors::default();
        let primary = input.primary.unwrap_or(defaults.primary);
        let background = input.background.unwrap_or(defaults.background);
        let foreground = input.foreground.unwrap_or(defaults.foreground);
        let highlight = input
            .background_highlight
            .unwrap_or(defaults.background_highlight);
        let readable = |fill: Color| {
            let fill =
                dojo::extraction::Rgb((fill.0 >> 16) as u8, (fill.0 >> 8) as u8, fill.0 as u8);
            let foreground_rgb = dojo::extraction::Rgb(
                (foreground.0 >> 16) as u8,
                (foreground.0 >> 8) as u8,
                foreground.0 as u8,
            );
            if foreground_rgb.contrast(fill) >= 4.5 {
                return foreground;
            }
            let black = dojo::extraction::Rgb(0, 0, 0);
            let white = dojo::extraction::Rgb(255, 255, 255);
            if black.contrast(fill) >= white.contrast(fill) {
                Color(0)
            } else {
                Color(0xffffff)
            }
        };
        Ok(Self {
            primary,
            background,
            foreground,
            background_highlight: highlight,
            border: input.border.unwrap_or(defaults.border),
            foreground_muted: input.foreground_muted.unwrap_or(defaults.foreground_muted),
            primary_foreground: input
                .primary_foreground
                .unwrap_or_else(|| readable(primary)),
            selection_foreground: input
                .selection_foreground
                .unwrap_or_else(|| readable(highlight)),
            surface: input.surface.unwrap_or(background),
            surface_raised: input.surface_raised.unwrap_or(highlight),
        })
    }
}

impl gpui::Global for ThemeColors {}

impl ThemeColors {
    pub fn semantic_palette(
        self,
        appearance: dojo::theme::Appearance,
    ) -> anyhow::Result<dojo::theme::SemanticPalette> {
        use dojo::extraction::{Palette, Rgb};
        let color = |value: Color| Rgb::parse(&value.hex());
        let mut terminal = [color(self.background)?; 16];
        terminal[4] = color(self.primary)?;
        terminal[7] = color(self.foreground)?;
        let mut palette =
            dojo::theme::SemanticPalette::derive(&Palette { colors: terminal }, appearance);
        for (role, color) in [
            ("background", self.background),
            ("foreground", self.foreground),
            ("primary", self.primary),
            ("background_highlight", self.background_highlight),
            ("border", self.border),
            ("foreground_muted", self.foreground_muted),
            ("primary_foreground", self.primary_foreground),
            ("selection_foreground", self.selection_foreground),
            ("surface", self.surface),
            ("surface_raised", self.surface_raised),
        ] {
            palette.colors.insert(role.into(), color.hex());
        }
        palette.validate()?;
        Ok(palette)
    }
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Theme {
    colors: ThemeColors,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Config {
    theme: Theme,
}

#[derive(Debug, PartialEq, Eq)]
enum Source {
    Missing,
    Contents(String),
}

pub struct ConfigStore {
    path: PathBuf,
    last_source: Option<Source>,
    runtime_path: Option<PathBuf>,
    last_runtime: Option<Source>,
}

impl ConfigStore {
    pub fn watch_paths(&self) -> Vec<PathBuf> {
        std::iter::once(self.path.clone())
            .chain(self.runtime_path.iter().cloned())
            .collect()
    }

    pub fn user_default() -> anyhow::Result<Self> {
        let config_home = match std::env::var_os("XDG_CONFIG_HOME") {
            Some(path) if !path.is_empty() => PathBuf::from(path),
            _ => std::env::var_os("HOME")
                .map(PathBuf::from)
                .context("neither XDG_CONFIG_HOME nor HOME is set")?
                .join(".config"),
        };
        if !config_home.is_absolute() {
            bail!("XDG_CONFIG_HOME must be absolute");
        }
        Ok(Self {
            path: config_home.join("dojo/config.toml"),
            last_source: None,
            runtime_path: Some(
                dojo::theme::Paths::user_default()?
                    .root
                    .join("current/dojo.json"),
            ),
            last_runtime: None,
        })
    }

    pub fn load(&mut self) -> anyhow::Result<Option<ThemeColors>> {
        let source = match fs::read_to_string(&self.path) {
            Ok(contents) => Source::Contents(contents),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Source::Missing,
            Err(error) => {
                return Err(error).with_context(|| format!("reading {}", self.path.display()));
            }
        };
        let runtime = match self.runtime_path.as_ref().map(fs::read_to_string) {
            Some(Ok(contents)) => Source::Contents(contents),
            Some(Err(error)) if error.kind() != std::io::ErrorKind::NotFound => {
                return Err(error).context("reading selected runtime theme");
            }
            _ => Source::Missing,
        };
        if self.last_source.as_ref() == Some(&source)
            && self.last_runtime.as_ref() == Some(&runtime)
        {
            return Ok(None);
        }
        self.last_source = Some(source);
        self.last_runtime = Some(runtime);
        let fallback = match self.last_source.as_ref() {
            Some(Source::Contents(contents)) => {
                let config: Config = toml::from_str(contents)
                    .with_context(|| format!("invalid Dojo config at {}", self.path.display()))?;
                config.theme.colors
            }
            _ => ThemeColors::default(),
        };
        if let Some(Source::Contents(contents)) = self.last_runtime.as_ref() {
            let value: serde_json::Value =
                serde_json::from_str(contents).context("invalid selected Dojo runtime theme")?;
            for role in [
                "primary",
                "background",
                "foreground",
                "background_highlight",
                "border",
                "foreground_muted",
            ] {
                anyhow::ensure!(
                    value.get(role).is_some(),
                    "selected runtime theme is missing {role}"
                );
            }
            Ok(Some(
                serde_json::from_value(value).context("invalid selected Dojo runtime theme")?,
            ))
        } else {
            Ok(Some(fallback))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_exports_preserve_readable_foreground_pairs() -> anyhow::Result<()> {
        use dojo::extraction::Rgb;
        use dojo::theme::{Theme, presets};
        for preset in presets()? {
            let theme = Theme::from_palette(preset.palette.clone())?;
            let outputs = theme.outputs(preset.appearance)?;
            let colors: ThemeColors = serde_json::from_slice(&outputs["dojo.json"])?;
            for (role, actual, fill) in [
                (
                    "selection_foreground",
                    colors.selection_foreground,
                    colors.background_highlight,
                ),
                (
                    "primary_foreground",
                    colors.primary_foreground,
                    colors.primary,
                ),
            ] {
                assert_eq!(
                    actual.hex(),
                    preset.palette.color(role),
                    "{} {role}",
                    preset.id
                );
                assert!(
                    preset.palette.authored
                        || Rgb::parse(&actual.hex())?.contrast(Rgb::parse(&fill.hex())?) >= 4.5,
                    "{} {role}",
                    preset.id
                );
            }
            assert_eq!(colors.surface.hex(), preset.palette.color("surface"));
            assert_eq!(
                colors.surface_raised.hex(),
                preset.palette.color("surface_raised")
            );
        }
        Ok(())
    }

    #[test]
    fn legacy_nord_colors_get_readable_selection_text() -> anyhow::Result<()> {
        let colors: ThemeColors = serde_json::from_str(
            r##"{"background":"#2e3440","foreground":"#d8dee9","background_highlight":"#eceff4"}"##,
        )?;
        assert_eq!(colors.selection_foreground, Color(0));
        assert_eq!(colors.surface, colors.background);
        assert_eq!(colors.surface_raised, colors.background_highlight);
        let authored: ThemeColors = serde_json::from_str(
            r##"{"selection_foreground":"#2e3440","primary_foreground":"#3b4252"}"##,
        )?;
        assert_eq!(authored.selection_foreground, Color(0x2e3440));
        assert_eq!(authored.primary_foreground, Color(0x3b4252));
        Ok(())
    }

    #[test]
    fn empty_config_and_partial_colors_use_defaults() -> anyhow::Result<()> {
        let empty: Config = toml::from_str("")?;
        assert_eq!(empty.theme.colors, ThemeColors::default());
        let partial: Config =
            toml::from_str("[theme.colors]\nbackground = '#123456'\nforeground = '#ABCDEF'")?;
        assert_eq!(partial.theme.colors.background, Color(0x123456));
        assert_eq!(partial.theme.colors.foreground, Color(0xabcdef));
        assert_eq!(partial.theme.colors.border, ThemeColors::default().border);
        let primary: Config = toml::from_str("[theme.colors]\nprimary = '#123456'")?;
        assert_eq!(primary.theme.colors.primary, Color(0x123456));
        Ok(())
    }

    #[test]
    fn config_rejects_unknown_fields_and_malformed_colors() {
        for invalid in [
            "unrecognized = true",
            "[theme]\nunrecognized = true",
            "[theme.colors]\nbackgroun = '#123456'",
            "[theme.colors]\nbackground = '123456'",
            "[theme.colors]\nbackground = '#fff'",
            "[theme.colors]\nbackground = '#gggggg'",
            "[theme.colors]\nbackground = 123456",
        ] {
            assert!(
                toml::from_str::<Config>(invalid).is_err(),
                "accepted {invalid}"
            );
        }
    }

    #[test]
    fn reload_retains_valid_colors_until_the_entire_edit_is_valid() -> anyhow::Result<()> {
        let directory = std::env::temp_dir().join(format!(
            "dojo-config-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        fs::create_dir(&directory)?;
        let path = directory.join("config.toml");
        let mut store = ConfigStore {
            path: path.clone(),
            last_source: None,
            runtime_path: None,
            last_runtime: None,
        };
        let mut active = store
            .load()?
            .context("missing config should load defaults")?;
        assert_eq!(active, ThemeColors::default());
        assert!(store.load()?.is_none());

        fs::write(&path, "[theme.colors]\nbackground = '#123456'")?;
        active = store.load()?.context("valid edit should load")?;
        assert_eq!(active.background, Color(0x123456));

        fs::write(
            &path,
            "[theme.colors]\nbackground = '#abcdef'\nborder = 'invalid'",
        )?;
        assert!(store.load().is_err());
        assert_eq!(active.background, Color(0x123456));
        assert!(store.load()?.is_none());

        fs::write(&path, "[theme.colors]\nbackground = '#abcdef'")?;
        active = store.load()?.context("corrected edit should load")?;
        assert_eq!(active.background, Color(0xabcdef));
        fs::remove_file(path)?;
        assert_eq!(store.load()?, Some(ThemeColors::default()));
        fs::remove_dir(directory)?;
        Ok(())
    }

    #[test]
    fn runtime_palette_is_separate_and_invalid_updates_keep_the_last_valid_value()
    -> anyhow::Result<()> {
        let directory = std::env::temp_dir().join(format!(
            "dojo-runtime-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        fs::create_dir(&directory)?;
        let path = directory.join("config.toml");
        let runtime = directory.join("dojo.json");
        fs::write(&path, "[theme.colors]\nbackground = '#123456'")?;
        let mut store = ConfigStore {
            path: path.clone(),
            last_source: None,
            runtime_path: Some(runtime.clone()),
            last_runtime: None,
        };
        assert_eq!(store.load()?.map(|c| c.background), Some(Color(0x123456)));
        fs::write(
            &runtime,
            r##"{"primary":"#cba6f7","background":"#abcdef","foreground":"#cdd6f4","background_highlight":"#45475a","border":"#585b70","foreground_muted":"#6c7086"}"##,
        )?;
        let active = store.load()?.context("runtime palette should load")?;
        assert_eq!(active.background, Color(0xabcdef));
        fs::write(&runtime, r##"{"background":"invalid"}"##)?;
        assert!(store.load().is_err());
        assert_eq!(active.background, Color(0xabcdef));
        fs::remove_file(&runtime)?;
        assert_eq!(store.load()?.map(|c| c.background), Some(Color(0x123456)));
        assert_eq!(
            fs::read_to_string(&path)?,
            "[theme.colors]\nbackground = '#123456'"
        );
        fs::remove_file(path)?;
        fs::remove_dir(directory)?;
        Ok(())
    }
}
