//! Authored preset palettes: no extraction, invented opposite appearance, or schedules.
use std::collections::BTreeSet;

use anyhow::Context as _;

use super::{
    Appearance, ApplyReport, Palette, Rgb, Selection, SemanticPalette, Theme, ThemeStore, readable,
};

const REVISION: &str = "static-palettes-dojo-2";

#[derive(Clone, Debug)]
pub struct Preset {
    pub id: String,
    pub family: String,
    pub name: String,
    pub appearance: Appearance,
    pub palette: SemanticPalette,
}

pub fn presets() -> anyhow::Result<Vec<Preset>> {
    parse(include_str!("presets.txt"))
}

fn parse(data: &str) -> anyhow::Result<Vec<Preset>> {
    let mut result = Vec::new();
    let mut ids = BTreeSet::new();
    for line in data
        .lines()
        .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
    {
        let fields = line.split('|').collect::<Vec<_>>();
        anyhow::ensure!(fields.len() == 7, "invalid preset row");
        let [
            family,
            variant,
            appearance,
            background,
            foreground,
            selection,
            terminal,
        ] = fields.as_slice()
        else {
            unreachable!()
        };
        let color = |value: &str| Rgb::parse(&format!("#{value}"));
        let background = color(background)?;
        let foreground = color(foreground)?;
        let selection = color(selection)?;
        let terminal = terminal
            .split_whitespace()
            .map(color)
            .collect::<anyhow::Result<Vec<_>>>()?;
        let terminal: [Rgb; 16] = terminal
            .try_into()
            .map_err(|_| anyhow::anyhow!("preset needs 16 ANSI colors"))?;
        let appearance = Appearance::parse(appearance)?;
        let name = if family == variant {
            family.to_string()
        } else {
            format!("{family} {variant}")
        };
        let id = name.to_lowercase().replace('é', "e").replace(' ', "-");
        let palette =
            if let Some(palette) = authored_palette(&id, appearance, terminal.map(Rgb::hex))? {
                palette
            } else {
                // ANSI black/white are not necessarily the author's background/text.
                let mut roles = terminal;
                roles[0] = background;
                roles[7] = foreground;
                let mut palette = SemanticPalette::derive(&Palette { colors: roles }, appearance);
                palette.terminal = terminal.map(Rgb::hex);
                palette.colors.insert(
                    "foreground".into(),
                    readable(
                        foreground,
                        Rgb::parse(palette.color("surface_raised"))?,
                        4.5,
                    )
                    .hex(),
                );
                palette
                    .colors
                    .insert("background_highlight".into(), selection.hex());
                palette.colors.insert(
                    "selection_foreground".into(),
                    readable(foreground, selection, 4.5).hex(),
                );
                palette
            };
        palette.validate()?;
        anyhow::ensure!(ids.insert(id.clone()), "duplicate preset {id}");
        result.push(Preset {
            id,
            family: family.to_string(),
            name,
            appearance,
            palette,
        });
    }
    Ok(result)
}

fn authored_palette(
    id: &str,
    appearance: Appearance,
    terminal: [String; 16],
) -> anyhow::Result<Option<SemanticPalette>> {
    let mut lines = include_str!("authored-presets.txt")
        .lines()
        .filter(|line| !line.starts_with('#') && !line.trim().is_empty());
    let roles = lines
        .next()
        .context("authored palette role header missing")?
        .split_whitespace()
        .collect::<Vec<_>>();
    for line in lines {
        let (name, values) = line
            .split_once('|')
            .context("invalid authored palette row")?;
        if name != id {
            continue;
        }
        let values = values.split_whitespace().collect::<Vec<_>>();
        anyhow::ensure!(
            roles.len() == values.len(),
            "authored palette {id} has the wrong role count"
        );
        let colors = roles
            .iter()
            .zip(values)
            .map(|(role, value)| (role.to_string(), format!("#{value}")))
            .collect();
        let mut palette = SemanticPalette {
            appearance,
            authored: true,
            colors,
            terminal,
        };
        if id.starts_with("tokyo-night-") {
            let (gutter, float_border) = match id {
                "tokyo-night-day" => ("#a8aecb", "#4094a3"),
                "tokyo-night-moon" => ("#3b4261", "#589ed7"),
                "tokyo-night-storm" => ("#3b4261", "#29a4bd"),
                _ => ("#3b4261", "#27a1b9"),
            };
            palette.colors.insert("line_number".into(), gutter.into());
            palette
                .colors
                .insert("float_border".into(), float_border.into());
            palette.colors.insert(
                "syntax_statement".into(),
                palette.color("secondary").to_string(),
            );
            palette.colors.insert(
                "syntax_identifier".into(),
                palette.color("secondary").to_string(),
            );
        }
        return Ok(Some(palette));
    }
    Ok(None)
}

