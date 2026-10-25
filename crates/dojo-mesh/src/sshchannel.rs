//! The channel of the OpenSSH method: a call is the system `ssh`, started with its own complete set
//! of options and no user configuration. The peer's host key comes from the signed roster, the node's
//! key from an SSH agent (1Password's by default), and nothing prompts: a connection that would have
//! to ask a question fails, or says what a person must do first.

use crate::channel::{Call, Channel, Done, Ended, Key};
use crate::method::Route;
use crate::transport::quote;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// How long to wait for a connection to be made before giving up on a peer.
const CONNECT_SECS: u32 = 5;
/// How often an idle connection is checked, and how many unanswered checks end it.
const KEEPALIVE_SECS: u32 = 15;
const KEEPALIVE_MISSES: u32 = 3;

pub struct Settings {
    pub ssh: String,
    pub ssh_add: String,
    /// The account to connect as on the peer.
    pub user: String,
    /// The public half of this node's key. The private half stays in the agent.
    pub node_key: PathBuf,
    /// The public half of the key used for remote input only.
    pub input_key: PathBuf,
    pub agent: Option<PathBuf>,
    /// A directory this channel writes the peers' pinned host keys into.
    pub known_hosts: PathBuf,
}

pub struct OpenSsh {
    pub settings: Settings,
}

/// What a host-key alias for a peer is, so a peer's address can change without its key pin changing.
pub fn alias(peer: &str) -> String {
    format!("dojo-{peer}")
}

impl OpenSsh {
    pub fn new(settings: Settings) -> Self {
        Self { settings }
    }

    fn key_file(&self, key: Key) -> &Path {
        match key {
            Key::Node => &self.settings.node_key,
            Key::Input => &self.settings.input_key,
        }
    }

    /// Every option of the connection. `-F /dev/null` keeps the user's own `ssh` configuration (their
    /// hosts, proxies, agent forwarding, control sockets) out of a mesh connection.
    pub fn arguments(&self, call: &Call, known_hosts: &Path) -> Result<Vec<String>, String> {
        let s = &self.settings;
        let mut args: Vec<String> = ["-F", "/dev/null", "-o", "BatchMode=yes", "-o", "RequestTTY=no"].map(String::from).to_vec();
        let mut option = |text: String| {
            args.push("-o".into());
            args.push(text);
        };
        option("PreferredAuthentications=publickey".into());
        option("PasswordAuthentication=no".into());
        option("KbdInteractiveAuthentication=no".into());
        option("IdentitiesOnly=yes".into());
        option(format!("IdentityFile={}", self.key_file(call.key).display()));
        match &s.agent {
            Some(socket) => option(format!("IdentityAgent={}", socket.display())),
            None => option("IdentityAgent=none".into()),
        }
        option(format!("UserKnownHostsFile={}", known_hosts.display()));
        option("GlobalKnownHostsFile=/dev/null".into());
        option("StrictHostKeyChecking=yes".into());
        option(format!("HostKeyAlias={}", alias(&call.peer.name)));
        for forbidden in ["ForwardAgent=no", "ForwardX11=no", "ClearAllForwardings=yes", "PermitLocalCommand=no", "ControlMaster=no", "ControlPath=none"] {
            option(forbidden.into());
        }
        option(format!("ConnectTimeout={CONNECT_SECS}"));
        option(format!("ServerAliveInterval={KEEPALIVE_SECS}"));
        option(format!("ServerAliveCountMax={KEEPALIVE_MISSES}"));
        args.push("-l".into());
        args.push(s.user.clone());
        let host = match &call.route {
            Route::Host { name, port } => {
                if name.starts_with('-') || name.contains(char::is_whitespace) {
                    return Err(format!("{name:?} is not an address"));
                }
                if let Some(port) = port {
                    args.push("-p".into());
                    args.push(port.to_string());
                }
                name.clone()
            }
            Route::Command(argv) => {
                if argv.is_empty() {
                    return Err("the command route is empty".into());
                }
                args.push("-o".into());
                args.push(format!("ProxyCommand={}", argv.iter().map(|word| quote(word)).collect::<Vec<_>>().join(" ")));
                alias(&call.peer.name)
            }
        };
        args.push("--".into());
        args.push(host);
        args.push(call.remote.clone());
        Ok(args)
    }

