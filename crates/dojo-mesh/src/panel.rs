//! What the shell's mesh panel draws, read in one go and computed nowhere else: a card per machine,
//! the health findings `doctor` would print, whether the key holder is ready, the signing keys this
//! node trusts and the join requests waiting for it. The panel changes nothing itself; every change is
//! the `dojo-mesh` command that does it, so the terminal and the panel cannot disagree.

use crate::cli::{self, Env};
use crate::config::{MeshConfig, MethodChoice, RosterPaths, roster_paths};
use crate::doctor::{Finding, Severity, diagnose};
use crate::enroll::{self, JoinRequest};
use crate::roster::Roster;
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// An entry this close to expiring is shown as needing attention.
const EXPIRING_DAYS: i64 = 14;
/// A join request is a few hundred bytes; this is the most one may be.
const MAX_REQUEST_FILE: u64 = 16 * 1024;
/// The most files looked at in one folder, so a folder full of other things costs nothing.
const MAX_SCANNED: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Tailscale,
    Roster,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Card {
    pub name: String,
    pub this: bool,
    pub online: bool,
    /// Whole days until the roster entry expires, rounded down; negative once it has. `None` for a method whose
    /// members do not expire this way.
    pub days_left: Option<i64>,
    pub expiring: bool,
    /// Where it is reached, as the first route shows it.
    pub route: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Signer {
    pub fingerprint: String,
    pub hardware: bool,
}

/// A join request that was found, whose signature verifies against its own node key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Request {
    pub name: String,
    /// What the new machine printed for its node key, for the person to compare.
    pub fingerprint: String,
    pub routes: Vec<String>,
    pub file: PathBuf,
    /// The roster already holds a different machine under this name: approving it needs `--replace`
    /// and the person should know.
    pub replaces: bool,
}

/// A public key found in the inbox that could be trusted as a signing key, once the person has
/// compared its fingerprint with the machine that holds it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Offer {
    pub fingerprint: String,
    pub hardware: bool,
    pub file: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Panel {
    pub kind: Kind,
    pub this: String,
    pub cards: Vec<Card>,
    pub findings: Vec<Finding>,
    /// Why peers cannot be reached from here yet (1Password locked or off), when that is so.
    pub blocked: Option<String>,
    /// Whether this node has a signing key to sign with.
    pub can_sign: bool,
    pub signers: Vec<Signer>,
    pub requests: Vec<Request>,
    /// Public keys waiting in the inbox that are not trusted yet.
    pub offers: Vec<Offer>,
    /// Files in the inbox folders that look like requests but were not taken (unreadable, altered or
    /// not signed by their own key).
    pub skipped: usize,
}

/// A card per member, this machine first.
pub fn cards(roster: &Roster, now: DateTime<Utc>) -> Vec<Card> {
    roster
        .members
        .iter()
        .enumerate()
        .map(|(index, member)| {
            let device = &member.device;
            // Floored, so an entry that expired an hour ago reads -1 and not "0 days left".
            let days_left = device.key_expiry.map(|expiry| (expiry - now).num_seconds().div_euclid(86_400));
            Card {
                name: device.name.clone(),
                this: index == 0,
                online: device.online,
                days_left,
                expiring: days_left.is_some_and(|days| days <= EXPIRING_DAYS),
                route: device.routes.first().map(|route| match route {
                    crate::method::Route::Host { name, port: Some(port) } => format!("{name}:{port}"),
                    crate::method::Route::Host { name, port: None } => name.clone(),
                    crate::method::Route::Command { .. } => "via a command".to_string(),
                }),
            }
        })
        .collect()
}

