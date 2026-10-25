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
    /// Peers whose remote input (cursor and keyboard) this node accepts. Normalised names.
    pub accept_input: Vec<String>,
    /// Where to reach the network's control plane, for the operator-only commands. Never holds a secret itself.
    pub control_plane: Option<ControlPlane>,
    /// How this node forms its mesh.
    pub method: MethodChoice,
    /// The directory holding the signed roster, for the roster method. Absolute.
    pub roster_dir: Option<PathBuf>,
    /// The file of trusted signing keys (`ssh-keygen` allowed-signers format), for the roster method. Absolute.
    pub signers_file: Option<PathBuf>,
    /// This node's name in the roster, when it is not its host name.
    pub node_name: Option<String>,
    /// The public half of this node's key, which the SSH agent holds the private half of. Absolute.
    pub node_key_file: Option<PathBuf>,
    /// The public half of the key this node sends remote input with, which can send nothing else. Absolute.
    pub input_key_file: Option<PathBuf>,
    /// The keys this node signs roster entries with, tried in this order: a private key file, or a
    /// public key file whose private half the agent holds. Absolute; none is required.
    pub signing_keys: Vec<PathBuf>,
    /// The SSH agent's socket, when it is not the one the environment names. Absolute.
    pub agent_socket: Option<PathBuf>,
}

/// The mesh methods a node can be set to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum MethodChoice {
    /// Membership from the local Tailscale client (the default).
    #[default]
    Tailscale,
    /// Membership from a signed roster directory.
    Roster,
}

