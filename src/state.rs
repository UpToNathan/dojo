use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use anyhow::{Context as _, bail};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Preferences {
    pub frame_enabled: bool,
}

pub struct StateStore {
    path: PathBuf,
    legacy_paths: Vec<PathBuf>,
}

impl StateStore {
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
        Ok(Self {
            path: state_home.join("dojo/state.toml"),
            legacy_paths: ["elysium", "khanda"]
                .into_iter()
                .map(|name| state_home.join(name).join("state.toml"))
                .collect(),
        })
    }

    pub fn load(&self) -> anyhow::Result<Option<Preferences>> {
        match fs::read_to_string(&self.path) {
            Ok(contents) => Ok(Some(toml::from_str(&contents).with_context(|| {
                format!("invalid Dojo state at {}", self.path.display())
            })?)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                for legacy_path in &self.legacy_paths {
                    match fs::read_to_string(legacy_path) {
                        Ok(contents) => {
                            let preferences = toml::from_str(&contents).with_context(|| {
                                format!("invalid legacy state at {}", legacy_path.display())
                            })?;
                            self.save(preferences)?;
                            return Ok(Some(preferences));
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => {
                            return Err(error)
                                .with_context(|| format!("reading {}", legacy_path.display()));
                        }
                    }
                }
                Ok(None)
            }
            Err(error) => Err(error).with_context(|| format!("reading {}", self.path.display())),
        }
    }

    pub fn save(&self, preferences: Preferences) -> anyhow::Result<()> {
        let parent = self
            .path
            .parent()
            .context("Dojo state path has no parent directory")?;
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;

        let temporary_path = self
            .path
            .with_extension(format!("toml.{}.tmp", std::process::id()));
        let contents = toml::to_string(&preferences).context("serializing Dojo state")?;
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temporary_path)
            .with_context(|| format!("creating {}", temporary_path.display()))?;
        file.write_all(contents.as_bytes())
            .with_context(|| format!("writing {}", temporary_path.display()))?;
        file.sync_all()
            .with_context(|| format!("syncing {}", temporary_path.display()))?;
        fs::rename(&temporary_path, &self.path).with_context(|| {
            format!(
                "replacing Dojo state file {} with {}",
                self.path.display(),
                temporary_path.display()
            )
        })?;
        Ok(())
    }
}

pub fn persist_frame_enabled(enabled: bool) -> anyhow::Result<()> {
    StateStore::user_default()?.save(Preferences {
        frame_enabled: enabled,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferences_round_trip_through_toml() {
        let preferences = Preferences {
            frame_enabled: false,
        };
        let serialized = toml::to_string(&preferences).expect("preferences should serialize");
        let decoded: Preferences = toml::from_str(&serialized).expect("preferences should parse");
        assert_eq!(decoded, preferences);
    }

    #[test]
    fn migrates_state_from_previous_names() {
        let root =
            std::env::temp_dir().join(format!("dojo-state-migration-test-{}", std::process::id()));
        for legacy_name in ["elysium", "khanda"] {
            let legacy_path = root.join(legacy_name).join("state.toml");
            let new_path = root.join("dojo/state.toml");
            fs::create_dir_all(legacy_path.parent().unwrap()).unwrap();
            fs::write(
                &legacy_path,
                toml::to_string(&Preferences {
                    frame_enabled: true,
                })
                .unwrap(),
            )
            .unwrap();

            let store = StateStore {
                path: new_path.clone(),
                legacy_paths: vec![legacy_path.clone()],
            };
            assert_eq!(
                store.load().unwrap(),
                Some(Preferences {
                    frame_enabled: true
                })
            );
            assert_eq!(
                fs::read_to_string(new_path).unwrap(),
                fs::read_to_string(legacy_path).unwrap()
            );
            fs::remove_dir_all(&root).unwrap();
        }
    }
}
