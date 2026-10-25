//! A signed roster: the members of a mesh, each entry signed by an operator signing key. This is the
//! pure part. It decides which signed statements count (signed, current, not revoked, well formed)
//! given a verifier that answers whether a signature is good; reading files and running
//! `ssh-keygen` are in `openssh`.

use crate::method::Route;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// What a signature is made for, so a signature from another use of the same key never counts.
pub const NAMESPACE: &str = "dojo-mesh-roster";

/// Entries issued further in the future than this are refused: the signer's clock is wrong, or the
/// entry was made to start later than its signer says.
const CLOCK_SKEW: Duration = Duration::minutes(5);

/// The key types a roster may carry. Anything else (including options smuggled before a key) is refused,
/// because these keys end up in `authorized_keys` lines.
const KEY_TYPES: [&str; 3] = ["ssh-ed25519", "sk-ssh-ed25519@openssh.com", "ecdsa-sha2-nistp256"];

/// Names that would be ambiguous in a command (`dojo-mesh exec all ...`).
const RESERVED: [&str; 5] = ["all", "self", "this", "local", "localhost"];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteSpec {
    pub host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
}

/// One member, as its signer vouched for it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceEntry {
    pub name: String,
    /// The key this node connects with: `type base64`, no comment.
    pub node_key: String,
    /// The key restricted to remote input, when the node has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_key: Option<String>,
    /// The node's SSH server key, pinned for connections to it.
    pub host_key: String,
    #[serde(default)]
    pub routes: Vec<RouteSpec>,
    pub issued: DateTime<Utc>,
    pub expires: DateTime<Utc>,
}

/// A key the signer withdrew trust from. Any device entry carrying it is refused.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Revocation {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub issued: DateTime<Utc>,
}

/// The two things a roster directory holds, told apart by `kind`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Statement {
    Device(DeviceEntry),
    Revoke(Revocation),
}

/// The only version understood. A newer one is refused rather than half read.
const VERSION: u32 = 1;

#[derive(Deserialize)]
struct Envelope {
    v: u32,
}

/// A file as found, before anything is believed about it.
#[derive(Clone, Debug)]
pub struct Candidate {
    /// What to call it in a message (its path relative to the roster).
    pub label: String,
    pub payload: Vec<u8>,
    /// The signature text, or `None` when the file has no signature beside it.
    pub signature: Option<Signature>,
}

#[derive(Clone, Debug)]
pub struct Signature {
    /// Where the signature file is, for a verifier that needs a path.
    pub path: std::path::PathBuf,
}

/// Says whether a signature is good for a payload under the node's trusted signing keys.
pub trait Verifier {
    /// `Err` carries the verifier's reason: wrong key, unknown signer, altered payload.
    fn verify(&self, payload: &[u8], signature: &Signature) -> Result<(), String>;
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Rejected {
    pub label: String,
    pub reason: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Verified {
    pub devices: Vec<DeviceEntry>,
    pub rejected: Vec<Rejected>,
}

fn reject(rejected: &mut Vec<Rejected>, label: &str, reason: impl Into<String>) {
    rejected.push(Rejected { label: label.to_string(), reason: reason.into() });
}

/// A name a member may have: lowercase letters, digits and dashes, at least two characters, not
/// starting or ending with a dash, and not a word the commands use.
pub fn valid_name(name: &str) -> Result<(), String> {
    let ok = name.len() >= 2
        && name.len() <= 63
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !name.starts_with('-')
        && !name.ends_with('-');
    if !ok {
        return Err(format!("{name:?} is not a valid node name (2 to 63 lowercase letters, digits or dashes)"));
    }
    if RESERVED.contains(&name) {
        return Err(format!("{name:?} is reserved"));
    }
    Ok(())
}

/// `type base64` from a public key, with any comment dropped. Refuses anything that is not exactly
/// one key of a known type: options, several lines, odd characters.
pub fn normalize_key(text: &str) -> Result<String, String> {
    if text.contains(['\n', '\r', '\0']) {
        return Err("a key must be one line".into());
    }
    let mut parts = text.split_whitespace();
    let (Some(kind), Some(body)) = (parts.next(), parts.next()) else { return Err("a key is its type followed by its base64 body".into()) };
    if !KEY_TYPES.contains(&kind) {
        return Err(format!("{kind:?} is not a key type the roster accepts"));
    }
    if body.is_empty() || !body.chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=') {
        return Err("the key body is not base64".into());
    }
    Ok(format!("{kind} {body}"))
}

fn valid_host(host: &str) -> bool {
    !host.is_empty() && !host.starts_with('-') && host.len() <= 253 && host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':'))
}

