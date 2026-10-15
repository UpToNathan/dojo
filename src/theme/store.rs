use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context as _;
use serde::{Deserialize, Serialize};

use super::{Appearance, Theme};

#[derive(Clone, Debug)]
pub struct Paths {
    pub home: PathBuf,
    pub config: PathBuf,
    pub data: PathBuf,
    pub root: PathBuf,
}

impl Paths {
    pub fn user_default() -> anyhow::Result<Self> {
        let home = PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?);
        let xdg = |key: &str, fallback: &str| -> anyhow::Result<PathBuf> {
            let path = std::env::var_os(key)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(fallback));
            anyhow::ensure!(path.is_absolute(), "{key} must be absolute");
            Ok(path)
        };
        let config = xdg("XDG_CONFIG_HOME", ".config")?;
        let data = xdg("XDG_DATA_HOME", ".local/share")?;
        let root = xdg("XDG_STATE_HOME", ".local/state")?.join("dojo/themes");
        Ok(Self {
            home,
            config,
            data,
            root,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub generation: String,
    pub appearance: Appearance,
    pub follow_system: bool,
    #[serde(deserialize_with = "active_targets")]
    pub targets: Vec<String>,
}

fn active_targets<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<String>, D::Error> {
    let targets = Vec::<String>::deserialize(deserializer)?;
    Ok(targets
        .into_iter()
        .filter(|target| !matches!(target.as_str(), "zed" | "delta"))
        .collect())
}

#[cfg(test)]
#[test]
fn saved_selections_discard_retired_application_targets() -> anyhow::Result<()> {
    let selection: Selection = serde_json::from_str(
        r#"{"generation":"saved","appearance":"dark","follow_system":false,"targets":["dojo","zed","t3code","delta","yazi"]}"#,
    )?;
    assert_eq!(selection.targets, ["dojo", "t3code", "yazi"]);
    assert_eq!(selection.generation, "saved");
    Ok(())
}

pub struct ThemeStore {
    pub paths: Paths,
}

impl ThemeStore {
    pub fn new(paths: Paths) -> Self {
        Self { paths }
    }
    pub fn lock(&self) -> anyhow::Result<File> {
        fs::create_dir_all(&self.paths.root)?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.paths.root.join("apply.lock"))?;
        file.lock()?;
        Ok(file)
    }
    pub fn generate(&self, theme: &Theme) -> anyhow::Result<String> {
        let id = format!(
            "{}-{}",
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos(),
            std::process::id()
        );
        let generations = self.paths.root.join("generations");
        fs::create_dir_all(&generations)?;
        let staging = generations.join(format!(".building-{id}"));
        fs::create_dir(&staging)?;
        if !theme.image_bytes.is_empty() {
            write_atomic(&staging.join("wallpaper"), &theme.image_bytes)?;
        }
        write_atomic(
            &staging.join("theme.json"),
            &serde_json::to_vec_pretty(theme)?,
        )?;
        for appearance in [Appearance::Dark, Appearance::Light] {
            let dir = staging.join(appearance.name());
            fs::create_dir(&dir)?;
            for (name, bytes) in theme.outputs(appearance)? {
                write_atomic(&dir.join(name), &bytes)?;
            }
            if !theme.image_bytes.is_empty() {
                write_atomic(
                    &dir.join("wallpaper.txt"),
                    generations
                        .join(&id)
                        .join("wallpaper")
                        .to_string_lossy()
                        .as_bytes(),
                )?;
            }
        }
        fs::rename(staging, generations.join(&id))?;
        Ok(id)
    }
    pub fn theme(&self, generation: &str) -> anyhow::Result<Theme> {
        validate_id(generation)?;
        let path = self
            .paths
            .root
            .join("generations")
            .join(generation)
            .join("theme.json");
        let theme: Theme = serde_json::from_slice(&fs::read(&path)?)
            .with_context(|| format!("reading {}", path.display()))?;
        anyhow::ensure!(
            theme.version == 1,
            "unsupported theme version {}",
            theme.version
        );
        theme.dark.validate()?;
        theme.light.validate()?;
        Ok(theme)
    }
    pub fn selection(&self) -> anyhow::Result<Option<Selection>> {
        if let Some(selection) =
            read_json_optional(&self.paths.root.join("current/selection.json"))?
        {
            return Ok(Some(selection));
        }
        read_json_optional(&self.paths.root.join("selection.json"))
    }
    pub fn previous(&self) -> anyhow::Result<Option<Selection>> {
        if self.paths.root.join("current/selection.json").is_file() {
            return read_json_optional(&self.paths.root.join("current/previous.json"));
        }
        read_json_optional(&self.paths.root.join("previous.json"))
    }
    pub fn commit(&self, selection: &Selection) -> anyhow::Result<()> {
        validate_id(&selection.generation)?;
        let destination = PathBuf::from("generations")
            .join(&selection.generation)
            .join(selection.appearance.name());
        anyhow::ensure!(
            self.paths.root.join(&destination).is_dir(),
            "generation is missing"
        );
        let active = self.paths.root.join("current");
        if let Ok(info) = fs::symlink_metadata(&active) {
            anyhow::ensure!(
                info.file_type().is_symlink(),
                "refusing to replace non-symlink {}",
                active.display()
            );
        }
        let activation_id = format!(
            "{}-{}",
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos(),
            std::process::id()
        );
        let activations = self.paths.root.join("activations");
        fs::create_dir_all(&activations)?;
        let staged = activations.join(format!(".building-{activation_id}"));
        fs::create_dir(&staged)?;
        for entry in fs::read_dir(self.paths.root.join(&destination))? {
            let entry = entry?;
            anyhow::ensure!(
                entry.file_type()?.is_file(),
                "generated appearance contains a non-regular file"
            );
            symlink(
                self.paths.root.join(&destination).join(entry.file_name()),
                staged.join(entry.file_name()),
            )?;
        }
        write_atomic(
            &staged.join("selection.json"),
            &serde_json::to_vec_pretty(selection)?,
        )?;
        if let Some(previous) = self.selection()? {
            write_atomic(
                &staged.join("previous.json"),
                &serde_json::to_vec_pretty(&previous)?,
            )?;
        }
        fs::rename(staged, activations.join(&activation_id))?;
        let temporary = self.paths.root.join(format!(
            ".current-{}",
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
        ));
        symlink(PathBuf::from("activations").join(activation_id), &temporary)?;
        fs::rename(temporary, active)?;
        Ok(())
    }
}

