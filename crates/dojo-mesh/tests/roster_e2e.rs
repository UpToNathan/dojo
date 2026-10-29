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
    /// SSH agents started for homes that need to sign with a key they hold only in an agent.
    agents: std::sync::Mutex<Vec<std::process::Child>>,
}

impl Drop for Lab {
    fn drop(&mut self) {
        for agent in self.agents.lock().unwrap().iter_mut() {
            let _ = agent.kill();
            let _ = agent.wait();
        }
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
        let lab = Self { root, _listeners: Vec::new(), agents: std::sync::Mutex::new(Vec::new()) };
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
        self.run_with_path(args, &std::env::var("PATH").unwrap_or_default())
    }

    fn run_with_path(&self, args: &[&str], path: &str) -> (i32, String) {
        let output = Command::new(env!("CARGO_BIN_EXE_dojo-mesh"))
            .arg("--config")
            .arg(self.root.join("mesh.toml"))
            .args(args)
            .env_clear()
            .env("PATH", path)
            .env("HOME", &self.root)
            .env("USER", "pat")
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

    // The update is held back: the roster this node believed stays exactly as it was.
    let head_before = tool("git", &["rev-parse", "HEAD"], &lab.root.join("roster"));
    let (code, out) = lab.run(&["roster", "sync"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("refused the update") && out.contains("devices/mallory.json") && out.contains("devices/beta.json"), "{out}");
    assert_eq!(tool("git", &["rev-parse", "HEAD"], &lab.root.join("roster")), head_before);
    let (code, out) = lab.run(&["roster", "check"]);
    assert_eq!((code, out.contains("Accepted (2):")), (0, true), "{out}");
    let (_, status) = lab.run(&["status"]);
    assert!(!status.contains("mallory") && status.contains("beta"), "{status}");

    // The same files pulled by hand (not through sync) are still refused entry by the checks.
    tool("git", &["pull", "-q", "--ff-only"], &lab.root.join("roster"));
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


/// `ssh` and `ssh-add` that record what they were given instead of connecting, in `bin/`.
fn fake_tools(lab: &Lab, offered_key: &str) -> String {
    use std::os::unix::fs::PermissionsExt;
    let bin = lab.root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let script = |name: &str, body: String| {
        let path = bin.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    };
    script("ssh", format!("printf '%s\\n' \"$@\" > '{}/ssh-args'; echo remote-output", lab.root.display()));
    script("ssh-add", format!("echo '{offered_key}'"));
    format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default())
}

#[test]
fn members_are_let_in_by_a_marked_block_that_never_includes_this_node_and_leaving_removes_it() {
    let mut lab = Lab::new();
    let open = lab.port(true);
    let (a, b) = (lab.key("alpha"), lab.key("beta"));
    lab.member("alpha", &a, open, "operator");
    let input = lab.key("beta-input");
    let entry = serde_json::json!({
        "v": 1, "kind": "device", "name": "beta", "node_key": b, "input_key": input, "host_key": lab.public("operator"),
        "routes": [{"host": "127.0.0.1", "port": open}],
        "issued": (Utc::now() - Duration::hours(1)).to_rfc3339(), "expires": (Utc::now() + Duration::days(90)).to_rfc3339(),
    });
    let path = lab.write("devices/beta.json", &entry.to_string());
    lab.sign(&path, "operator");
    std::fs::create_dir_all(lab.root.join(".ssh")).unwrap();
    std::fs::write(lab.root.join(".ssh/authorized_keys"), "ssh-rsa AAAAmine me\n").unwrap();

    let (code, out) = lab.run(&["roster", "authorize"]);
    assert_eq!(code, 0, "{out}");
    let keys = std::fs::read_to_string(lab.root.join(".ssh/authorized_keys")).unwrap();
    assert!(keys.starts_with("ssh-rsa AAAAmine me\n# BEGIN dojo-mesh"), "{keys}");
    assert!(keys.contains(&format!("{b} dojo-mesh:beta\n")), "{keys}");
    assert!(keys.contains(&format!("restrict,command=\"{} inject --as beta\" {input} dojo-mesh:beta:input", std::fs::canonicalize(env!("CARGO_BIN_EXE_dojo-mesh")).unwrap().display())), "{keys}");
    assert!(!keys.contains(&a), "this node's own key must not be let in: {keys}");

    // Revoking beta's node key takes it out of the block on the next authorize.
    let revoke = lab.write("revocations/beta.json", &serde_json::json!({"v": 1, "kind": "revoke", "key": b, "issued": Utc::now().to_rfc3339()}).to_string());
    lab.sign(&revoke, "operator");
    assert_eq!(lab.run(&["roster", "authorize"]).0, 0);
    let keys = std::fs::read_to_string(lab.root.join(".ssh/authorized_keys")).unwrap();
    assert_eq!(keys, "ssh-rsa AAAAmine me\n");
}

#[test]
fn exec_goes_over_the_system_ssh_with_the_pinned_host_key_and_a_locked_agent_is_a_step_for_a_person() {
    let mut lab = Lab::new();
    let open = lab.port(true);
    let (a, b) = (lab.key("alpha"), lab.key("beta"));
    lab.member("alpha", &a, open, "operator");
    lab.member("beta", &b, open, "operator");
    std::fs::write(lab.root.join("agent.sock"), "").unwrap();
    lab.config(&format!("node-key-file = \"{}\"\nagent-socket = \"{}\"\nallow = [\"beta\"]\n", lab.root.join("keys/alpha.pub").display(), lab.root.join("agent.sock").display()));

    let path = fake_tools(&lab, &format!("{a} alpha"));
    let (code, out) = lab.run_with_path(&["exec", "beta", "--", "echo", "hi"], &path);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("remote-output"), "{out}");
    let args = std::fs::read_to_string(lab.root.join("ssh-args")).unwrap();
    assert!(args.contains("HostKeyAlias=dojo-beta") && args.contains("StrictHostKeyChecking=yes") && args.contains("\n-l\npat\n"), "{args}");
    assert!(args.contains(&format!("\n-p\n{open}\n--\n127.0.0.1\n")), "{args}");
    let pinned = std::fs::read_to_string(lab.root.join(".local/share/dojo/mesh/known_hosts/beta")).unwrap();
    assert_eq!(pinned, format!("dojo-beta {}\n", lab.public("operator")));

    // 1Password locked: the agent lists nothing. No connection is made and the exit status is "needs a person".
    let _ = std::fs::remove_file(lab.root.join("ssh-args"));
    let path = fake_tools(&lab, "");
    let (code, out) = lab.run_with_path(&["exec", "beta", "--", "echo", "hi"], &path);
    assert_eq!(code, 3, "{out}");
    assert!(out.contains("unlock 1Password"), "{out}");
    assert!(!lab.root.join("ssh-args").exists());
}

#[test]
fn input_is_attributed_by_the_roster_and_only_to_a_member() {
    let mut lab = Lab::new();
    let open = lab.port(true);
    let (a, b) = (lab.key("alpha"), lab.key("beta"));
    lab.member("alpha", &a, open, "operator");
    lab.member("beta", &b, open, "operator");
    let run = |args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_dojo-mesh"))
            .arg("--config")
            .arg(lab.root.join("mesh.toml"))
            .args(args)
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", &lab.root)
            .env("SSH_CONNECTION", "203.0.113.5 50000 203.0.113.9 22")
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap();
        (output.status.code().unwrap(), format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr)))
    };
    // Nobody named: the address alone says nothing under this method.
    let (code, out) = run(&["inject"]);
    assert_eq!(code, 3, "{out}");
    assert!(out.contains("did not name its sender"), "{out}");
    // Named, but not in the roster.
    let (code, out) = run(&["inject", "--as", "stranger"]);
    assert_eq!(code, 3, "{out}");
    assert!(out.contains("stranger is not in the signed roster"), "{out}");
    // A member, but this node has not agreed to take its input.
    let (code, out) = run(&["inject", "--as", "beta"]);
    assert_eq!(code, 3, "{out}");
    assert!(out.contains("does not accept input from beta"), "{out}");
    // This node is not a sender either.
    let (code, out) = run(&["inject", "--as", "alpha"]);
    assert_eq!(code, 3, "{out}");
    assert!(out.contains("alpha is not in the signed roster"), "{out}");
}

