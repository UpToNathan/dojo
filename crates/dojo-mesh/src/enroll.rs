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

/// The namespace a join request is signed under, so a signature made for one purpose is never read as
/// another (the roster's entries have their own).
pub const JOIN_NAMESPACE: &str = "dojo-mesh-join";

/// What a new node sends the operator: its name and the public keys it vouches for itself with, signed
/// by its own node key. The signature covers every field, so none can be changed on the way (the host
/// key it asks to have pinned, say) without the operator seeing it fail; the operator then compares
/// only the node key's fingerprint with what the new node shows. It carries no authority: the
/// operator's own signature on the roster entry does.
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
    /// `ssh-keygen -Y sign` output (armored) over [`JoinRequest::body`], made with the node key.
    #[serde(default)]
    pub signature: String,
}

/// The signed bytes: the request without its signature, in a fixed field order.
#[derive(Serialize)]
struct JoinBody<'a> {
    v: u32,
    name: &'a str,
    node_key: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    input_key: &'a Option<String>,
    host_key: &'a str,
    routes: &'a [RouteSpec],
}

/// A signature is a few hundred bytes; this is the most one may be.
const MAX_SIGNATURE: usize = 4096;

impl JoinRequest {
    /// The bytes the node key signs.
    pub fn body(&self) -> Vec<u8> {
        serde_json::to_vec(&JoinBody { v: self.v, name: &self.name, node_key: &self.node_key, input_key: &self.input_key, host_key: &self.host_key, routes: &self.routes }).expect("a request is plain data")
    }