    /// Writes the peer's pinned key where `ssh` reads it, and returns the file. The directory and the
    /// file are the user's alone.
    fn pin(&self, peer: &str, host_key: &str) -> Result<PathBuf, String> {
        let dir = &self.settings.known_hosts;
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir).map_err(|error| format!("cannot keep pinned host keys in {}: {error}", dir.display()))?;
        let file = dir.join(peer);
        let line = format!("{} {}\n", alias(peer), host_key);
        let mut handle = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&file).map_err(|error| format!("cannot write {}: {error}", file.display()))?;
        std::io::Write::write_all(&mut handle, line.as_bytes()).map_err(|error| format!("cannot write {}: {error}", file.display()))?;
        Ok(file)
    }

    /// What a person must do before this node can use its key, or `None` when the agent offers it.
    /// A locked 1Password lists no keys, so an empty list means the same as a missing socket: the
    /// holder of the key is not ready.
    pub fn agent_step(&self, key: Key) -> Option<String> {
        let s = &self.settings;
        let Some(socket) = &s.agent else { return None };
        let key_file = self.key_file(key);
        let body = std::fs::read_to_string(key_file).ok().and_then(|text| text.split_whitespace().nth(1).map(str::to_string));
        let Some(body) = body else {
            return Some(format!("this node has no key yet: {} is missing; enroll it with dojo-mesh init or join", key_file.display()));
        };
        if !socket.exists() {
            return Some(format!("the SSH agent is not running at {}: open 1Password and turn on Settings > Developer > SSH Agent", socket.display()));
        }
        let output = Command::new(&s.ssh_add).arg("-L").env("SSH_AUTH_SOCK", socket).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
        let listed = String::from_utf8_lossy(&output.stdout);
        if listed.lines().any(|line| line.split_whitespace().nth(1) == Some(body.as_str())) {
            return None;
        }
        Some(if listed.lines().any(|line| line.split_whitespace().count() >= 2) {
            format!("the SSH agent does not offer this key ({}): check that it is in a vault 1Password's SSH agent shares", key_file.display())
        } else {
            "the SSH agent has no keys: unlock 1Password on this machine, then run the command again".to_string()
        })
    }
}

impl Channel for OpenSsh {
    fn blocked(&self) -> Option<String> {
        self.agent_step(Key::Node)
    }