impl Theme {
    pub fn from_preset(preset: &Preset, store: &ThemeStore) -> anyhow::Result<Self> {
        preset.palette.validate()?;
        let mut theme = Self {
            version: 1,
            wallpaper: String::new(),
            wallpaper_sha256: String::new(),
            image_bytes: Vec::new(),
            mode: None,
            preset: Some(preset.id.clone()),
            recipe: None,
            algorithm_revision: REVISION.into(),
            // A preset is one authored variant, not a fabricated light/dark pair.
            dark: preset.palette.clone(),
            light: preset.palette.clone(),
        };
        // Preserve only an already-selected wallpaper. With none selected, leave
        // the external wallpaper provider untouched; no fallback image is applied.
        if let Some(selection) = store.selection()? {
            let previous = store.theme(&selection.generation)?;
            if !previous.wallpaper.is_empty() {
                let snapshot = store
                    .paths
                    .root
                    .join("generations")
                    .join(&selection.generation)
                    .join("wallpaper");
                theme.image_bytes =
                    std::fs::read(&snapshot).context("preserving selected wallpaper snapshot")?;
                theme.wallpaper = previous.wallpaper;
                theme.wallpaper_sha256 = previous.wallpaper_sha256;
                theme.recipe = previous.recipe;
            }
        }
        Ok(theme)
    }
}

pub fn apply_preset(
    store: &ThemeStore,
    preset: &Preset,
    targets: Vec<String>,
) -> anyhow::Result<ApplyReport> {
    apply_with_mode(store, preset, targets, super::apply::set_system_appearance)
}

