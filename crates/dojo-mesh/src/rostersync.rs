//! Moving a roster between nodes without trusting the carrier. A roster is signed files, so whoever
//! carries them can withhold or garble them but cannot make a node believe something a trusted signing
//! key did not sign. A node therefore takes a bundle of files from any member (or a file on a stick),
//! keeps only the files whose signatures verify against its *own* trusted keys, and for each name keeps
//! the newest. It never deletes: a member is removed by a signed revocation, which is just another file.

use crate::signed::{Signature, Verifier};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A roster file bigger than this is not a roster file (the same bound `load` applies).
const MAX_FILE: usize = 64 * 1024;
const MAX_SIGNATURE: usize = 8 * 1024;
const MAX_FILES: usize = 2000;
/// A bundle bigger than this is refused before it is read.
pub const MAX_BUNDLE: usize = 8 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct Bundle {
    v: u32,
    files: Vec<BundleFile>,
}

#[derive(Serialize, Deserialize)]
struct BundleFile {
    /// `devices/NAME.json` or `revocations/NAME.json`, relative to the roster.
    path: String,
    payload: String,
    signature: String,
}

/// What a merge did, by path.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Merged {
    pub added: Vec<String>,
    pub updated: Vec<String>,
    pub unchanged: usize,
    /// Files not taken, with the reason (no good signature, older than what is here, malformed).
    pub refused: Vec<(String, String)>,
}

impl Merged {
    pub fn changed(&self) -> bool {
        !self.added.is_empty() || !self.updated.is_empty()
    }

    /// Files that arrived claiming a signature they do not have: someone altered them or signed with a
    /// key this node does not trust. An older entry or an expired one is ordinary and is not counted.
    pub fn suspicious(&self) -> Vec<&str> {
        self.refused.iter().filter(|(_, reason)| reason.starts_with("not signed")).map(|(path, _)| path.as_str()).collect()
    }
}