/// Checks one device entry's own contents and returns it with its keys normalized.
pub(crate) fn well_formed(mut entry: DeviceEntry) -> Result<DeviceEntry, String> {
    valid_name(&entry.name)?;
    entry.node_key = normalize_key(&entry.node_key).map_err(|error| format!("node key: {error}"))?;
    entry.host_key = normalize_key(&entry.host_key).map_err(|error| format!("host key: {error}"))?;
    if let Some(key) = &entry.input_key {
        entry.input_key = Some(normalize_key(key).map_err(|error| format!("input key: {error}"))?);
    }
    for route in &entry.routes {
        if !valid_host(&route.host) {
            return Err(format!("route host {:?} is not a plain name or address", route.host));
        }
        if route.port == Some(0) {
            return Err("route port 0 is not a port".into());
        }
    }
    if entry.expires <= entry.issued {
        return Err("it expires before it was issued".into());
    }
    Ok(entry)
}

/// Decides which candidates count. Order of checks, so the reason shown is the first real problem:
/// no signature, signature not good, unreadable, malformed, expired or not yet valid, revoked,
/// superseded.
pub fn evaluate(candidates: Vec<Candidate>, verifier: &dyn Verifier, now: DateTime<Utc>) -> Verified {
    let mut rejected = Vec::new();
    let mut devices: Vec<(String, DeviceEntry)> = Vec::new();
    let mut revoked: BTreeSet<String> = BTreeSet::new();

    for candidate in candidates {
        let label = candidate.label.as_str();
        let Some(signature) = &candidate.signature else {
            reject(&mut rejected, label, "unsigned: there is no signature beside it");
            continue;
        };
        if let Err(reason) = verifier.verify(&candidate.payload, signature) {
            reject(&mut rejected, label, format!("not signed by a trusted signing key ({reason})"));
            continue;
        }
        match serde_json::from_slice::<Envelope>(&candidate.payload) {
            Ok(envelope) if envelope.v != VERSION => {
                reject(&mut rejected, label, format!("version {} is not understood (this node reads version {VERSION})", envelope.v));
                continue;
            }
            Err(error) => {
                reject(&mut rejected, label, format!("unreadable: {error}"));
                continue;
            }
            Ok(_) => {}
        }
        match serde_json::from_slice::<Statement>(&candidate.payload) {
            Err(error) => reject(&mut rejected, label, format!("unreadable: {error}")),
            Ok(Statement::Revoke(revocation)) => match normalize_key(&revocation.key) {
                Ok(key) => {
                    revoked.insert(key);
                }
                Err(error) => reject(&mut rejected, label, format!("revocation: {error}")),
            },
            Ok(Statement::Device(entry)) => match well_formed(entry) {
                Ok(entry) => devices.push((candidate.label.clone(), entry)),
                Err(reason) => reject(&mut rejected, label, reason),
            },
        }
    }

    let mut current: BTreeMap<String, (String, DeviceEntry)> = BTreeMap::new();
    for (label, entry) in devices {
        if entry.expires <= now {
            reject(&mut rejected, &label, format!("expired on {}", entry.expires.format("%Y-%m-%d")));
            continue;
        }
        if entry.issued > now + CLOCK_SKEW {
            reject(&mut rejected, &label, format!("issued in the future ({}); check the signer's clock", entry.issued.format("%Y-%m-%d %H:%M UTC")));
            continue;
        }
        let keys = [Some(&entry.node_key), entry.input_key.as_ref(), Some(&entry.host_key)];
        if let Some(key) = keys.into_iter().flatten().find(|key| revoked.contains(*key)) {
            reject(&mut rejected, &label, format!("revoked: its key {} was withdrawn", short(key)));
            continue;
        }
        match current.get(&entry.name) {
            Some((older_label, older)) if older.issued >= entry.issued => reject(&mut rejected, &label, format!("superseded by the newer entry in {older_label}")),
            Some((older_label, _)) => {
                let older_label = older_label.clone();
                reject(&mut rejected, &older_label, format!("superseded by the newer entry in {label}"));
                current.insert(entry.name.clone(), (label, entry));
            }
            None => {
                current.insert(entry.name.clone(), (label, entry));
            }
        }
    }
    rejected.sort_by(|a, b| a.label.cmp(&b.label));
    Verified { devices: current.into_values().map(|(_, entry)| entry).collect(), rejected }
}