#[test]
fn doctor_warns_when_the_key_holder_is_not_ready() {
    let mut lab = Lab::new();
    let open = lab.port(true);
    let a = lab.key("alpha");
    lab.member("alpha", &a, open, "operator");
    lab.config(&format!("node-key-file = \"{}\"\nagent-socket = \"{}\"\n", lab.root.join("keys/alpha.pub").display(), lab.root.join("agent.sock").display()));
    let (code, out) = lab.run(&["doctor"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("warning: Peers cannot be reached from here yet") && out.contains("not running"), "{out}");
    std::fs::write(lab.root.join("agent.sock"), "").unwrap();
    let path = fake_tools(&lab, &format!("{a} alpha"));
    let (code, out) = lab.run_with_path(&["doctor"], &path);
    assert!(code == 0 && !out.contains("Peers cannot be reached"), "{out}");
}

impl Lab {
    /// Runs the binary as another node: its own config file and home.
    fn run_as(&self, config: &Path, home: &Path, args: &[&str], path: &str) -> (i32, String) {
        let output = Command::new(env!("CARGO_BIN_EXE_dojo-mesh"))
            .arg("--config")
            .arg(config)
            .args(args)
            .env_clear()
            .env("PATH", path)
            .env("HOME", home)
            .env("USER", "pat")
            .current_dir(&self.root)
            .output()
            .unwrap();
        (output.status.code().unwrap(), format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr)))
    }

    /// Starts an SSH agent at `home`'s default socket (where 1Password's would be) holding the private
    /// keys named, so a node whose key exists only as a public file can still sign.
    fn agent_with(&self, home: &Path, keys: &[&str]) {
        let socket = home.join(".1password/agent.sock");
        if socket.exists() {
            return;
        }
        std::fs::create_dir_all(socket.parent().unwrap()).unwrap();
        let child = Command::new("ssh-agent").arg("-D").arg("-a").arg(&socket).stdout(std::process::Stdio::null()).spawn().expect("ssh-agent starts");
        self.agents.lock().unwrap().push(child);
        for _ in 0..200 {
            if socket.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        for key in keys {
            let status = Command::new("ssh-add").arg("-q").arg(self.root.join("keys").join(key)).env("SSH_AUTH_SOCK", &socket).status().expect("ssh-add runs");
            assert!(status.success(), "{key}");
        }
    }

    /// A signed join request that machine `name` would make with the node key `keys/NODE.pub`
    /// (whose private half is beside it), written to `OUT` in the lab's directory.
    fn join_request(&self, name: &str, node: &str, out: &str, host_key: &Path, port: u16) -> PathBuf {
        let config = self.root.join(format!("{name}-{node}-join.toml"));
        std::fs::write(
            &config,
            format!(
                "method = \"roster\"\nnode-name = \"{name}\"\nnode-key-file = \"{}\"\nsigners-file = \"{}\"\nroster-dir = \"{}\"\n",
                self.root.join(format!("keys/{node}.pub")).display(),
                self.root.join("allowed_signers").display(),
                self.root.join("roster").display()
            ),
        )
        .unwrap();
        let path = std::env::var("PATH").unwrap_or_default();
        let (code, text) = self.run_as(&config, &self.root, &["join", "--route", &format!("127.0.0.1:{port}"), "--host-key-file", host_key.to_str().unwrap(), "--out", out], &path);
        assert_eq!(code, 0, "{text}");
        self.root.join(out)
    }

    fn fingerprint(&self, key: &str) -> String {
        let out = tool("ssh-keygen", &["-l", "-f", self.root.join("keys").join(format!("{key}.pub")).to_str().unwrap()], &self.root);
        out.split_whitespace().nth(1).unwrap().to_string()
    }
}

