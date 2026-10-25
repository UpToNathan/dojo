//! The signed-roster method end to end: real `ssh-keygen` signatures, a real git remote and the real
//! binary. Peers' routes point at loopback ports this test opens or leaves closed.

use chrono::{Duration, Utc};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static LABS: AtomicUsize = AtomicUsize::new(0);

struct Lab {
    root: PathBuf,
    /// Held open so a peer that is meant to be online answers.
    _listeners: Vec<TcpListener>,
}

impl Drop for Lab {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn tool(program: &str, args: &[&str], dir: &Path) -> String {
    let output = Command::new(program).args(args).current_dir(dir).output().unwrap_or_else(|error| panic!("{program}: {error}"));
    assert!(output.status.success(), "{program} {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).into_owned()
}

impl Lab {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("dojo-roster-{}-{}", std::process::id(), LABS.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&root);
        for dir in ["keys", "roster/devices", "roster/revocations"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        let lab = Self { root, _listeners: Vec::new() };
        // The operator's signing key is trusted; a stranger's is not.
        lab.key("operator");
        lab.key("stranger");
        lab.trust(&["operator"]);
        lab.config("");
        lab
    }

    fn key(&self, name: &str) -> String {
        let path = self.root.join("keys").join(name);
        tool("ssh-keygen", &["-q", "-t", "ed25519", "-N", "", "-C", name, "-f", path.to_str().unwrap()], &self.root);
        self.public(name)
    }

    /// `type base64`, no comment.
    fn public(&self, name: &str) -> String {
        let text = std::fs::read_to_string(self.root.join("keys").join(format!("{name}.pub"))).unwrap();
        text.split_whitespace().take(2).collect::<Vec<_>>().join(" ")
    }

    fn trust(&self, signers: &[&str]) {
        let lines: String = signers.iter().map(|name| format!("operator namespaces=\"dojo-mesh-roster\" {}\n", self.public(name))).collect();
        std::fs::write(self.root.join("allowed_signers"), lines).unwrap();
    }

    fn config(&self, extra: &str) {
        let text = format!(
            "method = \"roster\"\nroster-dir = \"{}\"\nsigners-file = \"{}\"\nnode-name = \"alpha\"\n{extra}",
            self.root.join("roster").display(),
            self.root.join("allowed_signers").display()
        );
        std::fs::write(self.root.join("mesh.toml"), text).unwrap();
    }

    /// An open loopback port (the peer answers) or a closed one (it does not).
    fn port(&mut self, open: bool) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        if open {
            self._listeners.push(listener);
        }
        port
    }

    fn device_json(&self, name: &str, node_key: &str, port: u16, valid_days: i64) -> String {
        self.device_between(name, node_key, port, Utc::now() - Duration::hours(1), Utc::now() + Duration::days(valid_days))
    }

    fn device_between(&self, name: &str, node_key: &str, port: u16, issued: chrono::DateTime<Utc>, expires: chrono::DateTime<Utc>) -> String {
        serde_json::json!({
            "v": 1, "kind": "device", "name": name, "node_key": node_key, "host_key": self.public("operator"),
            "routes": [{"host": "127.0.0.1", "port": port}],
            "issued": issued.to_rfc3339(),
            "expires": expires.to_rfc3339(),
        })
        .to_string()
    }

    fn write(&self, relative: &str, text: &str) -> PathBuf {
        let path = self.root.join("roster").join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
        path
    }

    fn sign(&self, path: &Path, key: &str) {
        let _ = std::fs::remove_file(format!("{}.sig", path.display()));
        tool("ssh-keygen", &["-Y", "sign", "-q", "-f", self.root.join("keys").join(key).to_str().unwrap(), "-n", "dojo-mesh-roster", path.to_str().unwrap()], &self.root);
    }

    /// A device entry signed by `signer`.
    fn member(&self, name: &str, node_key: &str, port: u16, signer: &str) -> PathBuf {
        let path = self.write(&format!("devices/{name}.json"), &self.device_json(name, node_key, port, 90));
        self.sign(&path, signer);
        path
    }

