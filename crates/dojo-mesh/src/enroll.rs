//! Enrollment: the pieces that make and sign roster entries. A new node describes itself in a join
//! request; the operator compares its fingerprint, then signs a device entry for it. Signing goes
//! through `ssh-keygen -Y sign`, so the key can be anywhere an SSH agent or a key file can sign:
//! a YubiKey, 1Password, or a file. The signing keys a node can use are tried in the order it lists.

use crate::method::Route;
use crate::signed::{self, DeviceEntry, NAMESPACE, Revocation, RouteSpec, Statement};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// What a new node sends the operator: its name and the public keys it vouches for itself with. It
/// carries no authority; the operator's signature does.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinRequest {
    pub v: u32,
    pub name: String,
    pub node_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_key: Option<String>,
    pub host_key: String,
    #[serde(default)]
    pub routes: Vec<RouteSpec>,
}

impl JoinRequest {
    /// Parses and checks a request, with keys normalized.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut request: Self = serde_json::from_str(text).map_err(|error| format!("not a join request: {error}"))?;
        if request.v != 1 {
            return Err(format!("join request version {} is not understood", request.v));
        }
        // The same checks a signed entry gets, so nothing is signed that would be refused later.
        let probe = signed::well_formed(DeviceEntry {
            name: request.name.clone(),
            node_key: request.node_key.clone(),
            input_key: request.input_key.clone(),
            host_key: request.host_key.clone(),
            routes: request.routes.clone(),
            issued: Utc::now(),
            expires: Utc::now() + Duration::days(1),
        })?;
        request.node_key = probe.node_key;
        request.input_key = probe.input_key;
        request.host_key = probe.host_key;
        Ok(request)
    }

    pub fn entry(&self, issued: DateTime<Utc>, days: i64) -> DeviceEntry {
        DeviceEntry {
            name: self.name.clone(),
            node_key: self.node_key.clone(),
            input_key: self.input_key.clone(),
            host_key: self.host_key.clone(),
            routes: self.routes.clone(),
            issued,
            expires: issued + Duration::days(days),
        }
    }
}

/// `host` or `host:port` as a route. A name with more than one colon is an IPv6 address and takes
/// no port here.
pub fn parse_route(text: &str) -> Result<RouteSpec, String> {
    let (host, port) = match text.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') => (host, Some(port.parse::<u16>().map_err(|_| format!("{port:?} is not a port"))?)),
        _ => (text, None),
    };
    let route = RouteSpec { host: host.to_string(), port };
    let probe = DeviceEntry {
        name: "probe".into(),
        node_key: "ssh-ed25519 AAAA".into(),
        input_key: None,
        host_key: "ssh-ed25519 AAAA".into(),
        routes: vec![route.clone()],
        issued: Utc::now(),
        expires: Utc::now() + Duration::days(1),
    };
    signed::well_formed(probe)?;
    let _: Route = route.to_route();
    Ok(route)
}

/// A device entry as the file the roster holds, before it is signed.
pub fn device_json(entry: &DeviceEntry) -> String {
    statement_json(&Statement::Device(entry.clone()))
}

pub fn revocation_json(key: &str, reason: Option<&str>, issued: DateTime<Utc>) -> String {
    statement_json(&Statement::Revoke(Revocation { key: key.to_string(), reason: reason.map(str::to_string), issued }))
}

fn statement_json(statement: &Statement) -> String {
    let mut value = serde_json::to_value(statement).expect("a statement is plain data");
    value.as_object_mut().expect("a statement is an object").insert("v".into(), 1.into());
    serde_json::to_string_pretty(&value).expect("a value prints") + "\n"
}

/// The keys this node may sign with, in the order they are tried.
pub struct Signing {
    /// A private key file, or a public key file whose private half an SSH agent holds.
    pub keys: Vec<PathBuf>,
    pub agent: Option<PathBuf>,
}

/// Signs `file`, writing `file.sig`. The first key that signs wins; when none do, every reason is given.
pub fn sign_file(file: &Path, signing: &Signing) -> Result<(), String> {
    if signing.keys.is_empty() {
        return Err("no signing key is set: list the keys this node signs with as signing-keys in mesh.toml".into());
    }
    let sig = PathBuf::from(format!("{}.sig", file.display()));
    let mut reasons = Vec::new();
    for key in &signing.keys {
        let _ = std::fs::remove_file(&sig);
        let mut command = Command::new("ssh-keygen");
        command.args(["-Y", "sign", "-q"]);
        let agent_held = key.extension().and_then(|ext| ext.to_str()) == Some("pub");
        if agent_held {
            command.arg("-U");
            if let Some(socket) = &signing.agent {
                command.env("SSH_AUTH_SOCK", socket);
            }
        }
        command.arg("-f").arg(key).args(["-n", NAMESPACE]).arg(file).stdin(Stdio::null());
        match command.output() {
            Ok(output) if output.status.success() && sig.exists() => return Ok(()),
            Ok(output) => {
                let text = String::from_utf8_lossy(&output.stderr);
                let line = text.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or("it did not sign");
                reasons.push(format!("{}: {line}", key.display()));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Err("ssh-keygen is not installed".into()),
            Err(error) => reasons.push(format!("{}: {error}", key.display())),
        }
    }
    let _ = std::fs::remove_file(&sig);
    Err(format!("no signing key could sign ({})", reasons.join("; ")))
}

/// Writes `text` to `path` and signs it. A file that could not be signed is removed again, so an
/// unsigned statement never lingers.
pub fn write_signed(path: &Path, text: &str, signing: &Signing) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
    }
    let previous = std::fs::read(path).ok();
    std::fs::write(path, text).map_err(|error| format!("{}: {error}", path.display()))?;
    if let Err(reason) = sign_file(path, signing) {
        // Put back what was there, signature included, rather than leave a statement that does not match.
        match previous {
            Some(bytes) => {
                let _ = std::fs::write(path, bytes);
            }
            None => {
                let _ = std::fs::remove_file(path);
            }
        }
        return Err(reason);
    }
    Ok(())
}

