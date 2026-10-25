//! The OpenSSH method's membership: reads a roster directory, checks each file's signature with
//! `ssh-keygen -Y verify` against the node's own trusted signing keys, and probes whether each peer
//! can be reached. The signing keys are never read from the roster directory, so whoever can write
//! to the roster's remote cannot add one.

use crate::config::MeshConfig;
use crate::device::Device;
use crate::method::{Membership, Route};
use crate::roster::{Admission, Excluded, Exclusion, Member, Roster, Source};
use crate::signed::{self, Candidate, DeviceEntry, NAMESPACE, Rejected, Signature, Verifier};
use chrono::{DateTime, Utc};
use std::io::Write;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// Every trusted signing key is listed under this principal in the signers file, so any of them may
/// sign: `operator namespaces="dojo-mesh-roster" ssh-ed25519 AAAA…`.
pub const PRINCIPAL: &str = "operator";

/// A roster file bigger than this is not a roster file.
const MAX_FILE: u64 = 64 * 1024;

/// How long to wait for a peer to answer before calling it offline.
const PROBE: Duration = Duration::from_millis(1500);

/// Checks signatures with the system `ssh-keygen`, so every key it supports (hardware-backed ones
/// included) works without Dojo handling key material.
pub struct SshKeygen {
    pub signers: PathBuf,
}

impl Verifier for SshKeygen {
    fn verify(&self, payload: &[u8], signature: &Signature) -> Result<(), String> {
        let mut child = Command::new("ssh-keygen")
            .args(["-Y", "verify", "-f"])
            .arg(&self.signers)
            .args(["-I", PRINCIPAL, "-n", NAMESPACE, "-s"])
            .arg(&signature.path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::NotFound { "ssh-keygen is not installed".to_string() } else { format!("could not run ssh-keygen: {error}") }
            })?;
        let mut stdin = child.stdin.take().expect("stdin was piped");
        let bytes = payload.to_vec();
        let feeder = std::thread::spawn(move || {
            let _ = stdin.write_all(&bytes);
        });
        let output = child.wait_with_output().map_err(|error| format!("lost track of ssh-keygen: {error}"))?;
        let _ = feeder.join();
        if output.status.success() {
            return Ok(());
        }
        // ssh-keygen words its refusals for people; keep the part that says why.
        let text = String::from_utf8_lossy(&output.stderr);
        let line = text.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or("verification failed");
        let lower = line.to_ascii_lowercase();
        Err(if lower.contains("incorrect signature") {
            "the file does not match its signature, so it was changed after signing or signed by another key".to_string()
        } else if lower.contains("no principal matched") || lower == "verification failed" || lower.contains("could not verify signature") {
            "no trusted signing key signed it".to_string()
        } else {
            line.to_string()
        })
    }
}

/// Reads the signed files of a roster directory: `devices/*.json` and `revocations/*.json`, each with
/// its signature beside it as `<file>.sig`. Anything that is not a plain file is left alone (a
/// symlink could point outside the roster).
pub fn load(dir: &Path) -> Result<(Vec<Candidate>, Vec<Rejected>), String> {
    let mut candidates = Vec::new();
    let mut rejected = Vec::new();
    for sub in ["devices", "revocations"] {
        let folder = dir.join(sub);
        let entries = match std::fs::read_dir(&folder) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("{}: {error}", folder.display())),
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") || !entry.file_type().is_ok_and(|kind| kind.is_file()) {
                continue;
            }
            let label = format!("{sub}/{}", entry.file_name().to_string_lossy());
            let payload = match std::fs::metadata(&path) {
                Ok(meta) if meta.len() > MAX_FILE => {
                    rejected.push(Rejected { label, reason: format!("larger than {} KiB, which no roster file is", MAX_FILE / 1024) });
                    continue;
                }
                _ => std::fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?,
            };
            let sig = PathBuf::from(format!("{}.sig", path.display()));
            let signature = std::fs::symlink_metadata(&sig).ok().filter(|meta| meta.is_file()).map(|_| Signature { path: sig });
            candidates.push(Candidate { label, payload, signature });
        }
    }
    candidates.sort_by(|a, b| a.label.cmp(&b.label));
    Ok((candidates, rejected))
}

