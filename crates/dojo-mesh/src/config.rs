use serde::Serialize;
use std::path::{Path, PathBuf};

/// A node's own mesh settings. Nothing here is shared with other nodes: every node decides alone
/// whom it admits.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct MeshConfig {
    /// The Tailscale tag that marks a device as a candidate node. Without one, only `allow` admits peers.
    pub membership_tag: Option<String>,
    /// Devices admitted whether or not they carry the tag. Normalised names (see [`name_key`]).
    pub allow: Vec<String>,
    /// Devices refused whether or not they carry the tag. Wins over `allow`.
    pub deny: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ConfigError {
    Read(String),
    Parse(String),
    Invalid(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read(message) | Self::Parse(message) | Self::Invalid(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for ConfigError {}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct Raw {
    membership_tag: Option<String>,
    #[serde(default)]
    allow: Vec<String>,
    #[serde(default)]
    deny: Vec<String>,
}

/// Device names compare without case and without the DNS root dot.
pub fn name_key(name: &str) -> String {
    name.trim().trim_end_matches('.').to_ascii_lowercase()
}

impl MeshConfig {
    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        let raw: Raw = toml::from_str(text).map_err(|error| ConfigError::Parse(error.message().to_string()))?;
        let membership_tag = match raw.membership_tag.map(|tag| tag.trim().to_string()) {
            None => None,
            Some(tag) if is_tag(&tag) => Some(tag),
            Some(tag) => {
                return Err(ConfigError::Invalid(format!(
                    "membership-tag {tag:?} is not a Tailscale tag; expected something like \"tag:name\""
                )));
            }
        };
        let allow = names("allow", &raw.allow)?;
        let deny = names("deny", &raw.deny)?;
        if let Some(both) = allow.iter().find(|name| deny.contains(name)) {
            return Err(ConfigError::Invalid(format!("{both:?} is listed in both allow and deny")));
        }
        Ok(Self { membership_tag, allow, deny })
    }

    /// Loads the file at `path`. A missing file is the empty configuration, a mesh of one.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).map_err(|error| match error {
                ConfigError::Parse(message) => ConfigError::Parse(format!("{}: {message}", path.display())),
                ConfigError::Invalid(message) => ConfigError::Invalid(format!("{}: {message}", path.display())),
                other => other,
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(ConfigError::Read(format!("{}: {error}", path.display()))),
        }
    }
}

fn is_tag(tag: &str) -> bool {
    tag.strip_prefix("tag:").is_some_and(|rest| {
        !rest.is_empty() && rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    })
}

fn names(field: &str, entries: &[String]) -> Result<Vec<String>, ConfigError> {
    let mut keys: Vec<String> = Vec::new();
    for entry in entries {
        let key = name_key(entry);
        if key.is_empty() {
            return Err(ConfigError::Invalid(format!("{field} contains an empty name")));
        }
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    Ok(keys)
}

/// `$XDG_CONFIG_HOME/dojo/mesh.toml`, falling back to `~/.config`.
pub fn default_path(xdg_config_home: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    let base = xdg_config_home.filter(|path| path.is_absolute()).or_else(|| home.map(|home| home.join(".config")))?;
    Some(base.join("dojo").join("mesh.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_is_a_mesh_of_one() {
        assert_eq!(MeshConfig::parse("").unwrap(), MeshConfig::default());
    }

    #[test]
    fn reads_tag_and_overrides_with_names_normalised() {
        let config = MeshConfig::parse(
            "membership-tag = \"tag:mesh\"\nallow = [\"Alpha\", \"beta.example.ts.net.\", \"alpha\"]\ndeny = [\"gamma\"]\n",
        )
        .unwrap();
        assert_eq!(config.membership_tag.as_deref(), Some("tag:mesh"));
        assert_eq!(config.allow, ["alpha", "beta.example.ts.net"]);
        assert_eq!(config.deny, ["gamma"]);
    }

    #[test]
    fn rejects_a_tag_that_tailscale_would_not_know() {
        for tag in ["mesh", "tag:", "tag:a b", "tag:a/b"] {
            let error = MeshConfig::parse(&format!("membership-tag = \"{tag}\"")).unwrap_err();
            assert!(matches!(error, ConfigError::Invalid(_)), "{tag}: {error}");
        }
    }

    #[test]
    fn rejects_a_name_that_is_both_allowed_and_denied() {
        let error = MeshConfig::parse("allow = [\"Alpha\"]\ndeny = [\"alpha.\"]").unwrap_err();
        assert!(error.to_string().contains("both allow and deny"), "{error}");
    }

    #[test]
    fn rejects_unknown_keys_and_empty_names() {
        assert!(matches!(MeshConfig::parse("tag = \"tag:x\""), Err(ConfigError::Parse(_))));
        assert!(matches!(MeshConfig::parse("allow = [\" \"]"), Err(ConfigError::Invalid(_))));
    }

    #[test]
    fn a_missing_file_is_the_default_and_a_bad_one_names_its_path() {
        let dir = std::env::temp_dir().join(format!("dojo-mesh-config-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(MeshConfig::load(&dir.join("absent.toml")).unwrap(), MeshConfig::default());
        let bad = dir.join("bad.toml");
        std::fs::write(&bad, "allow = 3").unwrap();
        let error = MeshConfig::load(&bad).unwrap_err().to_string();
        assert!(error.starts_with(&bad.display().to_string()), "{error}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn default_path_prefers_an_absolute_xdg_directory() {
        assert_eq!(
            default_path(Some("/x".into()), Some("/h".into())),
            Some(PathBuf::from("/x/dojo/mesh.toml"))
        );
        assert_eq!(
            default_path(Some("relative".into()), Some("/h".into())),
            Some(PathBuf::from("/h/.config/dojo/mesh.toml"))
        );
        assert_eq!(default_path(None, None), None);
    }
}