/// A path a roster may hold: one of its two folders, then a plain file name ending `.json`.
fn valid_path(path: &str) -> bool {
    let Some((folder, file)) = path.split_once('/') else { return false };
    let Some(stem) = file.strip_suffix(".json") else { return false };
    matches!(folder, "devices" | "revocations") && !stem.is_empty() && stem.len() <= 120 && stem.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn issued(payload: &str) -> Option<DateTime<Utc>> {
    let value: serde_json::Value = serde_json::from_str(payload).ok()?;
    DateTime::parse_from_rfc3339(value.get("issued")?.as_str()?).ok().map(|time| time.with_timezone(&Utc))
}

/// Every roster file with its signature, as one JSON document. Nothing is checked here: the node that
/// receives it checks everything.
pub fn export(dir: &Path) -> Result<String, String> {
    let mut files = Vec::new();
    for folder in ["devices", "revocations"] {
        let entries = match std::fs::read_dir(dir.join(folder)) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("{}: {error}", dir.join(folder).display())),
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let path = format!("{folder}/{name}");
            if !valid_path(&path) || !entry.file_type().is_ok_and(|kind| kind.is_file()) {
                continue;
            }
            let signature = PathBuf::from(format!("{}.sig", entry.path().display()));
            let (Ok(payload), Ok(signature)) = (std::fs::read_to_string(entry.path()), std::fs::read_to_string(&signature)) else { continue };
            if payload.len() <= MAX_FILE && signature.len() <= MAX_SIGNATURE {
                files.push(BundleFile { path, payload, signature });
            }
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    serde_json::to_string(&Bundle { v: 1, files }).map_err(|error| error.to_string())
}

fn write_atomically(path: &Path, text: &str) -> Result<(), String> {
    let parent = path.parent().ok_or("a roster file has a folder")?;
    std::fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
    let temporary = parent.join(format!(".incoming-{}", std::process::id()));
    std::fs::write(&temporary, text).map_err(|error| format!("{}: {error}", temporary.display()))?;
    std::fs::rename(&temporary, path).map_err(|error| format!("{}: {error}", path.display()))
}

/// Takes the files in `bundle` that verify under `verifier` and are new or newer than what `dir` has.
pub fn merge(dir: &Path, bundle: &str, verifier: &dyn Verifier) -> Result<Merged, String> {
    if bundle.len() > MAX_BUNDLE {
        return Err(format!("the bundle is larger than {} MiB, which no roster is", MAX_BUNDLE / 1024 / 1024));
    }
    let bundle: Bundle = serde_json::from_str(bundle).map_err(|error| format!("that is not a roster bundle: {error}"))?;
    if bundle.v != 1 {
        return Err(format!("bundle version {} is not understood (this node reads version 1)", bundle.v));
    }
    if bundle.files.len() > MAX_FILES {
        return Err(format!("the bundle has more than {MAX_FILES} files"));
    }
    let scratch = dir.join(format!(".verify-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).map_err(|error| format!("{}: {error}", scratch.display()))?;
    let mut merged = Merged::default();
    for file in bundle.files {
        let refuse = |merged: &mut Merged, reason: String| merged.refused.push((file.path.clone(), reason));
        if !valid_path(&file.path) {
            refuse(&mut merged, "not a path a roster holds".into());
            continue;
        }
        if file.payload.len() > MAX_FILE || file.signature.len() > MAX_SIGNATURE {
            refuse(&mut merged, "larger than a roster file can be".into());
            continue;
        }
        let signature_path = scratch.join("signature");
        std::fs::write(&signature_path, &file.signature).map_err(|error| format!("{}: {error}", signature_path.display()))?;
        if let Err(reason) = verifier.verify(file.payload.as_bytes(), &Signature { path: signature_path }) {
            refuse(&mut merged, format!("not signed by a trusted signing key ({reason})"));
            continue;
        }
        let Some(new_issue) = issued(&file.payload) else {
            refuse(&mut merged, "it has no issue time".into());
            continue;
        };
        let target = dir.join(&file.path);
        let sig_target = PathBuf::from(format!("{}.sig", target.display()));
        let existing = std::fs::read_to_string(&target).ok();
        match existing {
            Some(old) if old == file.payload => {
                if std::fs::read_to_string(&sig_target).is_err() {
                    write_atomically(&sig_target, &file.signature)?;
                }
                merged.unchanged += 1;
            }
            Some(old) if issued(&old).is_some_and(|old_issue| old_issue >= new_issue) => refuse(&mut merged, "older than the file this node already has".into()),
            Some(_) => {
                write_atomically(&sig_target, &file.signature)?;
                write_atomically(&target, &file.payload)?;
                merged.updated.push(file.path);
            }
            None => {
                write_atomically(&sig_target, &file.signature)?;
                write_atomically(&target, &file.payload)?;
                merged.added.push(file.path);
            }
        }
    }
    let _ = std::fs::remove_dir_all(&scratch);
    Ok(merged)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A signature is good when it says so; stands in for ssh-keygen.
    struct Says;
    impl Verifier for Says {
        fn verify(&self, _: &[u8], signature: &Signature) -> Result<(), String> {
            if std::fs::read_to_string(&signature.path).unwrap_or_default().trim() == "good" { Ok(()) } else { Err("no trusted signing key signed it".into()) }
        }
    }

    fn lab(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dojo-rostersync-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("devices")).unwrap();
        dir
    }

    fn put(dir: &Path, path: &str, issued: &str, sig: &str) {
        std::fs::write(dir.join(path), format!("{{\"v\":1,\"issued\":\"{issued}\"}}")).unwrap();
        std::fs::write(dir.join(format!("{path}.sig")), sig).unwrap();
    }

    #[test]
    fn a_verified_file_is_added_and_the_same_file_again_changes_nothing() {
        let (from, to) = (lab("a1"), lab("a2"));
        put(&from, "devices/orchard.json", "2026-10-01T00:00:00Z", "good");
        let bundle = export(&from).unwrap();
        let first = merge(&to, &bundle, &Says).unwrap();
        assert_eq!(first.added, ["devices/orchard.json"]);
        assert!(to.join("devices/orchard.json.sig").exists());
        let again = merge(&to, &bundle, &Says).unwrap();
        assert!(!again.changed() && again.unchanged == 1, "{again:?}");
        assert!(!to.join(format!(".verify-{}", std::process::id())).exists(), "scratch is cleaned up");
    }

    #[test]
    fn a_file_without_a_good_signature_is_never_written_and_is_called_out() {
        let (from, to) = (lab("b1"), lab("b2"));
        put(&from, "devices/orchard.json", "2026-10-01T00:00:00Z", "forged");
        let merged = merge(&to, &export(&from).unwrap(), &Says).unwrap();
        assert!(!merged.changed());
        assert!(!to.join("devices/orchard.json").exists());
        assert_eq!(merged.suspicious(), ["devices/orchard.json"]);
    }

    #[test]
    fn newer_replaces_older_and_older_never_replaces_newer() {
        let (from, to) = (lab("c1"), lab("c2"));
        put(&to, "devices/orchard.json", "2026-10-02T00:00:00Z", "good");
        put(&from, "devices/orchard.json", "2026-10-01T00:00:00Z", "good");
        let merged = merge(&to, &export(&from).unwrap(), &Says).unwrap();
        assert!(!merged.changed());
        assert!(merged.refused[0].1.contains("older"), "{merged:?}");
        assert!(merged.suspicious().is_empty(), "an old entry is ordinary");
        put(&from, "devices/orchard.json", "2026-10-03T00:00:00Z", "good");
        let merged = merge(&to, &export(&from).unwrap(), &Says).unwrap();
        assert_eq!(merged.updated, ["devices/orchard.json"]);
        assert!(std::fs::read_to_string(to.join("devices/orchard.json")).unwrap().contains("2026-10-03"));
    }

    #[test]
    fn nothing_is_ever_deleted_by_a_merge() {
        let (from, to) = (lab("d1"), lab("d2"));
        put(&to, "devices/keeper.json", "2026-10-01T00:00:00Z", "good");
        put(&from, "devices/other.json", "2026-10-01T00:00:00Z", "good");
        merge(&to, &export(&from).unwrap(), &Says).unwrap();
        assert!(to.join("devices/keeper.json").exists() && to.join("devices/other.json").exists());
    }

    #[test]
    fn paths_that_could_leave_the_roster_and_oversized_or_foreign_bundles_are_refused() {
        let to = lab("e1");
        for path in ["../escape.json", "devices/../../x.json", "/etc/passwd", "devices/a/b.json", "other/x.json", "devices/.json", "devices/x.txt", "devices/a b.json"] {
            assert!(!valid_path(path), "{path}");
        }
        for path in ["devices/orchard.json", "revocations/orchard-node-ab12.json"] {
            assert!(valid_path(path), "{path}");
        }
        let bundle = serde_json::json!({"v": 1, "files": [{"path": "../escape.json", "payload": "{\"issued\":\"2026-10-01T00:00:00Z\"}", "signature": "good"}]}).to_string();
        let merged = merge(&to, &bundle, &Says).unwrap();
        assert!(!merged.changed() && merged.refused.len() == 1);
        assert!(!to.parent().unwrap().join("escape.json").exists());
        assert!(merge(&to, "not json", &Says).is_err());
        assert!(merge(&to, "{\"v\":2,\"files\":[]}", &Says).is_err());
        assert!(merge(&to, &" ".repeat(MAX_BUNDLE + 1), &Says).is_err());
    }
}
