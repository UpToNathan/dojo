//! The keys a node uses and trusts, by purpose rather than by name or fingerprint. A node *signs with*
//! some keys, tried in order (the first is the primary and the rest are fallbacks), and *trusts* some
//! keys, which are the ones whose signatures on the roster it believes. A key is usually both. A key is
//! described by what it is (a security key, a key in 1Password, a key file), which is what a person
//! knows it by.

use crate::config::MeshConfig;
use crate::enroll;
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    /// A hardware security key: needs a touch.
    SecurityKey,
    /// A key held by 1Password's SSH agent.
    OnePassword,
    /// A key in a file.
    File,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::SecurityKey => "Security key",
            Self::OnePassword => "1Password key",
            Self::File => "Key file",
        }
    }
}

/// One key as the panel shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct KeyRow {
    /// What identifies it in commands: the path it signs with, when it signs.
    pub path: Option<PathBuf>,
    /// What identifies it as a trusted key. Never shown.
    pub fingerprint: String,
    pub kind: Kind,
    /// Its place among the keys this node signs with, from 1; `None` when it does not sign here.
    pub order: Option<usize>,
    pub trusted: bool,
}

fn public_of(path: &Path) -> Option<String> {
    let file = if path.extension().and_then(|ext| ext.to_str()) == Some("pub") { path.to_path_buf() } else { PathBuf::from(format!("{}.pub", path.display())) };
    let text = std::fs::read_to_string(file).ok()?;
    let line = text.lines().map(str::trim).find(|line| !line.is_empty() && !line.starts_with('#'))?;
    crate::signed::normalize_key(line).ok()
}

/// What a key is, as far as its type and where it sits show.
pub fn kind_of(path: &Path, key: &str) -> Kind {
    if key.starts_with("sk-") {
        Kind::SecurityKey
    } else if path.extension().and_then(|ext| ext.to_str()) == Some("pub") {
        Kind::OnePassword
    } else {
        Kind::File
    }
}

/// The signing keys that can sign with nobody present: not a security key, which waits for a touch.
/// What an unattended renewal uses.
pub fn signing_without_touch(config: &MeshConfig) -> Vec<PathBuf> {
    config.signing_keys.iter().filter(|path| public_of(path).is_none_or(|key| kind_of(path, &key) != Kind::SecurityKey)).cloned().collect()
}

/// The keys this node signs with and trusts, signing keys first in their order.
pub fn list(config: &MeshConfig, signers_file: &Path) -> Vec<KeyRow> {
    let trusted = enroll::signers(&std::fs::read_to_string(signers_file).unwrap_or_default());
    let mut rows: Vec<KeyRow> = Vec::new();
    for (index, path) in config.signing_keys.iter().enumerate() {
        let Some(key) = public_of(path) else { continue };
        let Ok(fingerprint) = enroll::fingerprint(&key) else { continue };
        rows.push(KeyRow { path: Some(path.clone()), fingerprint, kind: kind_of(path, &key), order: Some(index + 1), trusted: trusted.contains(&key) });
    }
    for key in &trusted {
        let Ok(fingerprint) = enroll::fingerprint(key) else { continue };
        if rows.iter().any(|row| row.fingerprint == fingerprint) {
            continue;
        }
        rows.push(KeyRow { path: None, fingerprint, kind: if key.starts_with("sk-") { Kind::SecurityKey } else { Kind::File }, order: None, trusted: true });
    }
    rows
}

/// Writes the signing keys into the configuration file, leaving the rest as it is.
pub fn write_signing_keys(config_path: &Path, keys: &[PathBuf]) -> Result<(), String> {
    let text = std::fs::read_to_string(config_path).unwrap_or_default();
    let literal = format!("[{}]", keys.iter().map(|key| format!("\"{}\"", key.display().to_string().replace('\\', "\\\\").replace('"', "\\\""))).collect::<Vec<_>>().join(", "));
    let updated = crate::config::set_top_level(&text, "signing-keys", &literal)?;
    std::fs::write(config_path, updated).map_err(|error| format!("{}: {error}", config_path.display()))
}

/// The list after moving `path` one place `up` or down; `None` when it is not there or cannot move.
pub fn moved(keys: &[PathBuf], path: &Path, up: bool) -> Option<Vec<PathBuf>> {
    let at = keys.iter().position(|key| key == path)?;
    let to = if up { at.checked_sub(1)? } else { (at + 1 < keys.len()).then_some(at + 1)? };
    let mut keys = keys.to_vec();
    keys.swap(at, to);
    Some(keys)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn a_key_moves_one_place_and_cannot_leave_the_ends() {
        let keys = paths(&["/a", "/b", "/c"]);
        assert_eq!(moved(&keys, Path::new("/b"), true), Some(paths(&["/b", "/a", "/c"])));
        assert_eq!(moved(&keys, Path::new("/b"), false), Some(paths(&["/a", "/c", "/b"])));
        assert_eq!(moved(&keys, Path::new("/a"), true), None, "already first");
        assert_eq!(moved(&keys, Path::new("/c"), false), None, "already last");
        assert_eq!(moved(&keys, Path::new("/x"), true), None, "not a signing key");
    }

    #[test]
    fn what_a_key_is_comes_from_its_type_and_where_it_sits() {
        assert_eq!(kind_of(Path::new("/c/signing_yubikey"), "sk-ssh-ed25519@openssh.com AAAA"), Kind::SecurityKey);
        assert_eq!(kind_of(Path::new("/c/signing_1password.pub"), "ssh-ed25519 AAAA"), Kind::OnePassword);
        assert_eq!(kind_of(Path::new("/c/id_ed25519"), "ssh-ed25519 AAAA"), Kind::File);
        assert_eq!(Kind::SecurityKey.label(), "Security key");
    }

    #[test]
    fn signing_keys_are_written_in_place_and_the_rest_of_the_file_is_kept() {
        let dir = std::env::temp_dir().join(format!("dojo-keys-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("mesh.toml");
        std::fs::write(&file, "# mine\nmethod = \"roster\"\nsigning-keys = [\"/a\"]\n\n[capability.x]\nport = 1\n").unwrap();
        write_signing_keys(&file, &paths(&["/b", "/a"])).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert_eq!(text, "# mine\nmethod = \"roster\"\nsigning-keys = [\"/b\", \"/a\"]\n\n[capability.x]\nport = 1\n");
        write_signing_keys(&file, &[]).unwrap();
        assert!(std::fs::read_to_string(&file).unwrap().contains("signing-keys = []"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