/// Where a secret lives. The configuration only ever names it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
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
        } else if text.starts_with("op://") && text.len() > "op://".len() && !text.contains(char::is_control) {
            // Item names may hold spaces ("op://Personal/Some Item/field"), so only control characters are refused.
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
    #[serde(default)]
    accept_input: Vec<String>,
    control_plane: Option<RawControlPlane>,
    method: Option<String>,
    roster_dir: Option<PathBuf>,
    signers_file: Option<PathBuf>,
    node_name: Option<String>,
    node_key_file: Option<PathBuf>,
    input_key_file: Option<PathBuf>,
    agent_socket: Option<PathBuf>,
    #[serde(default)]
    signing_keys: Vec<PathBuf>,
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
        let accept_input = names("accept-input", &raw.accept_input)?;
        let control_plane = raw.control_plane.map(control_plane).transpose()?;
        let method = match raw.method.as_deref().map(str::trim) {
            None | Some("tailscale") => MethodChoice::Tailscale,
            Some("roster") => MethodChoice::Roster,
            Some(other) => return Err(ConfigError::Invalid(format!("method {other:?} is not known; use \"tailscale\" or \"roster\""))),
        };
        for (key, path) in [("roster-dir", &raw.roster_dir), ("signers-file", &raw.signers_file), ("node-key-file", &raw.node_key_file), ("input-key-file", &raw.input_key_file), ("agent-socket", &raw.agent_socket)] {
            if path.as_deref().is_some_and(|path| !path.is_absolute()) {
                return Err(ConfigError::Invalid(format!("{key} must be an absolute path")));
            }
        }
        if let Some(path) = raw.signing_keys.iter().find(|path| !path.is_absolute()) {
            return Err(ConfigError::Invalid(format!("signing-keys must be absolute paths, not {}", path.display())));
        }
        let node_name = match raw.node_name.map(|name| name.trim().to_string()) {
            None => None,
            Some(name) => {
                crate::signed::valid_name(&name).map_err(|reason| ConfigError::Invalid(format!("node-name: {reason}")))?;
                Some(name)
            }
        };
        Ok(Self { membership_tag, allow, deny, capabilities, accept_input, control_plane, method, roster_dir: raw.roster_dir, signers_file: raw.signers_file, node_name, node_key_file: raw.node_key_file, input_key_file: raw.input_key_file, agent_socket: raw.agent_socket, signing_keys: raw.signing_keys })
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

pub fn is_tag(tag: &str) -> bool {
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

/// Sets a top-level `key` to `value` (TOML text) in configuration text, keeping everything else as
/// it is: an existing line is replaced in place, a new one goes before the first table. A value the
/// text cannot take in place (an array spread over several lines) is refused, and so is any result
/// that does not read back as asked, so a hand-edited file is never left half changed.
pub fn set_top_level(text: &str, key: &str, value: &str) -> Result<String, String> {
    let line = format!("{key} = {value}");
    let wanted: toml::Table = toml::from_str(&line).map_err(|error| format!("{key}: {error}"))?;
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let first_table = lines.iter().position(|candidate| candidate.trim_start().starts_with('['));
    let top = first_table.unwrap_or(lines.len());
    let existing = lines[..top].iter().position(|candidate| {
        let candidate = candidate.trim_start();
        candidate.strip_prefix(key).is_some_and(|rest| rest.trim_start().starts_with('='))
    });
    match existing {
        Some(index) => {
            let current = lines[index].split_once('=').map(|(_, value)| value.trim()).unwrap_or_default();
            if current.starts_with('[') && !current.contains(']') {
                return Err(format!("{key} is spread over several lines in the configuration; set it by hand"));
            }
            lines[index] = line;
        }
        None => {
            // Keep a blank line between what comes before and the table that follows.
            let at = lines[..top].iter().rposition(|candidate| !candidate.trim().is_empty()).map_or(0, |last| last + 1);
            lines.insert(at, line);
            if first_table.is_some() && lines.get(at + 1).is_some_and(|next| !next.trim().is_empty()) {
                lines.insert(at + 1, String::new());
            }
        }
    }
    let result = lines.join("\n") + "\n";
    let read: toml::Table = toml::from_str(&result).map_err(|error| format!("the configuration would not read back ({error}); set {key} by hand"))?;
    if read.get(key) != wanted.get(key) {
        return Err(format!("{key} could not be set in the configuration; set it by hand"));
    }
    Ok(result)
}

/// Where the roster method keeps its files and finds its keys.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RosterPaths {
    /// The signed roster.
    pub dir: PathBuf,
    /// The trusted signing keys, `ssh-keygen` allowed-signers format.
    pub signers: PathBuf,
    /// The public half of this node's key.
    pub node_key: PathBuf,
    /// The public half of the key this node sends remote input with.
    pub input_key: PathBuf,
    /// Where pinned host keys are written for `ssh` to read.
    pub known_hosts: PathBuf,
    /// The SSH agent that holds this node's key: the configured socket, else 1Password's.
    pub agent: Option<PathBuf>,
}

/// The paths from the configuration or the defaults: the roster at `$XDG_DATA_HOME/dojo/mesh/roster`,
/// the pinned host keys beside it, the signers and node key under `$XDG_CONFIG_HOME/dojo/mesh`.
pub fn roster_paths(config: &MeshConfig, xdg_data_home: Option<PathBuf>, xdg_config_home: Option<PathBuf>, home: Option<PathBuf>) -> Option<RosterPaths> {
    let absolute = |path: Option<PathBuf>| path.filter(|path| path.is_absolute());
    let data = absolute(xdg_data_home).or_else(|| home.clone().map(|home| home.join(".local/share"))).map(|base| base.join("dojo/mesh"));
    let conf = absolute(xdg_config_home).or_else(|| home.clone().map(|home| home.join(".config"))).map(|base| base.join("dojo/mesh"));
    Some(RosterPaths {
        dir: config.roster_dir.clone().or_else(|| data.as_ref().map(|base| base.join("roster")))?,
        signers: config.signers_file.clone().or_else(|| conf.as_ref().map(|base| base.join("allowed_signers")))?,
        node_key: config.node_key_file.clone().or_else(|| conf.as_ref().map(|base| base.join("node_key.pub")))?,
        input_key: config.input_key_file.clone().or_else(|| conf.as_ref().map(|base| base.join("input_key.pub")))?,
        known_hosts: data?.join("known_hosts"),
        agent: config.agent_socket.clone().or_else(|| home.map(|home| home.join(".1password/agent.sock"))),
    })
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
        assert_eq!(SecretRef::parse("op://Personal/Some Item/field"), Ok(SecretRef::Op("op://Personal/Some Item/field".into())));
        for bad in ["tskey-api-abc123", "env:", "env:A B", "file:relative", "op://", "op://a\nb", ""] {
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
    fn reads_the_peers_whose_input_is_accepted() {
        let config = MeshConfig::parse("accept-input = [\"Beta\", \"gamma.example.ts.net.\", \"beta\"]").unwrap();
        assert_eq!(config.accept_input, ["beta", "gamma.example.ts.net"]);
        assert!(MeshConfig::default().accept_input.is_empty(), "nobody may send input unless named");
        assert!(MeshConfig::parse("accept-input = [\" \"]").is_err());
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

    #[test]
    fn the_method_defaults_to_tailscale_and_roster_settings_are_read() {
        assert_eq!(MeshConfig::parse("").unwrap().method, MethodChoice::Tailscale);
        let config = MeshConfig::parse("method = \"roster\"\nroster-dir = \"/srv/roster\"\nsigners-file = \"/etc/signers\"\nnode-name = \"orchard\"").unwrap();
        assert_eq!(config.method, MethodChoice::Roster);
        assert_eq!(config.roster_dir.as_deref(), Some(Path::new("/srv/roster")));
        assert_eq!(config.signers_file.as_deref(), Some(Path::new("/etc/signers")));
        assert_eq!(config.node_name.as_deref(), Some("orchard"));
    }

    #[test]
    fn an_unknown_method_a_relative_path_or_a_bad_node_name_is_refused() {
        for (text, wanted) in [
            ("method = \"carrier-pigeon\"", "is not known"),
            ("roster-dir = \"relative\"", "roster-dir must be an absolute path"),
            ("signers-file = \"x/y\"", "signers-file must be an absolute path"),
            ("node-name = \"Bad_Name\"", "node-name"),
        ] {
            let error = MeshConfig::parse(text).unwrap_err().to_string();
            assert!(error.contains(wanted), "{text}: {error}");
        }
    }

    #[test]
    fn roster_paths_follow_the_configuration_then_the_xdg_defaults_then_home() {
        let config = MeshConfig::default();
        let home = Some(PathBuf::from("/home/n"));
        let paths = roster_paths(&config, None, None, home.clone()).unwrap();
        assert_eq!(paths.dir, Path::new("/home/n/.local/share/dojo/mesh/roster"));
        assert_eq!(paths.known_hosts, Path::new("/home/n/.local/share/dojo/mesh/known_hosts"));
        assert_eq!(paths.signers, Path::new("/home/n/.config/dojo/mesh/allowed_signers"));
        assert_eq!(paths.node_key, Path::new("/home/n/.config/dojo/mesh/node_key.pub"));
        assert_eq!(paths.input_key, Path::new("/home/n/.config/dojo/mesh/input_key.pub"));
        assert_eq!(paths.agent.as_deref(), Some(Path::new("/home/n/.1password/agent.sock")));
        let paths = roster_paths(&config, Some("/data".into()), Some("/conf".into()), home.clone()).unwrap();
        assert_eq!((paths.dir, paths.signers), ("/data/dojo/mesh/roster".into(), "/conf/dojo/mesh/allowed_signers".into()));
        let set = MeshConfig::parse("roster-dir = \"/r\"\nsigners-file = \"/s\"\nnode-key-file = \"/k.pub\"\ninput-key-file = \"/i.pub\"\nagent-socket = \"/a.sock\"").unwrap();
        let paths = roster_paths(&set, Some("/data".into()), None, None).unwrap();
        assert_eq!((paths.dir, paths.signers, paths.node_key, paths.agent), ("/r".into(), "/s".into(), "/k.pub".into(), Some("/a.sock".into())));
        assert_eq!(roster_paths(&config, None, None, None), None);
    }

    #[test]
    fn a_top_level_setting_is_replaced_in_place_or_added_before_the_first_table() {
        let text = "# mine\nallow = [\"a\"]\nmethod = \"tailscale\"\n\n[capability.x]\nmethod = \"keep\"\n";
        let changed = set_top_level(text, "method", "\"roster\"").unwrap();
        assert_eq!(changed, "# mine\nallow = [\"a\"]\nmethod = \"roster\"\n\n[capability.x]\nmethod = \"keep\"\n");
        let added = set_top_level("allow = [\"a\"]\n\n[capability.x]\nport = 1\n", "node-name", "\"harbor\"").unwrap();
        assert_eq!(added, "allow = [\"a\"]\nnode-name = \"harbor\"\n\n[capability.x]\nport = 1\n");
        assert_eq!(set_top_level("", "method", "\"roster\"").unwrap(), "method = \"roster\"\n");
        assert_eq!(set_top_level("allow = []", "method", "\"roster\"").unwrap(), "allow = []\nmethod = \"roster\"\n");
        let table_only = set_top_level("[capability.x]\nport = 1\n", "method", "\"roster\"").unwrap();
        assert!(table_only.starts_with("method = \"roster\"\n\n[capability.x]"), "{table_only}");
        let config = MeshConfig::parse(&added).unwrap();
        assert_eq!(config.node_name.as_deref(), Some("harbor"));
    }

    #[test]
    fn a_setting_that_cannot_be_changed_safely_is_refused() {
        assert!(set_top_level("signing-keys = [\n  \"/a\",\n]\n", "signing-keys", "[\"/b\"]").is_err());
        assert!(set_top_level("x = 1", "method", "\"unterminated").is_err());
        assert!(set_top_level("allow = [", "method", "\"roster\"").is_err());
    }
}