#[test]
fn a_node_is_enrolled_renewed_and_removed_with_fingerprints_compared_and_every_change_signed() {
    let mut lab = Lab::new();
    let open = lab.port(true);
    let (a, b) = (lab.key("alpha"), lab.key("beta"));
    lab.key("beta-input");
    lab.member("alpha", &a, open, "operator");
    let path = std::env::var("PATH").unwrap_or_default();
    // The operator signs with a key file; the roster directory is a git repository with no remote.
    tool("git", &["init", "-q", "roster"], &lab.root);
    tool("git", &["config", "user.email", "operator@example.com"], &lab.root.join("roster"));
    tool("git", &["config", "user.name", "operator"], &lab.root.join("roster"));
    tool("git", &["config", "commit.gpgsign", "false"], &lab.root.join("roster"));
    lab.config(&format!("signing-keys = [\"{}\"]\n", lab.root.join("keys/operator").display()));

    // Beta describes itself.
    let beta_config = lab.root.join("beta.toml");
    std::fs::write(
        &beta_config,
        format!(
            "method = \"roster\"\nnode-name = \"beta\"\nnode-key-file = \"{}\"\ninput-key-file = \"{}\"\nsigners-file = \"{}\"\nroster-dir = \"{}\"\n",
            lab.root.join("keys/beta.pub").display(),
            lab.root.join("keys/beta-input.pub").display(),
            lab.root.join("allowed_signers").display(),
            lab.root.join("roster").display()
        ),
    )
    .unwrap();
    let host_key = lab.root.join("keys/operator.pub");
    let (code, out) = lab.run_as(&beta_config, &lab.root, &["join", "--route", &format!("127.0.0.1:{open}"), "--host-key-file", host_key.to_str().unwrap(), "--out", "beta.join.json"], &path);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains(&lab.fingerprint("beta")), "{out}");
    let request = lab.root.join("beta.join.json");

    // Approval needs the fingerprint a person compared.
    let (code, out) = lab.run(&["approve", request.to_str().unwrap()]);
    assert_eq!(code, 3, "{out}");
    assert!(out.contains(&lab.fingerprint("beta")) && !lab.root.join("roster/devices/beta.json").exists(), "{out}");
    let (code, out) = lab.run(&["approve", request.to_str().unwrap(), "--fingerprint", &lab.fingerprint("alpha")]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("does not match") && !lab.root.join("roster/devices/beta.json").exists(), "{out}");
    let (code, out) = lab.run(&["approve", request.to_str().unwrap(), "--fingerprint", &lab.fingerprint("beta")]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("approved beta") && out.contains("no remote"), "{out}");
    let (code, out) = lab.run(&["roster", "check"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("beta") && out.contains("Ignored: nothing"), "{out}");
    let keys = std::fs::read_to_string(lab.root.join(".ssh/authorized_keys")).unwrap();
    assert!(keys.contains(&format!("{b} dojo-mesh:beta")), "{keys}");
    assert!(keys.contains("inject --as beta"), "{keys}");

    // A different machine under the same name is not accepted by accident.
    let impostor_file = lab.join_request("beta", "stranger", "impostor.join.json", &host_key, open);
    let (code, out) = lab.run(&["approve", impostor_file.to_str().unwrap(), "--fingerprint", &lab.fingerprint("stranger")]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("different key"), "{out}");
    // A request edited on the way is refused before anything is compared, even with the right key's fingerprint.
    let edited = std::fs::read_to_string(&request).unwrap().replace(&lab.public("operator"), &lab.public("stranger"));
    let edited_file = lab.root.join("edited.join.json");
    std::fs::write(&edited_file, edited).unwrap();
    let (code, out) = lab.run(&["approve", edited_file.to_str().unwrap(), "--fingerprint", &lab.fingerprint("beta")]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("signature does not match") && out.contains("Do not approve"), "{out}");
    // An unsigned request from an older version is turned away with the way forward.
    let old_file = lab.root.join("old.join.json");
    std::fs::write(&old_file, serde_json::json!({"v": 1, "name": "beta", "node_key": b, "host_key": lab.public("operator")}).to_string()).unwrap();
    let (code, out) = lab.run(&["approve", old_file.to_str().unwrap(), "--fingerprint", &lab.fingerprint("beta")]);
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("unsigned") && out.contains("dojo-mesh join"), "{out}");

    // Renewal keeps the node and moves its expiry.
    let (code, out) = lab.run(&["renew", "beta", "--days", "30"]);
    assert_eq!(code, 0, "{out}");
    let renewed = std::fs::read_to_string(lab.root.join("roster/devices/beta.json")).unwrap();
    let expires: chrono::DateTime<Utc> = serde_json::from_str::<serde_json::Value>(&renewed).unwrap()["expires"].as_str().unwrap().parse().unwrap();
    assert!((expires - Utc::now()).num_days() <= 30 && (expires - Utc::now()).num_days() >= 29);
    assert_eq!(lab.run(&["roster", "check"]).0, 0);

    // Removal revokes every key it held and lets the node out of authorized_keys.
    let (code, out) = lab.run(&["rm", "beta", "--reason", "lost"]);
    assert_eq!(code, 0, "{out}");
    assert!(!lab.root.join("roster/devices/beta.json").exists());
    assert_eq!(std::fs::read_dir(lab.root.join("roster/revocations")).unwrap().flatten().filter(|e| e.path().extension().is_some_and(|x| x == "json")).count(), 3);
    let (_, out) = lab.run(&["roster", "check"]);
    assert!(!out.contains("beta"), "{out}");
    assert!(!std::fs::read_to_string(lab.root.join(".ssh/authorized_keys")).unwrap().contains("beta"));
    let (code, out) = lab.run(&["approve", request.to_str().unwrap(), "--fingerprint", &lab.fingerprint("beta")]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("revoked"), "{out}");
    // Everything the operator did is in the roster's history.
    let log = tool("git", &["log", "--format=%s"], &lab.root.join("roster"));
    assert!(log.contains("Enroll beta") && log.contains("Renew beta") && log.contains("Remove beta"), "{log}");
}