pub(super) fn validate_id(id: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'),
        "invalid generation id"
    );
    Ok(())
}

pub(super) fn read_json_optional<T: serde::de::DeserializeOwned>(
    path: &Path,
) -> anyhow::Result<Option<T>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(
            serde_json::from_slice(&bytes)
                .with_context(|| format!("reading {}", path.display()))?,
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
    }
}

pub(super) fn write_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let parent = path.parent().context("output path needs a parent")?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(
        ".dojo-{}-{}.tmp",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    if let Err(error) = (|| -> std::io::Result<()> {
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        Ok(())
    })() {
        if let Err(cleanup) = fs::remove_file(&temporary) {
            eprintln!("Could not clean temporary theme file: {cleanup}");
        }
        return Err(error).with_context(|| format!("publishing {}", path.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generation_paths_cannot_escape_storage() {
        for invalid in ["../etc", "/tmp/x", "", "a/b", "a\\b", "a b"] {
            assert!(validate_id(invalid).is_err());
        }
        assert!(validate_id("1234-5678").is_ok());
    }

    #[test]
    fn palette_and_selection_commit_as_one_snapshot_with_previous_state() -> anyhow::Result<()> {
        let root = std::env::temp_dir().join(format!(
            "dojo-snapshot-{}",
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
        ));
        fs::create_dir(&root)?;
        let store = ThemeStore::new(Paths {
            home: root.clone(),
            config: root.join("config"),
            data: root.join("data"),
            root: root.join("themes"),
        });
        let colors = crate::extraction::Palette {
            colors: [crate::extraction::Rgb(20, 25, 35); 16],
        };
        let theme = Theme {
            version: 1,
            wallpaper: "fixture".into(),
            wallpaper_sha256: "fixture".into(),
            image_bytes: Vec::new(),
            mode: Some(crate::extraction::Mode::Normal),
            preset: None,
            algorithm_revision: "fixture".into(),
            dark: super::super::SemanticPalette::derive(&colors, Appearance::Dark),
            light: super::super::SemanticPalette::derive(&colors, Appearance::Light),
        };
        let generation = store.generate(&theme)?;
        let mut selection = Selection {
            generation,
            appearance: Appearance::Dark,
            follow_system: false,
            targets: vec!["dojo".into()],
        };
        store.commit(&selection)?;
        assert!(store.paths.root.join("current/dojo.json").is_file());
        assert_eq!(
            store.selection()?.map(|s| s.appearance),
            Some(Appearance::Dark)
        );
        assert!(store.previous()?.is_none());
        selection.appearance = Appearance::Light;
        store.commit(&selection)?;
        assert_eq!(
            store.selection()?.map(|s| s.appearance),
            Some(Appearance::Light)
        );
        assert_eq!(
            store.previous()?.map(|s| s.appearance),
            Some(Appearance::Dark)
        );
        let palette: serde_json::Value =
            serde_json::from_slice(&fs::read(store.paths.root.join("current/palette.json"))?)?;
        assert_eq!(palette["appearance"], "light");
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
