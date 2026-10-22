//! The triggers users chose, kept per app so a rebinding survives restarts.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write as _;
use std::path::PathBuf;

use anyhow::{Context as _, bail};
use serde::{Deserialize, Serialize};

use super::trigger::Trigger;

/// What the user chose for one shortcut. `None` means they declined to bind it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Assignment {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<Trigger>,
    /// Shown by `ListShortcuts` before the app binds again.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

/// Choices by app, then by the app's own shortcut id.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Assignments {
    #[serde(default)]
    pub apps: BTreeMap<String, BTreeMap<String, Assignment>>,
}

pub struct Store {
    path: PathBuf,
}

impl Store {
    pub fn user_default() -> anyhow::Result<Self> {
        let state_home = match std::env::var_os("XDG_STATE_HOME") {
            Some(path) => PathBuf::from(path),
            None => std::env::var_os("HOME")
                .map(PathBuf::from)
                .context("neither XDG_STATE_HOME nor HOME is set")?
                .join(".local/state"),
        };
        if !state_home.is_absolute() {
            bail!("XDG_STATE_HOME must be absolute");
        }
        Ok(Self::at(state_home.join("dojo/global-shortcuts.toml")))
    }

    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    /// A missing file is empty. A file that cannot be read is moved aside, so the next save
    /// does not destroy it, and the problem is returned for the caller to report.
    pub fn load(&self) -> (Assignments, Option<anyhow::Error>) {
        let contents = match fs::read_to_string(&self.path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return (Assignments::default(), None);
            }
            Err(error) => {
                return (
                    Assignments::default(),
                    Some(anyhow::Error::new(error).context(format!(
                        "reading {}",
                        self.path.display()
                    ))),
                );
            }
        };
        match toml::from_str(&contents) {
            Ok(assignments) => (assignments, None),
            Err(error) => {
                let aside = self.path.with_extension("toml.corrupt");
                let moved = fs::rename(&self.path, &aside);
                let problem = anyhow::Error::new(error).context(match moved {
                    Ok(()) => format!(
                        "invalid shortcut choices in {}, kept as {}",
                        self.path.display(),
                        aside.display()
                    ),
                    Err(_) => format!("invalid shortcut choices in {}", self.path.display()),
                });
                (Assignments::default(), Some(problem))
            }
        }
    }

    pub fn save(&self, assignments: &Assignments) -> anyhow::Result<()> {
        let parent = self
            .path
            .parent()
            .context("shortcut state path has no parent directory")?;
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        let contents = toml::to_string(assignments).context("serializing shortcut choices")?;
        let mut file = tempfile::NamedTempFile::new_in(parent)
            .with_context(|| format!("creating a temporary file in {}", parent.display()))?;
        file.write_all(contents.as_bytes())?;
        file.as_file().sync_all()?;
        file.persist(&self.path)
            .with_context(|| format!("writing {}", self.path.display()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trigger(text: &str) -> Trigger {
        Trigger::from_xdg(text).unwrap()
    }

    #[test]
    fn round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::at(dir.path().join("nested/global-shortcuts.toml"));
        let mut assignments = Assignments::default();
        let app = assignments.apps.entry("org.example.Recorder".into()).or_default();
        app.insert(
            "toggle".into(),
            Assignment {
                trigger: Some(trigger("CTRL+ALT+r")),
                description: "Start or stop recording".into(),
            },
        );
        app.insert("declined".into(), Assignment::default());
        store.save(&assignments).unwrap();

        let (loaded, problem) = store.load();
        assert!(problem.is_none());
        assert_eq!(loaded, assignments);
    }

    #[test]
    fn a_missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let (loaded, problem) = Store::at(dir.path().join("none.toml")).load();
        assert_eq!(loaded, Assignments::default());
        assert!(problem.is_none());
    }

    #[test]
    fn a_corrupt_file_is_kept_aside_and_reported() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("global-shortcuts.toml");
        fs::write(&path, "apps = [not toml").unwrap();
        let store = Store::at(path.clone());

        let (loaded, problem) = store.load();
        assert_eq!(loaded, Assignments::default());
        assert!(problem.unwrap().to_string().contains("kept as"));
        assert!(!path.exists());
        assert!(dir.path().join("global-shortcuts.toml.corrupt").exists());

        store.save(&Assignments::default()).unwrap();
        assert_eq!(store.load().0, Assignments::default());
    }

    #[test]
    fn a_bad_trigger_makes_the_file_corrupt_rather_than_silently_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("global-shortcuts.toml");
        fs::write(&path, "[apps.a.b]\ntrigger = \"Ctrl+Nope Nope\"\n").unwrap();
        let (_, problem) = Store::at(path).load();
        assert!(problem.is_some());
    }
}