#[test]
fn nothing_is_signed_without_a_signing_key_that_works() {
    let mut lab = Lab::new();
    let open = lab.port(true);
    let (a, b) = (lab.key("alpha"), lab.key("beta"));
    lab.member("alpha", &a, open, "operator");
    lab.key("beta-input");
    let _ = &b;
    let request = lab.join_request("beta", "beta", "beta.join.json", &lab.root.join("keys/operator.pub"), open);
    let fp = lab.fingerprint("beta");
    // No signing keys set.
    let (code, out) = lab.run(&["approve", request.to_str().unwrap(), "--fingerprint", &fp]);
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("no signing key is set") && !lab.root.join("roster/devices/beta.json").exists(), "{out}");
    // A listed key that cannot sign (an agent-held key with no agent) is a step for a person, and leaves nothing behind.
    lab.config(&format!("signing-keys = [\"{}\"]\nagent-socket = \"{}\"\n", lab.root.join("keys/operator.pub").display(), lab.root.join("no-agent.sock").display()));
    let (code, out) = lab.run(&["approve", request.to_str().unwrap(), "--fingerprint", &fp]);
    assert_eq!(code, 3, "{out}");
    assert!(out.contains("no signing key could sign") && !lab.root.join("roster/devices/beta.json").exists() && !lab.root.join("roster/devices/beta.json.sig").exists(), "{out}");
    // The first key that works wins, in the order listed.
    lab.config(&format!("signing-keys = [\"{}\", \"{}\"]\nagent-socket = \"{}\"\n", lab.root.join("keys/operator.pub").display(), lab.root.join("keys/operator").display(), lab.root.join("no-agent.sock").display()));
    let (code, out) = lab.run(&["approve", request.to_str().unwrap(), "--fingerprint", &fp]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(lab.run(&["roster", "check"]).0, 0);
}

#[test]
fn a_node_trusts_a_signing_key_only_after_the_fingerprint_it_was_shown_matches() {
    let lab = Lab::new();
    std::fs::remove_file(lab.root.join("allowed_signers")).unwrap();
    let new = lab.key("replacement");
    let fp = lab.fingerprint("replacement");
    let pub_file = lab.root.join("keys/replacement.pub");
    let (code, out) = lab.run(&["trust", "add", pub_file.to_str().unwrap()]);
    assert_eq!(code, 2, "{out}");
    let (code, out) = lab.run(&["trust", "add", pub_file.to_str().unwrap(), "--fingerprint", &lab.fingerprint("operator")]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("It was not trusted") && !lab.root.join("allowed_signers").exists(), "{out}");
    let (code, out) = lab.run(&["trust", "add", pub_file.to_str().unwrap(), "--fingerprint", &fp]);
    assert_eq!(code, 0, "{out}");
    let (code, out) = lab.run(&["trust", "list"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains(&fp) && out.contains("software"), "{out}");
    assert!(std::fs::read_to_string(lab.root.join("allowed_signers")).unwrap().contains(&new));
    // Only software-held keys are trusted, and doctor says so plainly.
    let (_, out) = lab.run(&["doctor"]);
    assert!(out.contains("held in software"), "{out}");
    // The old key can be dropped by its fingerprint.
    assert_eq!(lab.run(&["trust", "remove", &fp]).0, 0);
    assert_eq!(std::fs::read_to_string(lab.root.join("allowed_signers")).unwrap(), "");
    assert_eq!(lab.run(&["trust", "remove", &fp]).0, 1);
}

#[test]
fn init_makes_the_keys_in_1password_or_registers_ones_made_elsewhere() {
    use std::os::unix::fs::PermissionsExt;
    let lab = Lab::new();
    let bin = lab.root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let key = lab.public("operator");
    let item = serde_json::json!({"id": "i", "fields": [{"id": "public_key", "label": "public key", "value": key}]}).to_string();
    let op = bin.join("op");
    std::fs::write(&op, format!("#!/bin/sh\necho \"$@\" >> '{}/op-calls'\necho '{}'\n", lab.root.display(), item)).unwrap();
    std::fs::set_permissions(&op, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default());
    lab.config(&format!("node-key-file = \"{}\"\ninput-key-file = \"{}\"\n", lab.root.join("conf/node.pub").display(), lab.root.join("conf/input.pub").display()));
    let (code, out) = lab.run_with_path(&["init", "--vault", "Private"], &path);
    assert_eq!(code, 0, "{out}");
    assert!(std::fs::read_to_string(lab.root.join("conf/node.pub")).unwrap().starts_with(&format!("{key} dojo-mesh:alpha")));
    assert!(std::fs::read_to_string(lab.root.join("conf/input.pub")).unwrap().contains("dojo-mesh:alpha:input"));
    let calls = std::fs::read_to_string(lab.root.join("op-calls")).unwrap();
    assert_eq!(calls.lines().count(), 2, "{calls}");
    assert!(calls.contains("SSH Key") && calls.contains("--ssh-generate-key Ed25519") && calls.contains("--vault Private") && calls.contains("node key (alpha)") && calls.contains("input key (alpha)"), "{calls}");
    // Keys that exist are never replaced.
    let (code, out) = lab.run_with_path(&["init"], &path);
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("already has keys"), "{out}");
    // Without 1Password's command line, the way out is named.
    let (code, out) = lab.run_with_path(&["init", "--custody", "1password"], "/nonexistent");
    assert_eq!(code, 2);
    std::fs::remove_dir_all(lab.root.join("conf")).unwrap();
    let (code, out2) = lab.run_with_path(&["init"], "/nonexistent");
    assert_eq!(code, 4, "{out2}");
    assert!(out2.contains("--custody existing"), "{out2}");
    let _ = out;
    // Keys made somewhere else (a YubiKey, another agent) are registered by their public halves.
    let node = lab.root.join("keys/operator.pub");
    let (code, out) = lab.run_with_path(&["init", "--custody", "existing", "--node-key", node.to_str().unwrap()], &path);
    assert_eq!(code, 0, "{out}");
    assert!(lab.root.join("conf/node.pub").exists() && !lab.root.join("conf/input.pub").exists());
}

/// Runs the binary on its own configuration file, with fake `op` and `ssh-keygen` ahead on the path.
/// The fake `ssh-keygen` has no hardware: asked for a security-key credential it makes a plain one,
/// and it finds no earlier credential on the "key".
fn onboarding_run(lab: &Lab, config: &Path, home: &Path, args: &[&str]) -> (i32, String) {
    // No terminal here; a way to ask for a PIN stands in for one.
    onboarding_run_with(lab, config, home, args, &[("SSH_ASKPASS", "/bin/true")])
}