    fn run(&self, call: Call) -> Result<Done, String> {
        let Some(host_key) = call.peer.host_key.clone() else {
            return Err(format!("{} has no host key in the roster, so its server cannot be recognised", call.peer.name));
        };
        if let Some(step) = self.agent_step(call.key) {
            return Ok(Done { ended: Ended::NeedsHuman(step), stdout: Vec::new() });
        }
        let known_hosts = self.pin(&call.peer.name, &host_key)?;
        let args = self.arguments(&call, &known_hosts)?;
        let mut command = Command::new(&self.settings.ssh);
        command.args(args);
        if let Some(socket) = &self.settings.agent {
            command.env("SSH_AUTH_SOCK", socket);
        }
        crate::ssh::carry(command, "ssh", call.input, call.output, call.timeout, |_| None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::{Input, Output, Peer};
    use std::os::unix::fs::PermissionsExt;

    fn settings(dir: &Path) -> Settings {
        Settings {
            ssh: "ssh".into(),
            ssh_add: "ssh-add".into(),
            user: "pat".into(),
            node_key: dir.join("node_key.pub"),
            input_key: dir.join("input_key.pub"),
            agent: Some(dir.join("agent.sock")),
            known_hosts: dir.join("known_hosts"),
        }
    }

    fn call(route: Route) -> Call {
        Call { peer: Peer { name: "orchard".into(), host_key: Some("ssh-ed25519 AAAAhost".into()) }, key: Key::Node, route, remote: "echo hi".into(), input: Input::Null, output: Output::Capture, timeout: None }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dojo-sshchannel-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn script(path: &Path, body: &str) {
        std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn a_connection_ignores_the_users_ssh_configuration_and_never_asks() {
        let dir = scratch("args");
        let channel = OpenSsh::new(settings(&dir));
        let args = channel.arguments(&call(Route::Host { name: "10.0.0.7".into(), port: Some(2222) }), &dir.join("kh")).unwrap();
        let text = args.join(" ");
        assert!(text.starts_with("-F /dev/null "), "{text}");
        for needed in [
            "BatchMode=yes",
            "PasswordAuthentication=no",
            "IdentitiesOnly=yes",
            "StrictHostKeyChecking=yes",
            "HostKeyAlias=dojo-orchard",
            "ForwardAgent=no",
            "ClearAllForwardings=yes",
            "ControlPath=none",
            "GlobalKnownHostsFile=/dev/null",
            "ConnectTimeout=5",
            "ServerAliveInterval=15",
        ] {
            assert!(args.iter().any(|arg| arg == needed), "missing {needed}: {text}");
        }
        assert!(args.iter().any(|arg| *arg == format!("IdentityFile={}", dir.join("node_key.pub").display())));
        assert!(args.iter().any(|arg| *arg == format!("IdentityAgent={}", dir.join("agent.sock").display())));
        assert_eq!(args[args.len() - 5..], ["-p".to_string(), "2222".into(), "--".into(), "10.0.0.7".into(), "echo hi".into()][..], "{text}");
        assert_eq!(args.iter().position(|arg| arg == "-l").map(|at| args[at + 1].as_str()), Some("pat"));
    }

    #[test]
    fn a_command_route_becomes_a_proxy_command_with_each_word_quoted() {
        let dir = scratch("proxy");
        let channel = OpenSsh::new(settings(&dir));
        let args = channel.arguments(&call(Route::Command(vec!["cloudflared".into(), "access".into(), "ssh it's".into()])), &dir.join("kh")).unwrap();
        assert!(args.contains(&"ProxyCommand='cloudflared' 'access' 'ssh it'\\''s'".to_string()), "{args:?}");
        assert_eq!(args[args.len() - 2], "dojo-orchard");
        assert!(channel.arguments(&call(Route::Command(Vec::new())), &dir.join("kh")).is_err());
    }

    #[test]
    fn remote_input_uses_the_input_key_and_nothing_else() {
        let dir = scratch("input");
        let channel = OpenSsh::new(settings(&dir));
        let mut c = call(Route::Host { name: "h".into(), port: None });
        c.key = Key::Input;
        let args = channel.arguments(&c, &dir.join("kh")).unwrap();
        assert!(args.iter().any(|arg| *arg == format!("IdentityFile={}", dir.join("input_key.pub").display())));
        assert!(!args.iter().any(|arg| arg.contains("node_key.pub")));
        assert!(channel.agent_step(Key::Input).unwrap().contains("input_key.pub"));
    }

    #[test]
    fn an_address_that_could_be_an_option_is_refused() {
        let dir = scratch("option");
        let channel = OpenSsh::new(settings(&dir));
        assert!(channel.arguments(&call(Route::Host { name: "-oProxyCommand=evil".into(), port: None }), &dir.join("kh")).is_err());
    }

    #[test]
    fn the_peers_host_key_is_pinned_in_a_private_file_under_its_alias() {
        let dir = scratch("pin");
        let channel = OpenSsh::new(settings(&dir));
        let file = channel.pin("orchard", "ssh-ed25519 AAAAhost").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "dojo-orchard ssh-ed25519 AAAAhost\n");
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::metadata(file.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
    }

    #[test]
    fn the_agent_is_checked_before_a_connection_is_tried() {
        let dir = scratch("agent");
        let mut s = settings(&dir);
        s.ssh_add = dir.join("ssh-add").display().to_string();
        let channel = OpenSsh::new(s);
        // No key file yet.
        assert!(channel.agent_step(Key::Node).unwrap().contains("no key yet"));
        std::fs::write(dir.join("node_key.pub"), "ssh-ed25519 AAAAnode node\n").unwrap();
        // No agent.
        assert!(channel.agent_step(Key::Node).unwrap().contains("not running"));
        std::fs::write(dir.join("agent.sock"), "").unwrap();
        // Locked: no identities.
        script(&dir.join("ssh-add"), "echo 'The agent has no identities.' >&2; exit 1");
        assert!(channel.agent_step(Key::Node).unwrap().contains("unlock 1Password"));
        // Another key only.
        script(&dir.join("ssh-add"), "echo 'ssh-ed25519 AAAAother other'");
        assert!(channel.agent_step(Key::Node).unwrap().contains("does not offer this key"));
        // Offered.
        script(&dir.join("ssh-add"), "echo 'ssh-ed25519 AAAAother other'; echo 'ssh-ed25519 AAAAnode node'");
        assert_eq!(channel.agent_step(Key::Node), None);
    }

    #[test]
    fn a_locked_agent_ends_the_call_as_needing_a_person_without_starting_ssh() {
        let dir = scratch("locked");
        let mut s = settings(&dir);
        s.ssh_add = dir.join("ssh-add").display().to_string();
        s.ssh = dir.join("ssh").display().to_string();
        std::fs::write(dir.join("node_key.pub"), "ssh-ed25519 AAAAnode node\n").unwrap();
        std::fs::write(dir.join("agent.sock"), "").unwrap();
        script(&dir.join("ssh-add"), "exit 1");
        script(&dir.join("ssh"), &format!("touch {}/ssh-ran", dir.display()));
        let done = OpenSsh::new(s).run(call(Route::Host { name: "h".into(), port: None })).unwrap();
        assert!(matches!(done.ended, Ended::NeedsHuman(step) if step.contains("unlock 1Password")));
        assert!(!dir.join("ssh-ran").exists());
    }

    #[test]
    fn a_peer_without_a_host_key_is_never_connected_to() {
        let dir = scratch("nokey");
        let mut c = call(Route::Host { name: "h".into(), port: None });
        c.peer.host_key = None;
        assert!(OpenSsh::new(settings(&dir)).run(c).unwrap_err().contains("no host key"));
    }

    #[test]
    fn the_call_runs_ssh_with_the_arguments_and_returns_its_output_and_status() {
        let dir = scratch("run");
        let mut s = settings(&dir);
        s.ssh = dir.join("ssh").display().to_string();
        s.ssh_add = dir.join("ssh-add").display().to_string();
        std::fs::write(dir.join("node_key.pub"), "ssh-ed25519 AAAAnode node\n").unwrap();
        std::fs::write(dir.join("agent.sock"), "").unwrap();
        script(&dir.join("ssh-add"), "echo 'ssh-ed25519 AAAAnode node'");
        script(&dir.join("ssh"), "printf '%s\\n' \"$@\"; echo \"agent=$SSH_AUTH_SOCK\"; exit 7");
        let done = OpenSsh::new(s).run(call(Route::Host { name: "h".into(), port: None })).unwrap();
        assert_eq!(done.ended, Ended::Exited(7));
        let out = String::from_utf8(done.stdout).unwrap();
        assert!(out.contains("HostKeyAlias=dojo-orchard") && out.contains("\nh\necho hi\n"), "{out}");
        assert!(out.contains(&format!("agent={}", dir.join("agent.sock").display())), "{out}");
    }
}