/// The join requests in `folders` that can be taken: regular files named `*.join.json`, small, that
/// parse, whose signature verifies against their own node key, and that are not already members
/// under the same key. `members` maps a name to the node key the roster holds for it.
pub fn scan_requests(folders: &[PathBuf], members: &BTreeMap<String, String>) -> (Vec<Request>, usize) {
    let mut found: Vec<Request> = Vec::new();
    let mut skipped = 0;
    for folder in folders {
        let Ok(entries) = std::fs::read_dir(folder) else { continue };
        for entry in entries.flatten().take(MAX_SCANNED) {
            let path = entry.path();
            if !path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.ends_with(".join.json")) {
                continue;
            }
            // A link could point anywhere, and a big file is not a request.
            let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
            if !meta.is_file() || meta.len() > MAX_REQUEST_FILE {
                skipped += 1;
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                skipped += 1;
                continue;
            };
            let Ok(request) = JoinRequest::parse(&text) else {
                skipped += 1;
                continue;
            };
            if request.verify().is_err() {
                skipped += 1;
                continue;
            }
            let known = members.get(&request.name);
            if known == Some(&request.node_key) {
                continue; // already a member under this very key
            }
            let Ok(fingerprint) = enroll::fingerprint(&request.node_key) else {
                skipped += 1;
                continue;
            };
            if found.iter().any(|other| other.name == request.name && other.fingerprint == fingerprint) {
                continue;
            }
            found.push(Request {
                name: request.name.clone(),
                fingerprint,
                routes: request.routes.iter().map(|route| match route.port { Some(port) => format!("{}:{port}", route.host), None => route.host.clone() }).collect(),
                file: path,
                replaces: known.is_some(),
            });
        }
    }
    found.sort_by(|a, b| a.name.cmp(&b.name));
    (found, skipped)
}