fn onboarding_run_with(lab: &Lab, config: &Path, home: &Path, args: &[&str], extra_env: &[(&str, &str)]) -> (i32, String) {
    use std::os::unix::fs::PermissionsExt;
    let bin = lab.root.join("onboard-bin");
    if !bin.exists() {
        std::fs::create_dir_all(&bin).unwrap();
        let items: Vec<(&str, String)> = ["node", "input", "signing"].iter().map(|kind| (*kind, lab.key(&format!("op-{kind}")))).collect();
        let mut op = String::from("#!/bin/sh\necho \"$@\" >> \"$OP_CALLS\"\n");
        for (kind, key) in &items {
            let item = serde_json::json!({"id": "i", "fields": [{"id": "public_key", "label": "public key", "value": key}]});
            op.push_str(&format!("case \"$*\" in *'{kind} key'*) echo '{item}'; exit 0;; esac\n"));
        }
        op.push_str("exit 1\n");
        std::fs::write(bin.join("op"), op).unwrap();
        let real = tool("sh", &["-c", "command -v ssh-keygen"], &lab.root).trim().to_string();
        let keygen = format!(
            "#!/bin/sh\ncase \"$1\" in -K) echo 'no resident keys' >&2; exit 255;; esac\n\
             if [ \"$1\" = -t ] && [ \"$2\" = ed25519-sk ]; then\n  while [ $# -gt 0 ]; do case \"$1\" in -f) f=$2; shift;; -C) c=$2; shift;; esac; shift; done\n  exec '{real}' -q -t ed25519 -N '' -C \"$c\" -f \"$f\"\nfi\nexec '{real}' \"$@\"\n"
        );
        std::fs::write(bin.join("ssh-keygen"), keygen).unwrap();
        for name in ["op", "ssh-keygen"] {
            std::fs::set_permissions(bin.join(name), std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    // The node key made "in 1Password" is held by an agent at 1Password's default socket.
    lab.agent_with(home, &["op-node", "op-input", "op-signing"]);
    let output = Command::new(env!("CARGO_BIN_EXE_dojo-mesh"))
        .arg("--config")
        .arg(config)
        .args(args)
        .env_clear()
        .env("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()))
        .env("HOME", home)
        .env("USER", "pat")
        .env("OP_CALLS", lab.root.join("op-calls"))
        .envs(extra_env.iter().copied())
        .output()
        .unwrap();
    (output.status.code().unwrap(), format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr)))
}

#[test]
fn onboarding_sets_a_machine_up_in_order_and_is_safe_to_repeat() {
    let lab = Lab::new();
    let home = lab.root.join("home");
    let config = lab.root.join("onboard.toml");
    std::fs::write(&config, "# mine\naccept-input = [\"harbor\"]\n\n[capability.voice]\ncommand = \"/bin/true\"\n").unwrap();
    let host = lab.root.join("keys/operator.pub");
    let args = ["onboard", "--node-name", "orchard", "--route", "192.0.2.10:2222", "--sign", "yubikey", "--sign", "1password", "--host-key-file", host.to_str().unwrap()];
    let (code, out) = onboarding_run(&lab, &config, &home, &args);
    assert_eq!(code, 0, "{out}");

    // The configuration keeps what was there, and lists the signing keys with the YubiKey first.
    let text = std::fs::read_to_string(&config).unwrap();
    assert!(text.starts_with("# mine\naccept-input = [\"harbor\"]\n"), "{text}");
    assert!(text.contains("[capability.voice]\ncommand = \"/bin/true\""), "{text}");
    let conf = toml::from_str::<toml::Table>(&text).unwrap();
    assert_eq!(conf["method"].as_str(), Some("roster"));
    assert_eq!(conf["node-name"].as_str(), Some("orchard"));
    let keys: Vec<&str> = conf["signing-keys"].as_array().unwrap().iter().map(|key| key.as_str().unwrap()).collect();
    let mesh = home.join(".config/dojo/mesh");
    assert_eq!(keys, [mesh.join("signing_yubikey").to_str().unwrap(), mesh.join("signing_1password.pub").to_str().unwrap()]);

    // Its own keys come from 1Password, and the signing key there is made too (three items in all).
    assert!(mesh.join("node_key.pub").exists() && mesh.join("input_key.pub").exists());
    let calls = std::fs::read_to_string(lab.root.join("op-calls")).unwrap();
    assert_eq!(calls.lines().count(), 3, "{calls}");
    assert!(calls.contains("signing key (orchard)"), "{calls}");

    // Both signing keys are believed, and the node is signed into its own roster by the first.
    let signers = std::fs::read_to_string(mesh.join("allowed_signers")).unwrap();
    assert_eq!(signers.lines().count(), 2, "{signers}");
    let roster = home.join(".local/share/dojo/mesh/roster/devices");
    assert!(roster.join("orchard.json").exists() && roster.join("orchard.json.sig").exists());
    assert!(std::fs::read_to_string(roster.join("orchard.json")).unwrap().contains("192.0.2.10"));
    assert!(!mesh.join("orchard.join.json").exists(), "the request was a step, not a leftover");
    let (code, check) = onboarding_run(&lab, &config, &home, &["roster", "check"]);
    assert_eq!(code, 0, "{check}");

    // Again: nothing is made twice, nothing changes.
    let before = (text.clone(), signers.clone());
    let (code, out) = onboarding_run(&lab, &config, &home, &args);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("already"), "{out}");
    assert_eq!(std::fs::read_to_string(lab.root.join("op-calls")).unwrap().lines().count(), 3);
    assert_eq!((std::fs::read_to_string(&config).unwrap(), std::fs::read_to_string(mesh.join("allowed_signers")).unwrap()), before);

    // A node that signs nothing writes the request an operator signs, and says what comes next.
    let guest_home = lab.root.join("guest-home");
    let guest_config = lab.root.join("guest.toml");
    let (code, out) = onboarding_run(&lab, &guest_config, &guest_home, &["onboard", "--node-name", "guest", "--route", "192.0.2.11:2222", "--host-key-file", host.to_str().unwrap()]);
    assert_eq!(code, 0, "{out}");
    let request = guest_home.join(".config/dojo/mesh/guest.join.json");
    assert!(request.exists(), "{out}");
    assert!(out.contains("dojo-mesh approve guest.join.json"), "{out}");
    assert!(!std::fs::read_to_string(&guest_config).unwrap().contains("signing-keys"));
    let (code, out) = onboarding_run(&lab, &config, &home, &["approve", request.to_str().unwrap()]);
    assert_eq!(code, 3, "{out}");
}

#[test]
fn onboarding_refuses_what_it_cannot_do_safely() {
    let lab = Lab::new();
    let home = lab.root.join("home");
    let config = lab.root.join("onboard.toml");
    // A signing list spread over several lines is the person's to edit; nothing is written.
    let original = "signing-keys = [\n  \"/a\",\n]\n";
    std::fs::write(&config, original).unwrap();
    let host = lab.root.join("keys/operator.pub");
    let (code, out) = onboarding_run(&lab, &config, &home, &["onboard", "--node-name", "orchard", "--sign", "yubikey", "--host-key-file", host.to_str().unwrap()]);
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("by hand"), "{out}");
    assert!(std::fs::read_to_string(&config).unwrap().contains("method"), "method is set before anything else");
    // An unknown way of signing is a usage error.
    std::fs::write(&config, "").unwrap();
    let (code, out) = onboarding_run(&lab, &config, &home, &["onboard", "--node-name", "orchard", "--sign", "tpm"]);
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("yubikey, 1password"), "{out}");
    // Without a terminal or a way to ask for a PIN, the YubiKey is not touched at all.
    std::fs::write(&config, "").unwrap();
    let fresh = lab.root.join("fresh-home");
    let (code, out) = onboarding_run_with(&lab, &config, &fresh, &["onboard", "--node-name", "orchard", "--sign", "yubikey", "--host-key-file", host.to_str().unwrap()], &[]);
    assert_eq!(code, 3, "{out}");
    assert!(out.contains("no terminal"), "{out}");
    assert!(!fresh.join(".config/dojo/mesh/signing_yubikey").exists());
    // A name the roster would not accept is refused before the configuration is touched.
    std::fs::write(&config, "").unwrap();
    let (code, _) = onboarding_run(&lab, &config, &home, &["onboard", "--node-name", "A"]);
    assert_eq!(code, 2);
    assert_eq!(std::fs::read_to_string(&config).unwrap(), "");
}