/// The start of a key's body, enough to tell keys apart in a message.
fn short(key: &str) -> String {
    let body = key.split_whitespace().nth(1).unwrap_or(key);
    format!("…{}", &body[body.len().saturating_sub(10)..])
}

impl RouteSpec {
    pub fn to_route(&self) -> Route {
        Route::Host { name: self.host.clone(), port: self.port }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::path::PathBuf;

    const KEY_A: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA alpha-comment";
    const KEY_B: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";
    const HOST: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHHHHHHHHHHHHHHHHHHHHHHHHHHHHHHHHHHHHHHHHHHHH";

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 25, 12, 0, 0).unwrap()
    }

    /// A verifier that trusts payloads containing the word "TRUSTED" in their signature file name.
    struct ByName;

    impl Verifier for ByName {
        fn verify(&self, _: &[u8], signature: &Signature) -> Result<(), String> {
            if signature.path.to_string_lossy().contains("trusted") { Ok(()) } else { Err("unknown signer".into()) }
        }
    }

    fn entry(name: &str, node_key: &str, issued_days_ago: i64, valid_days: i64) -> String {
        serde_json::json!({
            "v": 1, "kind": "device", "name": name, "node_key": node_key, "host_key": HOST,
            "routes": [{"host": "100.64.0.2", "port": 22}],
            "issued": (now() - Duration::days(issued_days_ago)).to_rfc3339(),
            "expires": (now() - Duration::days(issued_days_ago) + Duration::days(valid_days)).to_rfc3339(),
        })
        .to_string()
    }

    fn revoke(key: &str) -> String {
        serde_json::json!({"v": 1, "kind": "revoke", "key": key, "issued": now().to_rfc3339()}).to_string()
    }

    fn signed(label: &str, payload: String) -> Candidate {
        Candidate { label: label.into(), payload: payload.into_bytes(), signature: Some(Signature { path: PathBuf::from(format!("{label}.trusted.sig")) }) }
    }

    fn untrusted(label: &str, payload: String) -> Candidate {
        Candidate { label: label.into(), payload: payload.into_bytes(), signature: Some(Signature { path: PathBuf::from(format!("{label}.other.sig")) }) }
    }

    fn reasons(verified: &Verified) -> Vec<(&str, &str)> {
        verified.rejected.iter().map(|r| (r.label.as_str(), r.reason.as_str())).collect()
    }

    #[test]
    fn a_signed_current_entry_counts_and_its_key_loses_the_comment() {
        let verified = evaluate(vec![signed("alpha", entry("alpha", KEY_A, 1, 90))], &ByName, now());
        assert!(verified.rejected.is_empty());
        assert_eq!(verified.devices.len(), 1);
        assert_eq!(verified.devices[0].node_key, KEY_A.rsplit_once(' ').unwrap().0);
        assert_eq!(verified.devices[0].routes[0].to_route(), Route::Host { name: "100.64.0.2".into(), port: Some(22) });
    }

    #[test]
    fn unsigned_and_untrusted_entries_are_ignored_with_their_reason() {
        let unsigned = Candidate { label: "bare".into(), payload: entry("bare", KEY_A, 1, 90).into_bytes(), signature: None };
        let verified = evaluate(vec![unsigned, untrusted("forged", entry("forged", KEY_B, 1, 90))], &ByName, now());
        assert!(verified.devices.is_empty());
        let reasons = reasons(&verified);
        assert!(reasons.iter().any(|(l, r)| *l == "bare" && r.starts_with("unsigned")), "{reasons:?}");
        assert!(reasons.iter().any(|(l, r)| *l == "forged" && r.contains("not signed by a trusted signing key") && r.contains("unknown signer")), "{reasons:?}");
    }

    #[test]
    fn expired_and_future_entries_are_ignored() {
        let verified = evaluate(
            vec![signed("old", entry("old", KEY_A, 100, 90)), signed("later", entry("later", KEY_B, -2, 90))],
            &ByName,
            now(),
        );
        assert!(verified.devices.is_empty());
        let reasons = reasons(&verified);
        assert!(reasons.iter().any(|(l, r)| *l == "old" && r.starts_with("expired on 2026-")), "{reasons:?}");
        assert!(reasons.iter().any(|(l, r)| *l == "later" && r.starts_with("issued in the future")), "{reasons:?}");
    }

    #[test]
    fn a_revoked_key_removes_every_entry_that_carries_it() {
        let verified = evaluate(
            vec![signed("alpha", entry("alpha", KEY_A, 1, 90)), signed("beta", entry("beta", KEY_B, 1, 90)), signed("revoke-a", revoke(KEY_A))],
            &ByName,
            now(),
        );
        assert_eq!(verified.devices.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), ["beta"]);
        assert!(reasons(&verified).iter().any(|(l, r)| *l == "alpha" && r.starts_with("revoked")));
    }

    #[test]
    fn an_untrusted_revocation_revokes_nothing() {
        let verified = evaluate(vec![signed("alpha", entry("alpha", KEY_A, 1, 90)), untrusted("revoke-a", revoke(KEY_A))], &ByName, now());
        assert_eq!(verified.devices.len(), 1);
    }

    #[test]
    fn the_newer_of_two_entries_for_a_name_wins() {
        let verified = evaluate(vec![signed("a-old", entry("alpha", KEY_A, 20, 90)), signed("a-new", entry("alpha", KEY_B, 2, 90))], &ByName, now());
        assert_eq!(verified.devices.len(), 1);
        assert!(verified.devices[0].node_key.contains("BBBB"));
        assert!(reasons(&verified).iter().any(|(l, r)| *l == "a-old" && r.starts_with("superseded")));
        // The order they are read in does not matter.
        let flipped = evaluate(vec![signed("a-new", entry("alpha", KEY_B, 2, 90)), signed("a-old", entry("alpha", KEY_A, 20, 90))], &ByName, now());
        assert_eq!(flipped.devices, verified.devices);
    }

    fn with_key(name: &str, node_key: &str) -> String {
        serde_json::json!({
            "v": 1, "kind": "device", "name": name, "node_key": node_key, "host_key": HOST,
            "issued": (now() - Duration::days(1)).to_rfc3339(), "expires": (now() + Duration::days(10)).to_rfc3339(),
        })
        .to_string()
    }

    #[test]
    fn malformed_entries_are_ignored_and_cannot_carry_options_into_a_key_line() {
        let candidates = vec![
            signed("newline", with_key("evil", "ssh-ed25519 AAAA\nrestrict ssh-ed25519 BBBB")),
            signed("options", with_key("opts", "command=\"rm -rf\" ssh-ed25519 AAAAB")),
            signed("badtype", with_key("odd", "ssh-rsa AAAAB3NzaC1yc2E")),
            signed("name", entry("Bad_Name", KEY_A, 1, 90)),
            signed("reserved", entry("all", KEY_B, 1, 90)),
            signed("version", entry("alpha", KEY_A, 1, 90).replace("\"v\":1", "\"v\":2")),
            signed("garbage", "not json".into()),
        ];
        let verified = evaluate(candidates, &ByName, now());
        assert!(verified.devices.is_empty(), "{:?}", verified.devices);
        let reasons = reasons(&verified);
        assert_eq!(reasons.len(), 7, "{reasons:?}");
        assert!(reasons.iter().any(|(l, r)| *l == "newline" && r.contains("one line")), "{reasons:?}");
        assert!(reasons.iter().any(|(l, r)| *l == "options" && r.contains("not a key type")), "{reasons:?}");
        assert!(reasons.iter().any(|(l, r)| *l == "version" && r.contains("version 2")), "{reasons:?}");
    }

    #[test]
    fn a_bad_route_or_an_entry_that_expires_before_it_starts_is_refused() {
        let bad_host = entry("alpha", KEY_A, 1, 90).replace("100.64.0.2", "evil host;rm");
        let backwards = entry("beta", KEY_B, 1, -5);
        let verified = evaluate(vec![signed("a", bad_host), signed("b", backwards)], &ByName, now());
        assert!(verified.devices.is_empty());
        assert!(reasons(&verified).iter().any(|(l, r)| *l == "a" && r.contains("not a plain name")));
        assert!(reasons(&verified).iter().any(|(l, r)| *l == "b" && r.contains("expires before")));
    }

    #[test]
    fn names_are_checked() {
        for good in ["ab", "orchard", "node-2", "a1"] {
            assert!(valid_name(good).is_ok(), "{good}");
        }
        for bad in ["", "a", "-ab", "ab-", "Orchard", "node_2", "all", "localhost", "a b"] {
            assert!(valid_name(bad).is_err(), "{bad}");
        }
    }
}