/// The public keys in `folders` (`*.pub`, small, plain files) that are not in `trusted`.
pub fn scan_offers(folders: &[PathBuf], trusted: &[String]) -> Vec<Offer> {
    let mut found: Vec<Offer> = Vec::new();
    for folder in folders {
        let Ok(entries) = std::fs::read_dir(folder) else { continue };
        for entry in entries.flatten().take(MAX_SCANNED) {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("pub") {
                continue;
            }
            let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
            if !meta.is_file() || meta.len() > 4096 {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            let Some(line) = text.lines().map(str::trim).find(|line| !line.is_empty() && !line.starts_with('#')) else { continue };
            let Ok(key) = crate::signed::normalize_key(line) else { continue };
            if trusted.contains(&key) {
                continue;
            }
            let Ok(fingerprint) = enroll::fingerprint(&key) else { continue };
            if found.iter().any(|other| other.fingerprint == fingerprint) {
                continue;
            }
            found.push(Offer { fingerprint, hardware: key.starts_with("sk-"), file: path });
        }
    }
    found.sort_by(|a, b| a.fingerprint.cmp(&b.fingerprint));
    found
}

/// Where requests wait: the mesh's own inbox, and the user's downloads (where a file handed over by
/// a browser or a file share usually lands).
pub fn inbox_folders(paths: &RosterPaths, home: Option<&Path>) -> Vec<PathBuf> {
    let mut folders = Vec::new();
    if let Some(base) = paths.known_hosts.parent() {
        folders.push(base.join("inbox"));
    }
    if let Some(home) = home {
        folders.push(home.join("Downloads"));
    }
    folders
}

fn signers(paths: &RosterPaths) -> Vec<Signer> {
    enroll::signers(&std::fs::read_to_string(&paths.signers).unwrap_or_default())
        .into_iter()
        .filter_map(|key| Some(Signer { fingerprint: enroll::fingerprint(&key).ok()?, hardware: key.starts_with("sk-") }))
        .collect()
}

/// The panel's state for this node now.
pub fn read() -> Panel {
    match Env::from_process(None) {
        Ok(env) => {
            let config = cli::load_config(&env).unwrap_or_default();
            read_with(&env, &config)
        }
        Err(_) => Panel { kind: Kind::Tailscale, this: String::new(), cards: Vec::new(), findings: Vec::new(), blocked: None, can_sign: false, signers: Vec::new(), requests: Vec::new(), offers: Vec::new(), skipped: 0 },
    }
}

pub fn read_with(env: &Env, config: &MeshConfig) -> Panel {
    let now = Utc::now();
    let roster = cli::load_roster(env, config);
    let var = |name: &str| std::env::var_os(name).map(PathBuf::from);
    let paths = roster_paths(config, var("XDG_DATA_HOME"), var("XDG_CONFIG_HOME"), env.home.clone());
    let roster_method = config.method == MethodChoice::Roster;
    let mut findings = diagnose(&roster, config, now);
    if roster_method {
        findings.extend(crate::enroll_cli::custody_findings(env, config));
    }
    let blocked = env.method.channel.blocked();
    if let Some(step) = &blocked {
        findings.push(Finding { severity: Severity::Warning, device: None, message: format!("Peers cannot be reached from here yet: {step}.") });
    }
    let (signers, requests, offers, skipped) = match (&paths, roster_method) {
        (Some(paths), true) => {
            let members: BTreeMap<String, String> = roster.members.iter().filter_map(|m| m.device.keys.as_ref().map(|keys| (m.device.name.clone(), keys.node_key.clone()))).collect();
            let folders = inbox_folders(paths, env.home.as_deref());
            let (requests, skipped) = scan_requests(&folders, &members);
            let trusted = enroll::signers(&std::fs::read_to_string(&paths.signers).unwrap_or_default());
            (signers(paths), requests, scan_offers(&folders, &trusted), skipped)
        }
        _ => (Vec::new(), Vec::new(), Vec::new(), 0),
    };
    Panel {
        kind: if roster_method { Kind::Roster } else { Kind::Tailscale },
        this: roster.this().name.clone(),
        cards: cards(&roster, now),
        findings,
        blocked,
        can_sign: roster_method && !config.signing_keys.is_empty(),
        signers,
        requests,
        offers,
        skipped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::{Device, Keys};
    use crate::roster::{Admission, Member};
    use crate::signed::RouteSpec;
    use std::process::Command;

    fn device(name: &str, online: bool, expires_in_days: Option<i64>) -> Device {
        Device {
            name: name.into(),
            dns_name: None,
            tags: Vec::new(),
            online,
            key_expiry: expires_in_days.map(|days| Utc::now() + chrono::Duration::days(days) + chrono::Duration::hours(1)),
            key_expired: false,
            routes: vec![crate::method::Route::Host { name: "192.0.2.7".into(), port: Some(2222) }],
            keys: None,
        }
    }

    fn roster(devices: Vec<Device>) -> Roster {
        let mut members = devices.into_iter().enumerate().map(|(index, device)| Member { device, admitted_by: if index == 0 { Admission::This } else { Admission::Signed } });
        let this = members.next().unwrap();
        Roster { source: crate::roster::Source::Method("roster".into()), membership_tag: None, members: std::iter::once(this).chain(members).collect(), excluded: Vec::new(), rejected: Vec::new() }
    }

    #[test]
    fn a_card_per_machine_with_this_one_first_days_left_and_where_it_is_reached() {
        let cards = cards(&roster(vec![device("alpha", true, Some(80)), device("beta", false, Some(5)), device("gone", true, Some(-3))]), Utc::now());
        assert_eq!(cards.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["alpha", "beta", "gone"]);
        assert!(cards[0].this && !cards[1].this);
        assert_eq!((cards[0].days_left, cards[0].expiring), (Some(80), false));
        assert_eq!((cards[1].days_left, cards[1].expiring, cards[1].online), (Some(5), true, false));
        assert_eq!(cards[2].days_left, Some(-3), "an expired entry counts down past zero");
        assert!(cards[2].expiring);
        assert_eq!(cards[0].route.as_deref(), Some("192.0.2.7:2222"));
    }

    #[test]
    fn a_method_without_expiry_shows_none() {
        let cards = cards(&roster(vec![device("alpha", true, None), device("beta", true, None)]), Utc::now());
        assert!(cards.iter().all(|c| c.days_left.is_none() && !c.expiring));
    }

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dojo-panel-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A request signed by a fresh key pair, written as `file` in `folder`; returns it and the key.
    fn request_in(folder: &Path, name: &str, file: &str) -> (JoinRequest, String) {
        let private = folder.join(format!("{name}-{file}-key"));
        let status = Command::new("ssh-keygen").args(["-q", "-t", "ed25519", "-N", "", "-f"]).arg(&private).status().unwrap();
        assert!(status.success());
        let public = std::fs::read_to_string(format!("{}.pub", private.display())).unwrap().split_whitespace().take(2).collect::<Vec<_>>().join(" ");
        let host = public.clone();
        let mut request = JoinRequest { v: 2, name: name.into(), node_key: public.clone(), input_key: None, host_key: host, routes: vec![RouteSpec { host: "192.0.2.9".into(), port: Some(2222) }], signature: String::new() };
        request.signature = enroll::sign_bytes(&request.body(), Path::new(&format!("{}.pub", private.display())), enroll::JOIN_NAMESPACE, None).unwrap();
        std::fs::write(folder.join(file), serde_json::to_string_pretty(&request).unwrap()).unwrap();
        (request, public)
    }

    #[test]
    fn only_requests_whose_signature_verifies_are_listed_with_what_to_compare() {
        let folder = dir("scan");
        let (good, _) = request_in(&folder, "orchard", "orchard.join.json");
        // Altered after signing: the host key swapped.
        let (mut altered, _) = request_in(&folder, "mallory", "mallory.join.json");
        altered.routes[0].host = "198.51.100.1".into();
        std::fs::write(folder.join("mallory.join.json"), serde_json::to_string(&altered).unwrap()).unwrap();
        // Not requests at all.
        std::fs::write(folder.join("notes.txt"), "hello").unwrap();
        std::fs::write(folder.join("broken.join.json"), "{").unwrap();
        std::fs::write(folder.join("big.join.json"), vec![b' '; (MAX_REQUEST_FILE + 1) as usize]).unwrap();
        std::os::unix::fs::symlink("/etc/passwd", folder.join("link.join.json")).unwrap();

        let (found, skipped) = scan_requests(&[folder.clone(), folder.join("absent")], &BTreeMap::new());
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].name, "orchard");
        assert_eq!(found[0].fingerprint, enroll::fingerprint(&good.node_key).unwrap());
        assert_eq!(found[0].routes, ["192.0.2.9:2222"]);
        assert!(!found[0].replaces);
        assert_eq!(skipped, 4, "altered, broken, oversized and a link");
        let _ = std::fs::remove_dir_all(folder);
    }

    #[test]
    fn a_request_from_a_machine_that_is_already_a_member_is_not_pending_and_a_clash_is_flagged() {
        let folder = dir("known");
        let (member, _) = request_in(&folder, "orchard", "orchard.join.json");
        let (_, _) = request_in(&folder, "beta", "beta.join.json");
        let mut members = BTreeMap::new();
        members.insert("orchard".to_string(), member.node_key.clone());
        members.insert("beta".to_string(), "ssh-ed25519 AAAAsomeoneelse".to_string());
        let (found, _) = scan_requests(&[folder.clone()], &members);
        assert_eq!(found.iter().map(|r| (r.name.as_str(), r.replaces)).collect::<Vec<_>>(), [("beta", true)], "orchard is a member already; beta's name is held by another key");
        let _ = std::fs::remove_dir_all(folder);
    }

    #[test]
    fn the_same_request_in_two_folders_is_listed_once() {
        let (one, two) = (dir("two-a"), dir("two-b"));
        let (request, _) = request_in(&one, "orchard", "orchard.join.json");
        std::fs::write(two.join("orchard.join.json"), serde_json::to_string_pretty(&request).unwrap()).unwrap();
        let (found, _) = scan_requests(&[one.clone(), two.clone()], &BTreeMap::new());
        assert_eq!(found.len(), 1);
        let _ = (std::fs::remove_dir_all(one), std::fs::remove_dir_all(two));
    }

    #[test]
    fn public_keys_in_the_inbox_are_offered_unless_already_trusted_and_only_if_they_are_keys() {
        let folder = dir("offers");
        let make = |name: &str| -> String {
            let private = folder.join(name);
            assert!(Command::new("ssh-keygen").args(["-q", "-t", "ed25519", "-N", "", "-f"]).arg(&private).status().unwrap().success());
            std::fs::read_to_string(format!("{}.pub", private.display())).unwrap().split_whitespace().take(2).collect::<Vec<_>>().join(" ")
        };
        let (new, known) = (make("new"), make("known"));
        // Not keys: an option-laden line and a block of text.
        std::fs::write(folder.join("evil.pub"), "command=\"rm -rf ~\" ssh-ed25519 AAAAC3Nza").unwrap();
        std::fs::write(folder.join("notes.pub"), "just words\n").unwrap();
        std::fs::remove_file(folder.join("new")).unwrap();
        std::fs::remove_file(folder.join("known")).unwrap();
        let offers = scan_offers(&[folder.clone()], &[known]);
        assert_eq!(offers.len(), 1, "{offers:?}");
        assert_eq!(offers[0].fingerprint, enroll::fingerprint(&new).unwrap());
        assert!(!offers[0].hardware);
        let _ = std::fs::remove_dir_all(folder);
    }

    #[test]
    fn requests_wait_in_the_mesh_inbox_and_downloads() {
        let paths = RosterPaths {
            dir: "/d/roster".into(),
            signers: "/c/allowed_signers".into(),
            node_key: "/c/node.pub".into(),
            input_key: "/c/input.pub".into(),
            known_hosts: "/data/dojo/mesh/known_hosts".into(),
            agent: None,
        };
        assert_eq!(inbox_folders(&paths, Some(Path::new("/home/pat"))), [PathBuf::from("/data/dojo/mesh/inbox"), PathBuf::from("/home/pat/Downloads")]);
        let _ = Keys { node_key: String::new(), input_key: None, host_key: String::new() };
    }
}