#[test]
fn the_roster_travels_between_members_and_only_what_verifies_is_kept() {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    let mut lab = Lab::new();
    let open = lab.port(true);
    let (a, b) = (lab.key("alpha"), lab.key("beta"));
    lab.member("alpha", &a, open, "operator");
    lab.member("beta", &b, open, "operator");
    std::fs::write(lab.root.join("agent.sock"), "").unwrap();
    lab.config(&format!(
        "node-key-file = \"{}\"\nagent-socket = \"{}\"\nsigning-keys = [\"{}\"]\n",
        lab.root.join("keys/alpha.pub").display(),
        lab.root.join("agent.sock").display(),
        lab.root.join("keys/operator").display()
    ));
    // Beta is a second node with its own, empty roster directory; the "ssh" to beta runs beta's dojo-mesh.
    let beta_dir = lab.root.join("beta-roster");
    let beta_config = lab.root.join("beta.toml");
    std::fs::write(&beta_config, format!("method = \"roster\"\nnode-name = \"beta\"\nsigners-file = \"{}\"\nroster-dir = \"{}\"\n", lab.root.join("allowed_signers").display(), beta_dir.display())).unwrap();
    let bin = lab.root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let ssh = format!(
        "#!/bin/sh\nfor last; do :; done\ncase \"$last\" in *'roster export'*) exec '{0}' --config '{1}' roster export;; *'roster receive'*) exec '{0}' --config '{1}' roster receive;; esac\nexit 9\n",
        env!("CARGO_BIN_EXE_dojo-mesh"),
        beta_config.display()
    );
    std::fs::write(bin.join("ssh"), ssh).unwrap();
    std::fs::write(bin.join("ssh-add"), format!("#!/bin/sh\necho '{a} alpha'\n")).unwrap();
    for name in ["ssh", "ssh-add"] {
        std::fs::set_permissions(bin.join(name), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default());
    let on_beta = |args: &[&str], stdin: &str| -> (i32, String) {
        let mut child = Command::new(env!("CARGO_BIN_EXE_dojo-mesh"))
            .arg("--config")
            .arg(&beta_config)
            .args(args)
            .env_clear()
            .env("PATH", &path)
            .env("HOME", &lab.root)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
        let output = child.wait_with_output().unwrap();
        (output.status.code().unwrap(), format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr)))
    };

    // A first copy by hand: export here, receive there.
    let (code, bundle) = lab.run_with_path(&["roster", "export"], &path);
    assert_eq!(code, 0, "{bundle}");
    let (code, out) = on_beta(&["roster", "receive"], &bundle);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("2 added"), "{out}");
    assert!(beta_dir.join("devices/alpha.json.sig").exists() && beta_dir.join("devices/beta.json").exists());

    // Beta has a member alpha does not, and a file signed by a stranger. Alpha takes the first only.
    let sign_in_beta = |name: &str, signer: &str, body: String| {
        let path = beta_dir.join(format!("devices/{name}.json"));
        std::fs::write(&path, body).unwrap();
        let _ = std::fs::remove_file(format!("{}.sig", path.display()));
        tool("ssh-keygen", &["-Y", "sign", "-q", "-f", lab.root.join("keys").join(signer).to_str().unwrap(), "-n", "dojo-mesh-roster", path.to_str().unwrap()], &lab.root);
    };
    let gamma = lab.key("gamma");
    sign_in_beta("gamma", "operator", lab.device_json("gamma", &gamma, open, 90));
    sign_in_beta("evil", "stranger", lab.device_json("evil", &gamma, open, 90));
    let (code, out) = lab.run_with_path(&["roster", "sync"], &path);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("beta: 1 added") && out.contains("devices/evil.json"), "{out}");
    assert!(lab.root.join("roster/devices/gamma.json").exists());
    assert!(!lab.root.join("roster/devices/evil.json").exists(), "a file no trusted key signed is never written");
    let (_, status) = lab.run_with_path(&["status", "--all"], &path);
    assert!(status.contains("gamma") && !status.contains("evil"), "{status}");

    // A change here reaches beta by itself, and an older copy never replaces a newer one.
    std::fs::remove_file(beta_dir.join("devices/evil.json")).unwrap();
    std::fs::remove_file(beta_dir.join("devices/evil.json.sig")).unwrap();
    let before = std::fs::read_to_string(beta_dir.join("devices/beta.json")).unwrap();
    let (code, out) = lab.run_with_path(&["renew", "beta", "--days", "30"], &path);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("sent the roster to beta"), "{out}");
    assert_ne!(std::fs::read_to_string(beta_dir.join("devices/beta.json")).unwrap(), before);
    assert_eq!(std::fs::read_to_string(beta_dir.join("devices/beta.json")).unwrap(), std::fs::read_to_string(lab.root.join("roster/devices/beta.json")).unwrap());
    let (code, out) = on_beta(&["roster", "receive"], &bundle);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("0 added, 0 updated"), "the old bundle changes nothing: {out}");
}