    /// Parses and checks a request's shape, with keys that must already be in canonical form (what
    /// was signed is what is read). The signature itself is checked by [`JoinRequest::verify`].
    pub fn parse(text: &str) -> Result<Self, String> {
        let request: Self = serde_json::from_str(text).map_err(|error| format!("not a join request: {error}"))?;
        if request.v == 1 {
            return Err("that join request is unsigned (version 1): make a new one with the current `dojo-mesh join`, which signs it with the machine's own key".into());
        }
        if request.v != 2 {
            return Err(format!("join request version {} is not understood", request.v));
        }
        if request.signature.trim().is_empty() || request.signature.len() > MAX_SIGNATURE {
            return Err("the join request carries no usable signature".into());
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
        if probe.node_key != request.node_key || probe.input_key != request.input_key || probe.host_key != request.host_key {
            return Err("the join request's keys are not in canonical form (`type base64`, no comment), so what was signed cannot be what is shown".into());
        }
        Ok(request)
    }

    /// Whether the node key signed exactly this request.
    pub fn verify(&self) -> Result<(), String> {
        verify_bytes(&self.body(), &self.signature, &self.node_key, JOIN_NAMESPACE)
            .map_err(|reason| format!("the join request's signature does not match its node key ({reason}): it was changed after the machine made it, or it was not made with that key"))
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

/// Signs `body` under `namespace` with the key at `public` (a `.pub` file): with its private file when
/// one sits beside it, otherwise through the SSH agent that holds it. The signature is returned, not
/// written anywhere.
pub fn sign_bytes(body: &[u8], public: &Path, namespace: &str, agent: Option<&Path>) -> Result<String, String> {
    let private = public.with_extension("");
    let beside = public.extension().and_then(|ext| ext.to_str()) == Some("pub") && private.is_file();
    let mut command = Command::new("ssh-keygen");
    command.args(["-Y", "sign", "-n", namespace]);
    if beside {
        command.arg("-f").arg(&private);
    } else {
        command.arg("-U").arg("-f").arg(public);
        if let Some(socket) = agent {
            command.env("SSH_AUTH_SOCK", socket);
        }
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| if error.kind() == std::io::ErrorKind::NotFound { "ssh-keygen is not installed".to_string() } else { format!("could not run ssh-keygen: {error}") })?;
    let mut stdin = child.stdin.take().expect("stdin was piped");
    let bytes = body.to_vec();
    let feeder = std::thread::spawn(move || {
        let _ = stdin.write_all(&bytes);
    });
    let output = child.wait_with_output().map_err(|error| format!("lost track of ssh-keygen: {error}"))?;
    let _ = feeder.join();
    if !output.status.success() {
        let text = String::from_utf8_lossy(&output.stderr);
        let line = text.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or("it did not sign");
        return Err(format!("{}: {line}", public.display()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Whether `signature` is `public_key`'s signature of `body` under `namespace`. The only key trusted is
/// the one given; nothing is read from the machine's own configuration.
pub fn verify_bytes(body: &[u8], signature: &str, public_key: &str, namespace: &str) -> Result<(), String> {
    use std::os::unix::fs::DirBuilderExt;
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let scratch = std::env::temp_dir().join(format!("dojo-mesh-verify-{}-{}", std::process::id(), COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    std::fs::DirBuilder::new().mode(0o700).create(&scratch).map_err(|error| format!("{}: {error}", scratch.display()))?;
    let outcome = (|| {
        let signers = scratch.join("signers");
        let sig = scratch.join("signature");
        std::fs::write(&signers, format!("request namespaces=\"{namespace}\" {public_key}\n")).map_err(|error| error.to_string())?;
        std::fs::write(&sig, signature).map_err(|error| error.to_string())?;
        let mut child = Command::new("ssh-keygen")
            .args(["-Y", "verify", "-f"])
            .arg(&signers)
            .args(["-I", "request", "-n", namespace, "-s"])
            .arg(&sig)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| if error.kind() == std::io::ErrorKind::NotFound { "ssh-keygen is not installed".to_string() } else { format!("could not run ssh-keygen: {error}") })?;
        let mut stdin = child.stdin.take().expect("stdin was piped");
        let bytes = body.to_vec();
        let feeder = std::thread::spawn(move || {
            let _ = stdin.write_all(&bytes);
        });
        let output = child.wait_with_output().map_err(|error| format!("lost track of ssh-keygen: {error}"))?;
        let _ = feeder.join();
        if output.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&output.stderr).lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or("verification failed").to_string()) }
    })();
    let _ = std::fs::remove_dir_all(&scratch);
    outcome
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
        let good = serde_json::json!({"v": 2, "name": "orchard", "node_key": KEY, "host_key": KEY, "routes": [{"host": "192.0.2.1", "port": 22}], "signature": "-----BEGIN SSH SIGNATURE-----\nxx\n-----END SSH SIGNATURE-----\n"}).to_string();
        let request = JoinRequest::parse(&good).unwrap();
        assert_eq!(request.node_key, KEY);
        for (patch, why) in [
            (serde_json::json!({"name": "Bad Name"}), "valid node name"),
            (serde_json::json!({"node_key": "command=\"x\" ssh-ed25519 AAAA"}), "key type"),
            (serde_json::json!({"routes": [{"host": "a b"}]}), "plain name"),
            (serde_json::json!({"v": 3}), "not understood"),
            (serde_json::json!({"v": 1}), "unsigned"),
            (serde_json::json!({"signature": ""}), "no usable signature"),
            (serde_json::json!({"node_key": format!("{KEY} a-comment")}), "canonical form"),
        ] {
            let mut value: serde_json::Value = serde_json::from_str(&good).unwrap();
            value.as_object_mut().unwrap().extend(patch.as_object().unwrap().clone());
            let error = JoinRequest::parse(&value.to_string()).unwrap_err();
            assert!(error.contains(why), "{error}");
        }
    }

    /// A key pair made by the real `ssh-keygen` in a private directory.
    fn keypair(dir: &Path, name: &str) -> (PathBuf, String) {
        let private = dir.join(name);
        let status = Command::new("ssh-keygen").args(["-q", "-t", "ed25519", "-N", "", "-f"]).arg(&private).status().expect("ssh-keygen runs");
        assert!(status.success());
        let public = std::fs::read_to_string(dir.join(format!("{name}.pub"))).unwrap();
        (dir.join(format!("{name}.pub")), public.split_whitespace().take(2).collect::<Vec<_>>().join(" "))
    }

    fn signed_request(dir: &Path, name: &str) -> JoinRequest {
        let (public, key) = keypair(dir, name);
        let (_, host) = keypair(dir, "host");
        let mut request = JoinRequest { v: 2, name: name.into(), node_key: key, input_key: None, host_key: host, routes: vec![RouteSpec { host: "192.0.2.1".into(), port: Some(2222) }], signature: String::new() };
        request.signature = sign_bytes(&request.body(), &public, JOIN_NAMESPACE, None).unwrap();
        request
    }

    fn lab(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dojo-join-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_request_signed_by_its_own_node_key_verifies_and_survives_a_round_trip_through_a_file() {
        let dir = lab("ok");
        let request = signed_request(&dir, "orchard");
        request.verify().unwrap();
        let again = JoinRequest::parse(&serde_json::to_string_pretty(&request).unwrap()).unwrap();
        again.verify().unwrap();
        assert_eq!(again, request);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn changing_any_field_after_signing_is_caught_even_when_the_node_key_is_untouched() {
        let dir = lab("tamper");
        let request = signed_request(&dir, "orchard");
        let (_, attacker) = keypair(&dir, "attacker");
        let mut host = request.clone();
        host.host_key = attacker.clone();
        assert!(host.verify().is_err(), "a swapped host key");
        let mut input = request.clone();
        input.input_key = Some(attacker);
        assert!(input.verify().is_err(), "an added input key");
        let mut route = request.clone();
        route.routes[0].host = "198.51.100.9".into();
        assert!(route.verify().is_err(), "a changed route");
        let mut name = request.clone();
        name.name = "mallory".into();
        assert!(name.verify().is_err(), "a changed name");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_request_signed_by_some_other_key_or_for_some_other_purpose_does_not_verify() {
        let dir = lab("other");
        let mut request = signed_request(&dir, "orchard");
        // Someone else's signature over the same body, claiming to be the node's.
        let (other_public, _) = keypair(&dir, "other");
        request.signature = sign_bytes(&request.body(), &other_public, JOIN_NAMESPACE, None).unwrap();
        assert!(request.verify().is_err(), "another key's signature");
        // The node's own signature, made for the roster's purpose, is not a join request.
        let public = dir.join("orchard.pub");
        request.signature = sign_bytes(&request.body(), &public, "dojo-mesh-roster", None).unwrap();
        assert!(request.verify().is_err(), "a signature made for another namespace");
        let _ = std::fs::remove_dir_all(dir);
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
        let request = JoinRequest { v: 2, name: "orchard".into(), node_key: KEY.into(), input_key: None, host_key: KEY.into(), routes: vec![], signature: String::new() };
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