/// Whether a peer's route answers. Offline peers fail fast instead of after a connection timeout.
pub trait Reachable: Send + Sync {
    fn reachable(&self, route: &Route) -> bool;
}

pub struct Tcp;

impl Reachable for Tcp {
    fn reachable(&self, route: &Route) -> bool {
        match route {
            Route::Host { name, port } => (name.as_str(), port.unwrap_or(22))
                .to_socket_addrs()
                .is_ok_and(|mut addresses| addresses.any(|address| TcpStream::connect_timeout(&address, PROBE).is_ok())),
            // A command route cannot be probed without running it; it is tried when used.
            Route::Command(_) => true,
        }
    }
}

/// Membership from a signed roster directory.
pub struct SignedMembership {
    pub dir: PathBuf,
    pub signers: PathBuf,
    verifier: Box<dyn Verifier + Send + Sync>,
    reachable: Box<dyn Reachable>,
    now: fn() -> DateTime<Utc>,
}

impl SignedMembership {
    pub fn new(dir: PathBuf, signers: PathBuf) -> Self {
        let verifier = Box::new(SshKeygen { signers: signers.clone() });
        Self { dir, signers, verifier, reachable: Box::new(Tcp), now: Utc::now }
    }

    pub fn with(dir: PathBuf, signers: PathBuf, verifier: Box<dyn Verifier + Send + Sync>, reachable: Box<dyn Reachable>, now: fn() -> DateTime<Utc>) -> Self {
        Self { dir, signers, verifier, reachable, now }
    }

    fn device(&self, entry: &DeviceEntry, online: bool) -> Device {
        Device {
            name: entry.name.clone(),
            dns_name: None,
            tags: Vec::new(),
            online,
            key_expiry: Some(entry.expires),
            key_expired: false,
            routes: entry.routes.iter().map(|route| route.to_route()).collect(),
            keys: Some(crate::device::Keys { node_key: entry.node_key.clone(), input_key: entry.input_key.clone(), host_key: entry.host_key.clone() }),
        }
    }
}

impl SignedMembership {
    /// This node's name and the roster's current statements. `Err` says why there is no roster to
    /// believe; the caller makes that a mesh of one.
    fn verified(&self, config: &MeshConfig, hostname: &str) -> Result<(String, signed::Verified), String> {
        let this_name = config.node_name.clone().unwrap_or_else(|| hostname.trim().to_ascii_lowercase());
        if !std::fs::metadata(&self.signers).is_ok_and(|meta| meta.is_file() && meta.len() > 0) {
            return Err(format!("no trusted signing keys at {}; the roster cannot be believed without one", self.signers.display()));
        }
        let (candidates, oversized) = load(&self.dir).map_err(|reason| format!("the roster cannot be read ({reason})"))?;
        let mut verified = signed::evaluate(candidates, self.verifier.as_ref(), (self.now)());
        verified.rejected.extend(oversized);
        verified.rejected.sort_by(|a, b| a.label.cmp(&b.label));
        Ok((this_name, verified))
    }

    /// The names of the members other than this node that the configuration does not deny, without
    /// asking any of them anything.
    pub fn member_names(&self, config: &MeshConfig, hostname: &str) -> Vec<String> {
        let Ok((this_name, verified)) = self.verified(config, hostname) else { return Vec::new() };
        verified
            .devices
            .into_iter()
            .filter(|entry| entry.name != this_name && !config.deny.iter().any(|key| crate::config::name_key(key) == entry.name))
            .map(|entry| entry.name)
            .collect()
    }
}

impl Membership for std::sync::Arc<SignedMembership> {
    fn roster(&self, config: &MeshConfig, hostname: &str) -> Roster {
        self.as_ref().roster(config, hostname)
    }
}

