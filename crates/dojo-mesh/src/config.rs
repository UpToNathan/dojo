use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// What a node says it offers: capability name, then attribute name and value. Values are text;
/// numbers and booleans in the file are kept as their text.
pub type Capabilities = BTreeMap<String, BTreeMap<String, String>>;

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
    pub capabilities: Capabilities,
    /// Where to reach the network's control plane, for the operator-only commands. Never holds a secret itself.
    pub control_plane: Option<ControlPlane>,
}

/// Where a secret lives. The configuration only ever names it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SecretRef {
    /// An environment variable: `env:NAME`.
    Env(String),
    /// A file whose contents are the secret: `file:/absolute/path`.
    File(PathBuf),
    /// A 1Password reference read with `op read`: `op://vault/item/field`.
    Op(String),
}

impl SecretRef {
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        if let Some(name) = text.strip_prefix("env:") {
            if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Ok(Self::Env(name.into()));
            }
        } else if let Some(path) = text.strip_prefix("file:") {
            if Path::new(path).is_absolute() {
                return Ok(Self::File(path.into()));
            }
        } else if text.starts_with("op://") && text.len() > "op://".len() && !text.contains(char::is_whitespace) {
            return Ok(Self::Op(text.into()));
        }
        Err("not a secret reference; use env:NAME, file:/absolute/path or op://vault/item/field (the secret itself never goes in this file)".into())
    }
}

/// How the operator-only commands sign in to the Tailscale API.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ControlPlane {
    /// The tailnet to manage; `-` means the one the credential belongs to.
    pub tailnet: String,
    pub credential: Credential,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Credential {
    ApiToken(SecretRef),
    OauthClient { client_id: SecretRef, client_secret: SecretRef },
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
    #[serde(default)]
    capability: BTreeMap<String, BTreeMap<String, toml::Value>>,
    control_plane: Option<RawControlPlane>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct RawControlPlane {
    tailnet: Option<String>,
    api_token: Option<String>,
    client_id: Option<String>,
    client_secret: Option<String>,
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
        let capabilities = capabilities(raw.capability)?;
        let control_plane = raw.control_plane.map(control_plane).transpose()?;
        Ok(Self { membership_tag, allow, deny, capabilities, control_plane })
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

fn slug(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn capabilities(raw: BTreeMap<String, BTreeMap<String, toml::Value>>) -> Result<Capabilities, ConfigError> {
    let mut out = Capabilities::new();
    for (name, attributes) in raw {
        if !slug(&name) {
            return Err(ConfigError::Invalid(format!("capability name {name:?} must be lowercase letters, digits and dashes")));
        }
        let mut values = BTreeMap::new();
        for (attribute, value) in attributes {
            if !slug(&attribute) {
                return Err(ConfigError::Invalid(format!("attribute {attribute:?} of capability {name} must be lowercase letters, digits and dashes")));
            }
            let text = match value {
                toml::Value::String(text) => text,
                toml::Value::Integer(number) => number.to_string(),
                toml::Value::Boolean(flag) => flag.to_string(),
                _ => return Err(ConfigError::Invalid(format!("{name}.{attribute} must be text, a number or true/false"))),
            };
            values.insert(attribute, text);
        }
        out.insert(name, values);
    }
    Ok(out)
}

fn control_plane(raw: RawControlPlane) -> Result<ControlPlane, ConfigError> {
    let secret = |field: &str, text: &str| {
        SecretRef::parse(text).map_err(|reason| ConfigError::Invalid(format!("control-plane {field}: {reason}")))
    };
    let tailnet = raw.tailnet.map(|name| name.trim().to_string()).unwrap_or_else(|| "-".into());
    if tailnet.is_empty() || tailnet.contains(['/', '?', '#', ' ']) {
        return Err(ConfigError::Invalid(format!("control-plane tailnet {tailnet:?} is not a tailnet name")));
    }
    let credential = match (raw.api_token, raw.client_id, raw.client_secret) {
        (Some(token), None, None) => Credential::ApiToken(secret("api-token", &token)?),
        (None, Some(id), Some(client_secret)) => {
            Credential::OauthClient { client_id: secret("client-id", &id)?, client_secret: secret("client-secret", &client_secret)? }
        }
        _ => {
            return Err(ConfigError::Invalid(
                "control-plane needs either api-token, or both client-id and client-secret".into(),
            ));
        }
    };
    Ok(ControlPlane { tailnet, credential })
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
    fn reads_capabilities_with_scalar_attributes() {
        let config = MeshConfig::parse(
            "[capability.nix-cache]\nport = 5000\nkey-name = \"alpha-1\"\nopen = true\n\n[capability.voice-agent]\n",
        )
        .unwrap();
        let cache = &config.capabilities["nix-cache"];
        assert_eq!(cache["port"], "5000");
        assert_eq!(cache["key-name"], "alpha-1");
        assert_eq!(cache["open"], "true");
        assert!(config.capabilities["voice-agent"].is_empty());
    }

    #[test]
    fn rejects_malformed_capabilities() {
        for text in [
            "[capability.Bad_Name]\nx = 1",
            "[capability.ok]\nBad = 1",
            "[capability.ok]\nlist = [1, 2]",
            "[capability.ok.nested]\nx = 1",
        ] {
            assert!(MeshConfig::parse(text).is_err(), "{text}");
        }
    }

    #[test]
    fn secret_references_name_a_place_never_a_value() {
        assert_eq!(SecretRef::parse("env:TS_TOKEN"), Ok(SecretRef::Env("TS_TOKEN".into())));
        assert_eq!(SecretRef::parse("file:/run/secrets/ts"), Ok(SecretRef::File("/run/secrets/ts".into())));
        assert_eq!(SecretRef::parse("op://Vault/Item/field"), Ok(SecretRef::Op("op://Vault/Item/field".into())));
        for bad in ["tskey-api-abc123", "env:", "env:A B", "file:relative", "op://", "op://a b", ""] {
            assert!(SecretRef::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn control_plane_takes_a_token_or_a_client_pair() {
        let token = MeshConfig::parse("[control-plane]\napi-token = \"env:TOKEN\"").unwrap().control_plane.unwrap();
        assert_eq!(token.tailnet, "-");
        assert_eq!(token.credential, Credential::ApiToken(SecretRef::Env("TOKEN".into())));
        let client = MeshConfig::parse("[control-plane]\ntailnet = \"example.com\"\nclient-id = \"env:ID\"\nclient-secret = \"op://v/i/f\"").unwrap().control_plane.unwrap();
        assert_eq!(client.tailnet, "example.com");
        assert!(matches!(client.credential, Credential::OauthClient { .. }));
    }

    #[test]
    fn control_plane_rejects_raw_secrets_and_partial_credentials() {
        for text in [
            "[control-plane]\napi-token = \"tskey-api-real\"",
            "[control-plane]",
            "[control-plane]\nclient-id = \"env:ID\"",
            "[control-plane]\napi-token = \"env:T\"\nclient-id = \"env:ID\"\nclient-secret = \"env:S\"",
            "[control-plane]\ntailnet = \"a/b\"\napi-token = \"env:T\"",
        ] {
            assert!(matches!(MeshConfig::parse(text), Err(ConfigError::Invalid(_))), "{text}");
        }
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