#[test]
fn a_machine_joins_through_an_invite_with_nothing_carried_by_hand() {
    use std::process::Stdio;
    let mut lab = Lab::new();
    let open = lab.port(true);
    lab.key("alpha");
    lab.key("beta");
    let a = lab.public("alpha");
    lab.member("alpha", &a, open, "operator");
    let carrier = lab.root.join("carrier");
    lab.config(&format!("signing-keys = [\"{}\"]\nrendezvous = \"directory\"\nrendezvous-dir = \"{}\"\n", lab.root.join("keys/operator").display(), carrier.display()));
    let path = std::env::var("PATH").unwrap_or_default();

    // The new machine: its own roster directory and no trusted signing key at all.
    let beta_config = lab.root.join("beta.toml");
    std::fs::write(
        &beta_config,
        format!(
            "method = \"roster\"\nnode-name = \"beta\"\nnode-key-file = \"{}\"\nsigners-file = \"{}\"\nroster-dir = \"{}\"\nrendezvous = \"directory\"\nrendezvous-dir = \"{}\"\n",
            lab.root.join("keys/beta.pub").display(),
            lab.root.join("beta-signers").display(),
            lab.root.join("beta-roster").display(),
            carrier.display()
        ),
    )
    .unwrap();
    let host_key = lab.root.join("keys/operator.pub");
    let join = |wait: &str| {
        Command::new(env!("CARGO_BIN_EXE_dojo-mesh"))
            .arg("--config")
            .arg(&beta_config)
            .args(["join", "--invite", "--route", &format!("127.0.0.1:{open}"), "--host-key-file", host_key.to_str().unwrap(), "--poll", "100", "--wait", wait])
            .env_clear()
            .env("PATH", &path)
            .env("HOME", &lab.root)
            .env("USER", "pat")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    };
    let finish = |child: std::process::Child| {
        let output = child.wait_with_output().unwrap();
        (output.status.code().unwrap(), format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr)))
    };

    // With no invite the new machine says so and gives up.
    let (code, out) = finish(join("1"));
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("no invite was waiting"), "{out}");

    // The operator invites; the new machine finds it by itself and asks.
    let (code, out) = lab.run(&["invite", "--json"]);
    assert_eq!(code, 0, "{out}");
    let id = serde_json::from_str::<serde_json::Value>(out.lines().last().unwrap()).unwrap()["id"].as_str().unwrap().to_string();
    let joining = join("30");
    let mut waiting = String::new();
    for _ in 0..100 {
        let (_, listed) = lab.run(&["requests", "--json"]);
        waiting = listed.lines().last().unwrap_or_default().to_string();
        if waiting.contains("beta") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(waiting.contains("\"name\":\"beta\"") && waiting.contains(&id), "{waiting}");
    let (_, text) = lab.run(&["requests"]);
    assert!(text.contains("beta") && text.contains("wants to join"), "{text}");

    // Approval signs the entry and sends the roster; the new machine installs it and trusts the signer.
    let (code, out) = lab.run(&["approve", "--invite", &id]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("approved beta") && out.contains("beta can finish joining"), "{out}");
    let (code, out) = finish(joining);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("beta joined alpha's mesh"), "{out}");
    let signers = std::fs::read_to_string(lab.root.join("beta-signers")).unwrap();
    assert!(signers.contains(&lab.public("operator")), "the operator's key came with the invite: {signers}");
    assert!(lab.root.join("beta-roster/devices/beta.json").exists() && lab.root.join("beta-roster/devices/alpha.json").exists());
    let (code, out) = lab.run_as(&beta_config, &lab.root, &["roster", "check"], &path);
    assert_eq!(code, 0, "the new machine believes the roster it was given: {out}");
    assert!(std::fs::read_dir(&carrier).map(|mut entries| entries.next().is_none()).unwrap_or(true), "the invite is cleaned up");

    // An invite can be turned down, and the new machine hears why.
    let (_, out) = lab.run(&["invite", "--json"]);
    let id = serde_json::from_str::<serde_json::Value>(out.lines().last().unwrap()).unwrap()["id"].as_str().unwrap().to_string();
    std::fs::remove_file(lab.root.join("beta-roster/devices/beta.json")).unwrap();
    let refused = join("30");
    for _ in 0..100 {
        if lab.run(&["requests", "--json"]).1.contains("beta") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let (code, out) = lab.run(&["deny", &id, "--reason", "not this one"]);
    assert_eq!(code, 0, "{out}");
    let (code, out) = finish(refused);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("turned the request down: not this one"), "{out}");
    assert!(std::fs::read_dir(&carrier).map(|mut entries| entries.next().is_none()).unwrap_or(true), "a denied invite is cleaned up too");

    // An invite that is withdrawn is gone for good.
    let (_, out) = lab.run(&["invite", "--json"]);
    let id = serde_json::from_str::<serde_json::Value>(out.lines().last().unwrap()).unwrap()["id"].as_str().unwrap().to_string();
    assert_eq!(lab.run(&["invite", "--cancel", &id]).0, 0);
    let (code, out) = finish(join("1"));
    assert_eq!(code, 1, "{out}");
}