    fn run(&self, args: &[&str]) -> (i32, String) {
        let output = Command::new(env!("CARGO_BIN_EXE_dojo-mesh"))
            .arg("--config")
            .arg(self.root.join("mesh.toml"))
            .args(args)
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", &self.root)
            .output()
            .unwrap();
        let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        (output.status.code().unwrap(), text)
    }
}

#[test]
fn signed_entries_form_the_mesh_and_offline_peers_are_told_apart() {
    let mut lab = Lab::new();
    let (open, closed) = (lab.port(true), lab.port(false));
    let (a, b, c) = (lab.key("alpha"), lab.key("beta"), lab.key("gamma"));
    lab.member("alpha", &a, open, "operator");
    lab.member("beta", &b, open, "operator");
    lab.member("gamma", &c, closed, "operator");
    let (code, out) = lab.run(&["status"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.starts_with("Mesh of 3\n"), "{out}");
    assert!(out.contains("alpha (this node)  online"), "{out}");
    assert!(out.contains("beta  online"), "{out}");
    assert!(out.contains("gamma  offline"), "{out}");
    let (code, out) = lab.run(&["roster", "check"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("Accepted (3):") && out.contains("Ignored: nothing"), "{out}");
}

#[test]
fn unsigned_tampered_untrusted_and_expired_entries_are_ignored_and_say_why() {
    let mut lab = Lab::new();
    let open = lab.port(true);
    let keys: Vec<String> = ["alpha", "beta", "mallory", "tampered", "stale"].iter().map(|name| lab.key(name)).collect();
    lab.member("alpha", &keys[0], open, "operator");
    lab.member("beta", &keys[1], open, "operator");
    // No signature at all.
    lab.write("devices/bare.json", &lab.device_json("bare", &keys[2], open, 90));
    // Signed by a key this node does not trust.
    lab.member("mallory", &keys[2], open, "stranger");
    // Signed, then changed to point somewhere else.
    let path = lab.member("tampered", &keys[3], open, "operator");
    std::fs::write(&path, lab.device_json("tampered", &keys[2], open, 90)).unwrap();
    // Signed but past its expiry.
    let path = lab.write("devices/stale.json", &lab.device_between("stale", &keys[4], open, Utc::now() - Duration::days(100), Utc::now() - Duration::days(3)));
    lab.sign(&path, "operator");

    let (code, out) = lab.run(&["status", "--all"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.starts_with("Mesh of 2\n"), "{out}");
    for name in ["bare", "mallory", "tampered", "stale"] {
        assert!(!out.contains(&format!("\n  {name} ")), "{name} must not be a member: {out}");
    }
    let (code, out) = lab.run(&["roster", "check"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("devices/bare.json: unsigned"), "{out}");
    assert!(out.contains("devices/mallory.json: not signed by a trusted signing key"), "{out}");
    assert!(out.contains("devices/tampered.json: not signed by a trusted signing key (the file does not match its signature"), "{out}");
    assert!(out.contains("devices/stale.json: expired on"), "{out}");
    let (_, doctor) = lab.run(&["doctor"]);
    assert!(doctor.contains("Ignored devices/mallory.json"), "{doctor}");
    assert!(!doctor.contains("membership tag"), "tag advice does not apply to a roster: {doctor}");
}

#[test]
fn a_signed_revocation_removes_a_device_and_an_untrusted_one_does_nothing() {
    let mut lab = Lab::new();
    let open = lab.port(true);
    let (a, b, c) = (lab.key("alpha"), lab.key("beta"), lab.key("gamma"));
    lab.member("alpha", &a, open, "operator");
    lab.member("beta", &b, open, "operator");
    lab.member("gamma", &c, open, "operator");
    let revoke = |key: &str| serde_json::json!({"v": 1, "kind": "revoke", "key": key, "issued": Utc::now().to_rfc3339()}).to_string();
    let path = lab.write("revocations/gamma.json", &revoke(&c));
    lab.sign(&path, "stranger");
    let (_, out) = lab.run(&["status"]);
    assert!(out.starts_with("Mesh of 3\n"), "an untrusted revocation changes nothing: {out}");
    lab.sign(&path, "operator");
    let (_, out) = lab.run(&["status"]);
    assert!(out.starts_with("Mesh of 2\n") && !out.contains("gamma"), "{out}");
    let (_, check) = lab.run(&["roster", "check"]);
    assert!(check.contains("devices/gamma.json: revoked"), "{check}");
}

#[test]
fn without_a_trusted_signing_key_the_roster_is_not_believed() {
    let lab = Lab::new();
    std::fs::remove_file(lab.root.join("allowed_signers")).unwrap();
    let key = lab.key("alpha");
    lab.member("alpha", &key, 22, "operator");
    let (code, out) = lab.run(&["status"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.starts_with("Mesh of one\n") && out.contains("no trusted signing keys"), "{out}");
}

#[test]
fn a_roster_remote_can_neither_add_nor_change_a_member() {
    let mut lab = Lab::new();
    let open = lab.port(true);
    let (a, b, evil) = (lab.key("alpha"), lab.key("beta"), lab.key("evil"));
    let remote = lab.root.join("remote.git");
    let operator = lab.root.join("operator-clone");
    std::fs::create_dir_all(&remote).unwrap();
    tool("git", &["init", "-q", "--bare", "-b", "main"], &remote);

    // The operator publishes a signed roster.
    tool("git", &["clone", "-q", remote.to_str().unwrap(), operator.to_str().unwrap()], &lab.root);
    for (name, key) in [("alpha", &a), ("beta", &b)] {
        let text = lab.device_json(name, key, open, 90);
        std::fs::create_dir_all(operator.join("devices")).unwrap();
        let path = operator.join(format!("devices/{name}.json"));
        std::fs::write(&path, text).unwrap();
        tool("ssh-keygen", &["-Y", "sign", "-q", "-f", lab.root.join("keys/operator").to_str().unwrap(), "-n", "dojo-mesh-roster", path.to_str().unwrap()], &lab.root);
    }
    let git = |dir: &Path, args: &[&str]| tool("git", &[&["-c", "user.name=t", "-c", "user.email=t@example.com"], args].concat(), dir);
    git(&operator, &["add", "."]);
    git(&operator, &["commit", "-q", "-m", "roster"]);
    git(&operator, &["push", "-q", "origin", "HEAD:main"]);

    // This node's roster directory is a clone of it.
    std::fs::remove_dir_all(lab.root.join("roster")).unwrap();
    tool("git", &["clone", "-q", "-b", "main", remote.to_str().unwrap(), lab.root.join("roster").to_str().unwrap()], &lab.root);
    let (code, out) = lab.run(&["roster", "check"]);
    assert_eq!((code, out.contains("Accepted (2):")), (0, true), "{out}");

    // Someone with write access to the remote adds a member and rewrites an existing one.
    let attacker = lab.root.join("attacker-clone");
    tool("git", &["clone", "-q", "-b", "main", remote.to_str().unwrap(), attacker.to_str().unwrap()], &lab.root);
    let forged = attacker.join("devices/mallory.json");
    std::fs::write(&forged, lab.device_json("mallory", &evil, open, 90)).unwrap();
    tool("ssh-keygen", &["-Y", "sign", "-q", "-f", lab.root.join("keys/stranger").to_str().unwrap(), "-n", "dojo-mesh-roster", forged.to_str().unwrap()], &lab.root);
    std::fs::write(attacker.join("devices/beta.json"), lab.device_json("beta", &evil, open, 90)).unwrap();
    git(&attacker, &["add", "."]);
    git(&attacker, &["commit", "-q", "-m", "innocent"]);
    git(&attacker, &["push", "-q", "origin", "HEAD:main"]);

    let (code, out) = lab.run(&["roster", "sync"]);
    assert_eq!(code, 0, "{out}");
    let (code, out) = lab.run(&["roster", "check"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("devices/mallory.json: not signed by a trusted signing key"), "{out}");
    assert!(out.contains("devices/beta.json: not signed by a trusted signing key"), "{out}");
    let (_, status) = lab.run(&["status"]);
    assert!(!status.contains("mallory"), "{status}");
    assert!(status.starts_with("Mesh of one\n"), "the changed entry is refused, not believed: {status}");

    // Rewriting the remote's history is refused rather than followed.
    git(&attacker, &["reset", "-q", "--hard", "HEAD~1"]);
    git(&attacker, &["commit", "-q", "--allow-empty", "-m", "other history"]);
    git(&attacker, &["push", "-q", "--force", "origin", "HEAD:main"]);
    let (code, out) = lab.run(&["roster", "sync"]);
    assert_eq!(code, 4, "{out}");
    assert!(out.contains("could not pull the roster"), "{out}");
}

#[test]
fn a_denied_node_is_left_out_whatever_the_roster_says() {
    let mut lab = Lab::new();
    let open = lab.port(true);
    let (a, b) = (lab.key("alpha"), lab.key("beta"));
    lab.member("alpha", &a, open, "operator");
    lab.member("beta", &b, open, "operator");
    lab.config("deny = [\"beta\"]\n");
    let (_, out) = lab.run(&["status", "--all"]);
    assert!(out.starts_with("Mesh of one\n") && out.contains("beta  denied"), "{out}");
}

#[test]
fn the_method_and_paths_in_the_configuration_are_checked() {
    let lab = Lab::new();
    for (text, wanted) in [("method = \"carrier-pigeon\"\n", "is not known"), ("method = \"roster\"\nroster-dir = \"relative/dir\"\n", "must be an absolute path"), ("node-name = \"Bad_Name\"\n", "node-name")] {
        std::fs::write(lab.root.join("bad.toml"), text).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_dojo-mesh")).arg("--config").arg(lab.root.join("bad.toml")).arg("status").env_clear().output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{text}");
        assert!(String::from_utf8_lossy(&output.stderr).contains(wanted), "{text}: {}", String::from_utf8_lossy(&output.stderr));
    }
}
