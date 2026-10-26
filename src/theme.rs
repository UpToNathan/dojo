//! Shared theme derivation, export and activation; user configuration stays owned by the user.

mod apply;
mod export;
pub mod lock_wallpaper;
mod presets;
mod preview;
mod refined;
mod store;
pub mod wallpaper_layout;

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::extraction::{Mode, Palette, Rgb};
use crate::procedural::Recipe;
/// Fits an RGBA source within the existing 256 MiB image allocation budget.
pub const MAX_WALLPAPER_PIXELS: u64 = 64_000_000;
pub use apply::{
    ApplyReport, LockWallpaperReport, TARGETS, TargetResult, apply, apply_procedural,
    apply_wallpaper,
    refresh_lock_wallpaper, refresh_system, refresh_target, set_system_appearance, system_appearance,
    watch,
};
pub use presets::{Preset, apply_preset, presets};
pub use preview::preview;
pub use refined::{RefinedTheme, derive_wallpaper, derive_wallpaper_mode, derive_wallpaper_modes};
pub use store::{Paths, Selection, ThemeStore};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    Dark,
    Light,
}

impl Appearance {
    pub fn name(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }
    pub fn parse(value: &str) -> anyhow::Result<Self> {
        match value {
            "dark" => Ok(Self::Dark),
            "light" => Ok(Self::Light),
            _ => anyhow::bail!("expected light or dark, got {value:?}"),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticPalette {
    pub appearance: Appearance,
    #[serde(default)]
    pub authored: bool,
    pub colors: BTreeMap<String, String>,
    pub terminal: [String; 16],
}

impl SemanticPalette {
    pub fn derive(palette: &Palette, appearance: Appearance) -> Self {
        let background = palette.colors[0];
        let foreground = readable(palette.colors[7], background, 7.);
        let surface = blend(background, foreground, 0.04);
        let raised = blend(background, foreground, 0.08);
        let selection = blend(background, palette.colors[4], 0.24);
        let primary = readable(palette.colors[4], background, 4.5);
        let secondary = readable(palette.colors[5], background, 4.5);
        let error = readable(palette.colors[1], background, 4.5);
        let success = readable(palette.colors[2], background, 4.5);
        let warning = readable(palette.colors[3], background, 4.5);
        let info = readable(palette.colors[6], background, 4.5);
        let mut colors = BTreeMap::new();
        for (name, value) in [
            ("background", background),
            ("foreground", readable(foreground, raised, 7.)),
            ("surface", surface),
            ("surface_raised", raised),
            ("background_highlight", selection),
            ("border", blend(background, foreground, 0.25)),
            (
                "foreground_muted",
                readable(blend(background, foreground, 0.55), raised, 4.5),
            ),
            ("primary", primary),
            ("secondary", secondary),
            ("primary_foreground", readable(foreground, primary, 4.5)),
            ("selection_foreground", readable(foreground, selection, 4.5)),
            ("error", error),
            ("success", success),
            ("warning", warning),
            ("info", info),
            ("diff_added_background", blend(background, success, 0.12)),
            ("diff_removed_background", blend(background, error, 0.12)),
        ] {
            colors.insert(name.into(), value.hex());
        }
        Self {
            appearance,
            authored: false,
            colors,
            terminal: palette.hex_colors(),
        }
    }

    pub fn color(&self, role: &str) -> &str {
        self.colors
            .get(role)
            .map(String::as_str)
            .unwrap_or("#ff00ff")
    }

    pub fn color_or<'a>(&'a self, role: &str, fallback: &str) -> &'a str {
        self.colors
            .get(role)
            .map(String::as_str)
            .unwrap_or_else(|| self.color(fallback))
    }

    pub fn syntax(&self, token: &str) -> &str {
        let fallback = match token {
            "comment" | "punctuation" => "foreground_muted",
            "keyword" => "secondary",
            "function" => "primary",
            "variable" => "foreground",
            "string" => "success",
            "number" => "warning",
            _ => "info",
        };
        self.color_or(&format!("syntax_{token}"), fallback)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        for required in [
            "background",
            "foreground",
            "surface",
            "surface_raised",
            "background_highlight",
            "border",
            "foreground_muted",
            "primary",
            "secondary",
            "primary_foreground",
            "selection_foreground",
            "error",
            "success",
            "warning",
            "info",
            "diff_added_background",
            "diff_removed_background",
        ] {
            anyhow::ensure!(
                self.colors.contains_key(required),
                "missing semantic role {required}"
            );
        }
        for color in self.colors.values().chain(self.terminal.iter()) {
            anyhow::ensure!(
                color.starts_with('#') && color.len() == 7,
                "theme colors must use #RRGGBB"
            );
            Rgb::parse(color)?;
        }
        let foreground = Rgb::parse(self.color("foreground"))?;
        for role in ["background", "surface", "surface_raised"] {
            anyhow::ensure!(
                // Preserve the author's text/surface choices rather than silently
                // recoloring static themes to satisfy the derivation policy.
                foreground.contrast(Rgb::parse(self.color(role))?)
                    >= if self.authored { 3. } else { 4.5 },
                "foreground fails contrast against {role}"
            );
        }
        Ok(())
    }
}

fn blend(a: Rgb, b: Rgb, amount: f64) -> Rgb {
    let channel = |a: u8, b: u8| ((1. - amount) * a as f64 + amount * b as f64).round() as u8;
    Rgb(channel(a.0, b.0), channel(a.1, b.1), channel(a.2, b.2))
}

fn readable(candidate: Rgb, background: Rgb, ratio: f64) -> Rgb {
    if candidate.contrast(background) >= ratio {
        return candidate;
    }
    let black = Rgb(0, 0, 0);
    let white = Rgb(255, 255, 255);
    let end = if black.contrast(background) > white.contrast(background) {
        black
    } else {
        white
    };
    for step in 1..=100 {
        let adjusted = blend(candidate, end, step as f64 / 100.);
        if adjusted.contrast(background) >= ratio {
            return adjusted;
        }
    }
    end
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Theme {
    pub version: u32,
    pub wallpaper: String,
    pub wallpaper_sha256: String,
    #[serde(skip)]
    image_bytes: Vec<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<Mode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    /// How to draw the wallpaper at any size, when it was generated rather than chosen; the
    /// retained `wallpaper` is then only a reference picture of the same recipe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipe: Option<Recipe>,
    pub algorithm_revision: String,
    pub dark: SemanticPalette,
    pub light: SemanticPalette,
}

impl Theme {
    pub fn from_palette(palette: SemanticPalette) -> anyhow::Result<Self> {
        palette.validate()?;
        Ok(Self {
            version: 1,
            wallpaper: String::new(),
            wallpaper_sha256: String::new(),
            image_bytes: Vec::new(),
            mode: None,
            preset: None,
            recipe: None,
            algorithm_revision: "config-fallback-1".into(),
            dark: palette.clone(),
            light: palette,
        })
    }

    pub fn with_wallpaper_snapshot(mut self, path: &Path, bytes: Vec<u8>) -> anyhow::Result<Self> {
        anyhow::ensure!(path.is_absolute(), "wallpaper path must be absolute");
        anyhow::ensure!(bytes.len() <= 50 * 1024 * 1024, "wallpaper exceeds 50 MiB");
        let mut reader =
            image::ImageReader::new(std::io::Cursor::new(&bytes)).with_guessed_format()?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(16_384);
        limits.max_image_height = Some(16_384);
        limits.max_alloc = Some(256 * 1024 * 1024);
        reader.limits(limits);
        let image = reader.decode()?;
        anyhow::ensure!(
            u64::from(image.width()) * u64::from(image.height()) <= MAX_WALLPAPER_PIXELS,
            "wallpaper exceeds the 64 million pixel limit"
        );
        self.wallpaper = path.to_string_lossy().into_owned();
        self.wallpaper_sha256 = format!("{:x}", Sha256::digest(&bytes));
        self.image_bytes = bytes;
        // A chosen image replaces whatever was generated.
        self.recipe = None;
        Ok(self)
    }

    /// The wallpaper drawn from `recipe`, with `reference` (a PNG of the whole canvas) retained as
    /// the picture palette derivation, the lock screen and the login screen read.
    pub fn with_recipe(self, recipe: Recipe, reference: Vec<u8>) -> anyhow::Result<Self> {
        let path = Path::new("/dojo/procedural").join(format!(
            "{}-{}.png",
            recipe.style.name(),
            recipe.seed
        ));
        let mut theme = self.with_wallpaper_snapshot(&path, reference)?;
        theme.recipe = Some(recipe);
        Ok(theme)
    }
    pub fn generate(path: &Path, mode: Mode) -> anyhow::Result<Self> {
        let path = path.canonicalize()?;
        let mut file = std::fs::File::open(&path)?;
        use std::io::Read as _;
        let mut image_bytes = Vec::new();
        std::io::Read::by_ref(&mut file)
            .take(50 * 1024 * 1024 + 1)
            .read_to_end(&mut image_bytes)?;
        Self::from_snapshot(&path, image_bytes, mode)
    }

    pub fn from_snapshot(path: &Path, image_bytes: Vec<u8>, mode: Mode) -> anyhow::Result<Self> {
        anyhow::ensure!(
            path.is_absolute(),
            "wallpaper snapshot needs an absolute source path"
        );
        anyhow::ensure!(
            image_bytes.len() <= 50 * 1024 * 1024,
            "wallpaper exceeds the 50 MiB input limit"
        );
        let wallpaper_sha256 = format!("{:x}", Sha256::digest(&image_bytes));
        let refined = derive_wallpaper_mode(&image_bytes, mode)?;
        let dark = refined.dark;
        let light = refined.light;
        dark.validate()?;
        light.validate()?;
        Ok(Self {
            version: 1,
            wallpaper: path.to_string_lossy().into_owned(),
            wallpaper_sha256,
            image_bytes,
            mode: Some(mode),
            preset: None,
            recipe: None,
            algorithm_revision: refined.revision.into(),
            dark,
            light,
        })
    }
    pub fn palette(&self, appearance: Appearance) -> &SemanticPalette {
        match appearance {
            Appearance::Dark => &self.dark,
            Appearance::Light => &self.light,
        }
    }
    pub fn outputs(&self, appearance: Appearance) -> anyhow::Result<BTreeMap<String, Vec<u8>>> {
        export::render(self, appearance)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn semantic_derivation_has_readable_surfaces_and_exportable_roles() -> anyhow::Result<()> {
        let terminal = [
            Rgb(12, 15, 23),
            Rgb(210, 80, 90),
            Rgb(90, 200, 130),
            Rgb(230, 200, 90),
            Rgb(100, 130, 220),
            Rgb(190, 100, 220),
            Rgb(80, 190, 210),
            Rgb(240, 242, 250),
            Rgb(100, 110, 130),
            Rgb(230, 110, 120),
            Rgb(130, 220, 160),
            Rgb(250, 220, 130),
            Rgb(150, 170, 240),
            Rgb(220, 150, 240),
            Rgb(140, 220, 230),
            Rgb(255, 255, 255),
        ];
        for appearance in [Appearance::Dark, Appearance::Light] {
            let colors = SemanticPalette::derive(&Palette { colors: terminal }, appearance);
            colors.validate()?;
        }
        Ok(())
    }
}