impl Membership for SignedMembership {
    fn roster(&self, config: &MeshConfig, hostname: &str) -> Roster {
        let this_name = config.node_name.clone().unwrap_or_else(|| hostname.trim().to_ascii_lowercase());
        let alone = |reason: String| {
            Roster::alone(Device { name: this_name.clone(), dns_name: None, tags: Vec::new(), online: true, key_expiry: None, key_expired: false, routes: Vec::new(), keys: None }, config, reason)
        };
        let (this_name, verified) = match self.verified(config, hostname) {
            Ok(found) => found,
            Err(reason) => return alone(reason),
        };
        let denied = |name: &str| config.deny.iter().any(|key| crate::config::name_key(key) == name);
        let (own, others): (Vec<&DeviceEntry>, Vec<&DeviceEntry>) = verified.devices.iter().partition(|entry| entry.name == this_name);
        let this = match own.first() {
            Some(entry) => self.device(entry, true),
            None => Device { name: this_name.clone(), dns_name: None, tags: Vec::new(), online: true, key_expiry: None, key_expired: false, routes: Vec::new(), keys: None },
        };

        // Peers are probed together so a roster of offline nodes costs one timeout, not one each.
        let probed: Vec<(&DeviceEntry, bool)> = std::thread::scope(|scope| {
            let handles: Vec<_> = others
                .iter()
                .map(|entry| {
                    let reachable = self.reachable.as_ref();
                    scope.spawn(move || (*entry, entry.routes.first().is_some_and(|route| reachable.reachable(&route.to_route()))))
                })
                .collect();
            handles.into_iter().map(|handle| handle.join().expect("probe thread")).collect()
        });

        let mut members = vec![Member { device: this, admitted_by: Admission::This }];
        let mut excluded = Vec::new();
        for (entry, online) in probed {
            if denied(&entry.name) {
                excluded.push(Excluded { device: self.device(entry, online), reason: Exclusion::Denied });
            } else {
                members.push(Member { device: self.device(entry, online), admitted_by: Admission::Signed });
            }
        }
        members[1..].sort_by(|a, b| a.device.name.cmp(&b.device.name));
        excluded.sort_by(|a, b| a.device.name.cmp(&b.device.name));
        Roster { source: Source::Method("roster".into()), membership_tag: None, members, excluded, rejected: verified.rejected }
    }
}

/// Pulls the roster directory from its git remote. Fast-forward only: a remote that rewrote history
/// is refused, not followed. Verification is separate and still applies to whatever arrives.
pub fn sync(dir: &Path) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["pull", "--ff-only", "--quiet"])
        .stdin(Stdio::null())
        .output()
        .map_err(|error| if error.kind() == std::io::ErrorKind::NotFound { "git is not installed".to_string() } else { format!("could not run git: {error}") })?;
    if output.status.success() {
        Ok("the roster is up to date".to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// Who a connection is, by the name its input key's forced command gives it, checked against the
/// signed roster. A sender that is not in the roster now (revoked, expired, removed) is nobody.
pub struct RosterIdentity {
    membership: std::sync::Arc<SignedMembership>,
    config: MeshConfig,
    hostname: String,
}

impl crate::method::Identity for RosterIdentity {
    fn who(&self, connection: &crate::method::Connection) -> Result<String, String> {
        let name = connection.named.as_deref().ok_or("this connection did not name its sender; a peer's input key must run inject --as NAME")?;
        if self.membership.member_names(&self.config, &self.hostname).iter().any(|member| member == name) {
            Ok(name.to_string())
        } else {
            Err(format!("{name} is not in the signed roster"))
        }
    }
}

/// The roster method: membership from the signed roster, calls over the system `ssh`.
pub fn method(paths: crate::config::RosterPaths, config: MeshConfig, hostname: String, user: String) -> crate::method::Method {
    let membership = std::sync::Arc::new(SignedMembership::new(paths.dir, paths.signers));
    let channel = crate::sshchannel::OpenSsh::new(crate::sshchannel::Settings {
        ssh: "ssh".into(),
        ssh_add: "ssh-add".into(),
        user,
        node_key: paths.node_key,
        input_key: paths.input_key,
        agent: paths.agent,
        known_hosts: paths.known_hosts,
    });
    crate::method::Method {
        name: "roster",
        membership: Box::new(membership.clone()),
        identity: Box::new(RosterIdentity { membership, config, hostname }),
        channel: Box::new(channel),
    }
}
