//! What a node offers its peers. A node describes itself; a consumer asks for a capability and is
//! told which peers offer it, never naming a peer in advance.

use crate::config::Capabilities;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The self-description a node gives, as JSON, to `dojo-mesh describe`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Description {
    pub node: String,
    pub capabilities: Capabilities,
    /// This node's screen outputs, when it can tell (a node without a compositor cannot).
    #[serde(default)]
    pub outputs: Option<Vec<String>>,
    #[serde(default)]
    pub boundaries: Vec<crate::config::Boundary>,
    #[serde(default)]
    pub accept_input: Vec<String>,
}

/// Builds this node's description. An attribute named `X-file` is published as `X`, holding the
/// file's contents, so a value that only exists after first start (a public key) is current each
/// time. A file that cannot be read leaves the attribute out. `read` returns a file's text.
pub fn describe(node: &str, declared: &Capabilities, home: Option<&Path>, read: impl Fn(&Path) -> Option<String>) -> Description {
    let mut capabilities = Capabilities::new();
    for (name, attributes) in declared {
        let mut published = BTreeMap::new();
        for (attribute, value) in attributes {
            match attribute.strip_suffix("-file") {
                Some(base) if !base.is_empty() => {
                    if let Some(text) = expand(value, home).and_then(|path| read(&path)) {
                        published.insert(base.to_string(), text.trim().to_string());
                    }
                }
                _ => {
                    published.insert(attribute.clone(), value.clone());
                }
            }
        }
        capabilities.insert(name.clone(), published);
    }
    Description { node: node.into(), capabilities, ..Description::default() }
}

fn expand(value: &str, home: Option<&Path>) -> Option<PathBuf> {
    match value.strip_prefix("~/") {
        Some(rest) => home.map(|home| home.join(rest)),
        None => Some(PathBuf::from(value)).filter(|path| path.is_absolute()),
    }
}

/// One node's offer of a capability.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Offer {
    pub node: String,
    /// The name to connect to.
    pub host: String,
    pub this_node: bool,
    pub attributes: BTreeMap<String, String>,
}

/// The offers of `capability`, this node first, then peers by name. A node that could not be asked
/// is simply absent; the second list says which and why.
pub fn offers(capability: &str, answers: Vec<Answer>) -> (Vec<Offer>, Vec<(String, String)>) {
    let mut found = Vec::new();
    let mut unreachable = Vec::new();
    for answer in answers {
        match answer.description {
            Ok(description) => {
                if let Some(attributes) = description.capabilities.get(capability) {
                    found.push(Offer { node: answer.node, host: answer.host, this_node: answer.this_node, attributes: attributes.clone() });
                }
            }
            Err(reason) => unreachable.push((answer.node, reason)),
        }
    }
    (found, unreachable)
}

pub struct Answer {
    pub node: String,
    pub host: String,
    pub this_node: bool,
    pub description: Result<Description, String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declared() -> Capabilities {
        let mut cache = BTreeMap::new();
        cache.insert("port".to_string(), "5000".to_string());
        cache.insert("public-key-file".to_string(), "~/keys/public".to_string());
        cache.insert("missing-file".to_string(), "/nowhere".to_string());
        let mut all = Capabilities::new();
        all.insert("nix-cache".into(), cache);
        all.insert("voice-agent".into(), BTreeMap::new());
        all
    }

    #[test]
    fn publishes_plain_attributes_and_reads_file_attributes_fresh() {
        let description = describe("alpha", &declared(), Some(Path::new("/home/a")), |path| {
            (path == Path::new("/home/a/keys/public")).then(|| " key-material\n".to_string())
        });
        let cache = &description.capabilities["nix-cache"];
        assert_eq!(cache["port"], "5000");
        assert_eq!(cache["public-key"], "key-material");
        assert!(!cache.contains_key("missing"), "an unreadable file leaves the attribute out");
        assert!(!cache.contains_key("public-key-file"), "the path is not published");
        assert!(description.capabilities["voice-agent"].is_empty());
    }

    #[test]
    fn a_relative_or_homeless_path_is_never_read() {
        let mut attributes = BTreeMap::new();
        attributes.insert("secret-file".to_string(), "relative/path".to_string());
        attributes.insert("other-file".to_string(), "~/x".to_string());
        let mut all = Capabilities::new();
        all.insert("c".into(), attributes);
        let description = describe("alpha", &all, None, |_| panic!("must not read"));
        assert!(description.capabilities["c"].is_empty());
    }

    fn answer(node: &str, this_node: bool, description: Result<Description, String>) -> Answer {
        Answer { node: node.into(), host: format!("{node}.example.ts.net"), this_node, description }
    }

    fn with(node: &str, capability: &str) -> Description {
        let mut capabilities = Capabilities::new();
        capabilities.insert(capability.into(), BTreeMap::from([("port".to_string(), "1".to_string())]));
        Description { node: node.into(), capabilities, ..Description::default() }
    }

    #[test]
    fn offers_lists_who_has_the_capability_and_who_could_not_be_asked() {
        let (found, unreachable) = offers(
            "nix-cache",
            vec![
                answer("alpha", true, Ok(with("alpha", "other"))),
                answer("beta", false, Ok(with("beta", "nix-cache"))),
                answer("gamma", false, Err("timed out".into())),
            ],
        );
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].node, "beta");
        assert_eq!(found[0].host, "beta.example.ts.net");
        assert_eq!(unreachable, [("gamma".to_string(), "timed out".to_string())]);
    }
}