#[test]
fn keys_are_listed_by_what_they_are_and_changed_without_editing_a_file() {
    use std::os::unix::fs::PermissionsExt;
    let lab = Lab::new();
    lab.key("second");
    lab.key("opkey");
    let operator = lab.root.join("keys/operator");
    let second = lab.root.join("keys/second");
    lab.config(&format!("signing-keys = [\"{}\"]\n", operator.display()));
    let rows = |lab: &Lab, path: &str| -> Vec<serde_json::Value> {
        let (code, out) = lab.run_with_path(&["keys", "--json"], path);
        assert_eq!(code, 0, "{out}");
        serde_json::from_str(out.lines().last().unwrap()).unwrap()
    };
    let path = std::env::var("PATH").unwrap_or_default();

    // The one key: a key file, first, and trusted (the lab trusts the operator).
    let listed = rows(&lab, &path);
    assert_eq!(listed.len(), 1);
    assert_eq!((listed[0]["kind"].as_str(), listed[0]["order"].as_u64(), listed[0]["trusted"].as_bool()), (Some("file"), Some(1), Some(true)));
    let (_, text) = lab.run(&["keys"]);
    assert!(text.contains("1. Key file  (primary)"), "{text}");
    assert!(!text.contains("SHA256"), "no fingerprint is shown: {text}");

    // Adding a key file makes it the last signing key and trusts it; the primary stays primary.
    let (code, out) = lab.run(&["keys", "add", "file", second.to_str().unwrap()]);
    assert_eq!(code, 0, "{out}");
    let listed = rows(&lab, &path);
    assert_eq!(listed.iter().map(|row| row["order"].as_u64()).collect::<Vec<_>>(), [Some(1), Some(2)]);
    assert!(listed.iter().all(|row| row["trusted"] == true));
    assert!(lab.run(&["keys", "add", "file", second.to_str().unwrap()]).1.contains("already"), "twice is refused");
    assert_eq!(lab.run(&["keys", "add", "file", "relative/key"]).0, 2);

    // Moving it up makes it the primary, and the rest of the configuration is untouched.
    let before = std::fs::read_to_string(lab.root.join("mesh.toml")).unwrap();
    let (code, out) = lab.run(&["keys", "move", second.to_str().unwrap(), "up"]);
    assert_eq!(code, 0, "{out}");
    let after = std::fs::read_to_string(lab.root.join("mesh.toml")).unwrap();
    assert!(after.find("keys/second").unwrap() < after.find("keys/operator").unwrap(), "{after}");
    assert_eq!(before.lines().count(), after.lines().count(), "nothing else changed");
    assert_eq!(lab.run(&["keys", "move", second.to_str().unwrap(), "up"]).0, 1, "already first");

    // Stopping signing with it keeps it trusted; stopping trusting it removes it everywhere.
    assert_eq!(lab.run(&["keys", "unsign", second.to_str().unwrap()]).0, 0);
    let listed = rows(&lab, &path);
    assert_eq!(listed.len(), 2);
    let kept = listed.iter().find(|row| row["order"].is_null()).unwrap();
    assert_eq!(kept["trusted"], true);
    let fingerprint = kept["fingerprint"].as_str().unwrap().to_string();
    assert_eq!(lab.run(&["keys", "distrust", &fingerprint]).0, 0);
    assert_eq!(rows(&lab, &path).len(), 1);
    assert_eq!(lab.run(&["keys", "distrust", "SHA256:nothing"]).0, 1);

    // A key made in 1Password is added, trusted and recognised as one.
    let bin = lab.root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let item = serde_json::json!({"id": "i", "fields": [{"id": "public_key", "label": "public key", "value": lab.public("opkey")}]}).to_string();
    std::fs::write(bin.join("op"), format!("#!/bin/sh\necho '{item}'\n")).unwrap();
    std::fs::set_permissions(bin.join("op"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let with_op = format!("{}:{path}", bin.display());
    let (code, out) = lab.run_with_path(&["keys", "add", "1password"], &with_op);
    assert_eq!(code, 0, "{out}");
    let listed = rows(&lab, &with_op);
    assert_eq!(listed.iter().find(|row| row["order"] == 2).unwrap()["kind"], "one-password");
}

#[test]
fn memberships_due_renew_unattended_and_never_wait_on_a_security_key() {
    let mut lab = Lab::new();
    let open = lab.port(true);
    let (a, b, c) = (lab.key("alpha"), lab.key("beta"), lab.key("gamma"));
    let write = |lab: &Lab, name: &str, key: &str, days: i64| {
        let path = lab.write(&format!("devices/{name}.json"), &lab.device_between(name, key, open, Utc::now() - Duration::days(30), Utc::now() + Duration::days(days)));
        lab.sign(&path, "operator");
    };
    write(&lab, "alpha", &a, 80);
    write(&lab, "beta", &b, 10);
    write(&lab, "gamma", &c, 60);
    let expiry = |lab: &Lab, name: &str| -> i64 {
        let text = std::fs::read_to_string(lab.root.join(format!("roster/devices/{name}.json"))).unwrap();
        let expires: chrono::DateTime<Utc> = serde_json::from_str::<serde_json::Value>(&text).unwrap()["expires"].as_str().unwrap().parse().unwrap();
        (expires - Utc::now()).num_days()
    };

    // With only a security key to sign with, something due is a step for a person, and nothing is waited on or changed.
    std::fs::write(lab.root.join("keys/fakesk.pub"), "sk-ssh-ed25519@openssh.com AAAAGnNrLXNzaC1lZDI1NTE5QG9wZW5zc2guY29tAAAAIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA fake\n").unwrap();
    lab.config(&format!("signing-keys = [\"{}\"]\n", lab.root.join("keys/fakesk.pub").display()));
    let (code, out) = lab.run(&["renew", "--due"]);
    assert_eq!(code, 3, "{out}");
    assert!(out.contains("without a touch") && out.contains("beta expires in 9 days"), "{out}");
    assert_eq!(expiry(&lab, "beta"), 9, "unchanged");

    // With a key that needs no touch after it, only what is due is renewed.
    lab.config(&format!("signing-keys = [\"{}\", \"{}\"]\n", lab.root.join("keys/fakesk.pub").display(), lab.root.join("keys/operator").display()));
    let (code, out) = lab.run(&["renew", "--due"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.lines().any(|line| line == "renewed beta"), "only beta was due: {out}");
    assert!((89..=90).contains(&expiry(&lab, "beta")), "{}", expiry(&lab, "beta"));
    assert_eq!((expiry(&lab, "alpha"), expiry(&lab, "gamma")), (79, 59), "the others are left alone");
    assert_eq!(lab.run(&["roster", "check"]).0, 0);

    // Nothing due: no output, success, and no signing.
    let (code, out) = lab.run(&["renew", "--due"]);
    assert_eq!((code, out.trim()), (0, ""));
    assert_eq!(lab.run(&["renew", "--due", "beta"]).0, 2, "--due takes no name");
}