/// `SHA256:…` for a public key line, as `ssh-keygen -l` prints it.
pub fn fingerprint(key: &str) -> Result<String, String> {
    let mut child = Command::new("ssh-keygen")
        .args(["-l", "-f", "/dev/stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| if error.kind() == std::io::ErrorKind::NotFound { "ssh-keygen is not installed".to_string() } else { format!("could not run ssh-keygen: {error}") })?;
    let mut stdin = child.stdin.take().expect("stdin was piped");
    let line = format!("{key}\n");
    let feeder = std::thread::spawn(move || {
        let _ = stdin.write_all(line.as_bytes());
    });
    let output = child.wait_with_output().map_err(|error| format!("lost track of ssh-keygen: {error}"))?;
    let _ = feeder.join();
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .nth(1)
        .filter(|word| word.starts_with("SHA256:"))
        .map(str::to_string)
        .ok_or_else(|| "that is not a public key ssh-keygen can read".to_string())
}

/// How a key is held, as far as its type shows: a security key (hardware, needs a touch) or software
/// (a file or an agent that may remember an approval).
pub fn held_as(key: &str) -> &'static str {
    if key.starts_with("sk-") { "hardware security key" } else { "software" }
}

/// The signers file's lines, as written by `trust`: every trusted key under one principal.
pub fn signer_line(key: &str) -> String {
    format!("operator namespaces=\"{NAMESPACE}\" {key}")
}

/// The public key in each line of an allowed-signers file, with the lines that are not signers skipped.
pub fn signers(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            // principal [options] keytype base64 [comment]; the options may hold a quoted space.
            let at = line.find(|c: char| c == ' ').map(|at| at + 1)?;
            let rest = &line[at..];
            let start = ["sk-ssh-ed25519@openssh.com ", "ssh-ed25519 ", "ecdsa-sha2-nistp256 "].iter().filter_map(|kind| rest.find(kind)).min()?;
            signed::normalize_key(&rest[start..]).ok()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

    #[test]
    fn a_request_is_checked_like_the_entry_it_would_become() {
        let good = serde_json::json!({"v": 1, "name": "orchard", "node_key": format!("{KEY} comment"), "host_key": KEY, "routes": [{"host": "192.0.2.1", "port": 22}]}).to_string();
        let request = JoinRequest::parse(&good).unwrap();
        assert_eq!(request.node_key, KEY);
        for (patch, why) in [
            (serde_json::json!({"name": "Bad Name"}), "valid node name"),
            (serde_json::json!({"node_key": "command=\"x\" ssh-ed25519 AAAA"}), "key type"),
            (serde_json::json!({"routes": [{"host": "a b"}]}), "plain name"),
            (serde_json::json!({"v": 2}), "not understood"),
        ] {
            let mut value: serde_json::Value = serde_json::from_str(&good).unwrap();
            value.as_object_mut().unwrap().extend(patch.as_object().unwrap().clone());
            let error = JoinRequest::parse(&value.to_string()).unwrap_err();
            assert!(error.contains(why), "{error}");
        }
    }

    #[test]
    fn routes_are_a_host_with_an_optional_port() {
        assert_eq!(parse_route("192.0.2.1:2222").unwrap(), RouteSpec { host: "192.0.2.1".into(), port: Some(2222) });
        assert_eq!(parse_route("orchard.example").unwrap(), RouteSpec { host: "orchard.example".into(), port: None });
        assert_eq!(parse_route("fd00::1").unwrap().port, None);
        assert!(parse_route("host:notaport").is_err());
        assert!(parse_route("-oProxyCommand=x").is_err());
        assert!(parse_route("-ohost").is_err());
    }

    #[test]
    fn statements_are_versioned_and_round_trip_through_the_roster_reader() {
        let request = JoinRequest { v: 1, name: "orchard".into(), node_key: KEY.into(), input_key: None, host_key: KEY.into(), routes: vec![] };
        let now = Utc::now();
        let text = device_json(&request.entry(now, 30));
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!((value["v"].as_u64(), value["kind"].as_str(), value["name"].as_str()), (Some(1), Some("device"), Some("orchard")));
        let revoke: serde_json::Value = serde_json::from_str(&revocation_json(KEY, Some("lost"), now)).unwrap();
        assert_eq!((revoke["v"].as_u64(), revoke["kind"].as_str(), revoke["reason"].as_str()), (Some(1), Some("revoke"), Some("lost")));
    }

    #[test]
    fn signers_are_read_back_from_the_file_they_were_written_to() {
        let text = format!("# trusted\n{}\noperator namespaces=\"dojo-mesh-roster\" sk-ssh-ed25519@openssh.com AAAAGnNr second\n\n", signer_line(KEY));
        assert_eq!(signers(&text), [KEY.to_string(), "sk-ssh-ed25519@openssh.com AAAAGnNr".to_string()]);
        assert_eq!(held_as("sk-ssh-ed25519@openssh.com AAAA"), "hardware security key");
        assert_eq!(held_as(KEY), "software");
    }
}