fn apply_with_mode(
    store: &ThemeStore,
    preset: &Preset,
    targets: Vec<String>,
    set_mode: impl FnOnce(Appearance) -> anyhow::Result<()>,
) -> anyhow::Result<ApplyReport> {
    for target in &targets {
        anyhow::ensure!(
            super::TARGETS.contains(&target.as_str()),
            "unknown target {target}"
        );
    }
    let theme = Theme::from_preset(preset, store)?;
    let generation = store.generate(&theme)?;
    // Generation/export validation happens before touching the session mode.
    set_mode(preset.appearance)?;
    super::apply(
        store,
        Selection {
            generation,
            appearance: preset.appearance,
            follow_system: false,
            targets,
        },
    )
    .with_context(|| {
        format!(
            "darkman switched to {}; preset publication failed",
            preset.appearance.name()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokyo_night_uses_authored_ui_and_syntax_roles_not_ansi_guesses() -> anyhow::Result<()> {
        let catalog = presets()?;
        let night = catalog
            .iter()
            .find(|preset| preset.id == "tokyo-night-night")
            .context("Night")?;
        assert_eq!(night.palette.color("surface"), "#16161e");
        assert_eq!(night.palette.color("foreground_muted"), "#a9b1d6");
        assert_eq!(night.palette.color("syntax_comment"), "#565f89");
        assert_eq!(night.palette.color("syntax_number"), "#ff9e64");
        assert_eq!(night.palette.color("syntax_type"), "#2ac3de");
        let dir = TestDir::new()?;
        let theme = Theme::from_preset(night, &store(dir.path()))?;
        let outputs = theme.outputs(Appearance::Dark)?;
        let opencode: serde_json::Value = serde_json::from_slice(&outputs["opencode.json"])?;
        assert_eq!(opencode["dark"]["background"]["base"], "#1a1b26");
        assert_eq!(opencode["dark"]["syntax"]["number"], "#ff9e64");
        Ok(())
    }

    struct TestDir(std::path::PathBuf);

    impl TestDir {
        fn new() -> anyhow::Result<Self> {
            let path = std::env::temp_dir().join(format!(
                "dojo-presets-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_nanos()
            ));
            std::fs::create_dir(&path)?;
            Ok(Self(path))
        }
        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn store(root: &std::path::Path) -> ThemeStore {
        ThemeStore::new(super::super::Paths {
            home: root.join("home"),
            config: root.join("config"),
            data: root.join("data"),
            root: root.join("state"),
        })
    }

    #[test]
    fn all_41_presets_validate_and_preserve_authored_ansi_colors() -> anyhow::Result<()> {
        let catalog = presets()?;
        assert_eq!(catalog.len(), 41);
        assert_eq!(
            catalog
                .iter()
                .filter(|p| p.appearance == Appearance::Light)
                .count(),
            13
        );
        let mocha = catalog
            .iter()
            .find(|p| p.id == "catppuccin-mocha")
            .context("Mocha")?;
        assert_eq!(mocha.palette.color("background"), "#1e1e2e");
        assert_eq!(mocha.palette.terminal[4], "#89b4fa");
        for preset in catalog {
            preset.palette.validate()?;
            let dir = TestDir::new()?;
            let theme = Theme::from_preset(&preset, &store(dir.path()))?;
            for appearance in [Appearance::Light, Appearance::Dark] {
                let outputs = theme.outputs(appearance)?;
                assert!(!outputs.is_empty());
                let helium: serde_json::Value = serde_json::from_slice(&outputs["helium.json"])?;
                let crate::extraction::Rgb(r, g, b) =
                    crate::extraction::Rgb::parse(theme.palette(appearance).color("background"))?;
                assert_eq!(
                    helium["theme"]["colors"]["ntp_background"],
                    serde_json::json!([r, g, b])
                );
                assert_eq!(
                    helium["theme"]["colors"]["toolbar"],
                    serde_json::json!([r, g, b])
                );
                let niri = std::str::from_utf8(&outputs["niri.kdl"])?;
                assert!(niri.contains(&format!(
                    "active-color \"{}33\"",
                    theme.palette(appearance).color("primary")
                )));
                assert!(niri.contains(&format!(
                    "inactive-color \"{}33\"",
                    theme.palette(appearance).color("border")
                )));
                for (name, bytes) in outputs {
                    if name.ends_with(".json") {
                        serde_json::from_slice::<serde_json::Value>(&bytes)?;
                    }
                }
            }
        }
        Ok(())
    }

    #[test]
    fn malformed_preset_data_is_rejected() {
        assert!(parse("Broken").is_err());
        assert!(parse("Family|Variant|light|invalid|ffffff|ffffff|ffffff").is_err());
        assert!(parse("Family|Variant|light|000000|ffffff|ffffff|ffffff").is_err());
    }

    #[test]
    fn preset_apply_switches_mode_and_does_not_invent_a_wallpaper_or_schedule() -> anyhow::Result<()>
    {
        let dir = TestDir::new()?;
        let store = store(dir.path());
        for preset in presets()? {
            let report = apply_with_mode(&store, &preset, vec!["dojo".into()], |appearance| {
                assert_eq!(appearance, preset.appearance);
                Ok(())
            })?;
            assert_eq!(report.selection.appearance, preset.appearance);
            assert!(!report.selection.follow_system);
            let loaded = store.theme(&report.selection.generation)?;
            assert_eq!(loaded.preset.as_deref(), Some(preset.id.as_str()));
            assert!(loaded.mode.is_none());
            assert!(loaded.wallpaper.is_empty());
            assert!(!store.paths.root.join("current/wallpaper.txt").exists());
        }
        Ok(())
    }

    #[test]
    fn preset_changes_preserve_the_selected_wallpaper_bytes() -> anyhow::Result<()> {
        let dir = TestDir::new()?;
        let store = store(dir.path());
        let mut catalog = presets()?;
        let mut previous = Theme::from_preset(&catalog.remove(0), &store)?;
        // Stored wallpaper bytes, not an original file that might have changed.
        previous.wallpaper = "/wallpaper/source-no-longer-present.png".into();
        previous.image_bytes = b"selected wallpaper snapshot".to_vec();
        previous.wallpaper_sha256 = "retained snapshot hash".into();
        let generation = store.generate(&previous)?;
        store.commit(&Selection {
            generation,
            appearance: Appearance::Light,
            follow_system: false,
            targets: vec!["dojo".into()],
        })?;
        let theme = Theme::from_preset(&catalog[0], &store)?;
        assert_eq!(theme.wallpaper, previous.wallpaper);
        assert_eq!(theme.wallpaper_sha256, previous.wallpaper_sha256);
        assert_eq!(theme.image_bytes, previous.image_bytes);
        let generation = store.generate(&theme)?;
        assert_eq!(
            std::fs::read(
                store
                    .paths
                    .root
                    .join("generations")
                    .join(generation)
                    .join("wallpaper")
            )?,
            previous.image_bytes
        );
        Ok(())
    }

    #[test]
    fn failed_mode_switch_does_not_select_the_preset() -> anyhow::Result<()> {
        let dir = TestDir::new()?;
        let store = store(dir.path());
        let catalog = presets()?;
        apply_with_mode(&store, &catalog[0], vec!["dojo".into()], |_| Ok(()))?;
        let before = store.selection()?.context("selection")?;
        assert!(
            apply_with_mode(&store, &catalog[1], vec!["dojo".into()], |_| anyhow::bail!(
                "darkman unavailable"
            ))
            .is_err()
        );
        assert_eq!(
            store.selection()?.context("selection")?.generation,
            before.generation
        );
        assert!(
            apply_with_mode(&store, &catalog[1], vec!["invalid".into()], |_| panic!(
                "invalid targets must not switch mode"
            ))
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn legacy_wallpaper_metadata_still_loads() -> anyhow::Result<()> {
        let dir = TestDir::new()?;
        let mut theme = Theme::from_preset(&presets()?[0], &store(dir.path()))?;
        theme.preset = None;
        theme.mode = Some(crate::extraction::Mode::Normal);
        let json = serde_json::to_vec(&theme)?;
        let loaded: Theme = serde_json::from_slice(&json)?;
        assert_eq!(loaded.mode, Some(crate::extraction::Mode::Normal));
        assert!(loaded.preset.is_none());
        Ok(())
    }

    fn image_snapshot() -> anyhow::Result<Vec<u8>> {
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            2,
            2,
            image::Rgba([20, 40, 60, 255]),
        ))
        .write_to(&mut bytes, image::ImageFormat::Png)?;
        Ok(bytes.into_inner())
    }

    #[test]
    fn wallpaper_only_apply_preserves_palette_preset_mode_and_targets() -> anyhow::Result<()> {
        let dir = TestDir::new()?;
        let store = store(dir.path());
        let preset = presets()?.remove(3);
        let previous =
            apply_with_mode(&store, &preset, vec!["dojo".into(), "nvim".into()], |_| {
                Ok(())
            })?;
        let before = store.theme(&previous.selection.generation)?;
        let bytes = image_snapshot()?;
        let report = super::super::apply_wallpaper(
            &store,
            &dir.path().join("new.png"),
            bytes.clone(),
            None,
        )?;
        let after = store.theme(&report.selection.generation)?;
        assert_eq!(after.preset, before.preset);
        assert_eq!(after.mode, before.mode);
        assert_eq!(after.dark.colors, before.dark.colors);
        assert_eq!(after.light.colors, before.light.colors);
        assert_eq!(after.dark.terminal, before.dark.terminal);
        assert_eq!(report.selection.appearance, previous.selection.appearance);
        assert_eq!(
            report.selection.follow_system,
            previous.selection.follow_system
        );
        assert_eq!(report.selection.targets, previous.selection.targets);
        let selected = std::fs::read_to_string(store.paths.root.join("current/wallpaper.txt"))?;
        assert_eq!(std::fs::read(selected)?, bytes);
        assert!(
            super::super::apply_wallpaper(
                &store,
                &dir.path().join("bad.png"),
                b"not an image".to_vec(),
                None
            )
            .is_err()
        );
        assert_eq!(
            store.selection()?.context("selection")?.generation,
            report.selection.generation
        );
        Ok(())
    }

    #[test]
    fn a_generated_wallpaper_is_kept_as_a_recipe_until_an_image_replaces_it() -> anyhow::Result<()> {
        use crate::procedural::{FILE_NAME, Recipe, Style};
        let dir = TestDir::new()?;
        let store = store(dir.path());
        let preset = presets()?.remove(3);
        let previous = apply_with_mode(&store, &preset, vec!["dojo".into()], |_| Ok(()))?;
        let before = store.theme(&previous.selection.generation)?;
        let recipe = Recipe::new(Style::Contours, 42);

        let report = super::super::apply_procedural(&store, recipe.clone(), 2.4, None)?;
        let after = store.theme(&report.selection.generation)?;
        // Only the wallpaper changed; the palette, preset and targets are the theme's own.
        assert_eq!(after.preset, before.preset);
        assert_eq!(after.dark.colors, before.dark.colors);
        assert_eq!(report.selection.targets, previous.selection.targets);
        assert_eq!(after.recipe.as_ref(), Some(&recipe));
        let selected = std::path::PathBuf::from(std::fs::read_to_string(
            store.paths.root.join("current/wallpaper.txt"),
        )?);
        // The wallpaper surface finds the recipe beside the picture it is pointed at, and the
        // picture is the reference of the canvas's shape.
        let stored = Recipe::from_json(&std::fs::read(selected.with_file_name(FILE_NAME))?)?;
        assert_eq!(stored, recipe);
        let reference = image::load_from_memory(&std::fs::read(&selected)?)?;
        assert_eq!((reference.width(), reference.height()), (1920, 800));

        // Switching presets keeps the generated wallpaper.
        let other = presets()?.remove(5);
        let switched = apply_with_mode(&store, &other, vec!["dojo".into()], |_| Ok(()))?;
        assert_eq!(
            store.theme(&switched.selection.generation)?.recipe,
            Some(recipe)
        );
        let kept = std::path::PathBuf::from(std::fs::read_to_string(
            store.paths.root.join("current/wallpaper.txt"),
        )?);
        assert!(kept.with_file_name(FILE_NAME).is_file());

        // A chosen image replaces it, and no recipe is left beside the new picture.
        let replaced = super::super::apply_wallpaper(
            &store,
            &dir.path().join("photo.png"),
            image_snapshot()?,
            None,
        )?;
        assert_eq!(store.theme(&replaced.selection.generation)?.recipe, None);
        let photo = std::path::PathBuf::from(std::fs::read_to_string(
            store.paths.root.join("current/wallpaper.txt"),
        )?);
        assert!(!photo.with_file_name(FILE_NAME).exists());
        Ok(())
    }

    #[test]
    fn wallpaper_bootstrap_uses_existing_config_colors_without_extraction() -> anyhow::Result<()> {
        let dir = TestDir::new()?;
        let store = store(dir.path());
        let fallback = presets()?[0].palette.clone();
        let report = super::super::apply_wallpaper(
            &store,
            &dir.path().join("image.png"),
            image_snapshot()?,
            Some(fallback.clone()),
        )?;
        let theme = store.theme(&report.selection.generation)?;
        assert_eq!(theme.dark.colors, fallback.colors);
        assert_eq!(theme.light.colors, fallback.colors);
        assert!(theme.mode.is_none());
        assert!(theme.preset.is_none());
        assert_eq!(report.selection.appearance, fallback.appearance);
        Ok(())
    }
}
