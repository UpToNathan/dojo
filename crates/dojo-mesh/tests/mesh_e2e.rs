//! The mesh end to end against a fake `tailscale`. It answers `status --json` from a fixture and
//! turns `ssh HOST COMMAND` into running COMMAND here, as if the peer's home were a directory of
//! its own. No network and no real device names are involved.

use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static SITES: AtomicUsize = AtomicUsize::new(0);

const STATUS: &str = r#"{
  "BackendState": "Running",
  "Self": {"HostName": "alpha", "DNSName": "alpha.example.ts.net.", "Tags": ["tag:mesh"], "Online": true},
  "Peer": {
    "k1": {"HostName": "beta", "DNSName": "beta.example.ts.net.", "Tags": ["tag:mesh"], "Online": true},
    "k2": {"HostName": "gone", "DNSName": "gone.example.ts.net.", "Tags": ["tag:mesh"], "Online": false},
    "k3": {"HostName": "checky", "DNSName": "checky.example.ts.net.", "Tags": ["tag:mesh"], "Online": true},
    "k4": {"HostName": "stranger", "DNSName": "stranger.example.ts.net.", "Tags": [], "Online": true}
  }
}"#;

const FAKE_TAILSCALE: &str = r#"#!/bin/sh
case "$1" in
status) cat "$FAKE_ROOT/status.json" ;;
ssh)
  host=$2; command=$3
  short=${host%%.*}
  if [ "$short" = checky ]; then
    echo "To authenticate, visit: https://login.tailscale.com/a/abc123" >&2
    exec sleep 30
  fi
  echo "$short" >> "$FAKE_ROOT/ssh.log"
  export HOME="$FAKE_ROOT/hosts/$short/home"
  export XDG_CONFIG_HOME="$FAKE_ROOT/hosts/$short/config"
  export XDG_STATE_HOME="$FAKE_ROOT/hosts/$short/state"
  exec sh -c "$command" ;;
esac
"#;

struct Site {
    root: PathBuf,
}

impl Site {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("dojo-mesh-e2e-{}-{}", std::process::id(), SITES.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&root);
        for dir in ["bin", "data/dojo/bin", "hosts/alpha/home", "hosts/alpha/config/dojo", "hosts/beta/home", "hosts/beta/config/dojo", "hosts/alpha/state"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        let tailscale = root.join("bin/tailscale");
        std::fs::write(&tailscale, FAKE_TAILSCALE).unwrap();
        std::process::Command::new("chmod").arg("+x").arg(&tailscale).status().unwrap();
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_dojo-mesh"), root.join("data/dojo/bin/dojo-mesh")).unwrap();
        std::fs::write(root.join("status.json"), STATUS).unwrap();
        let site = Self { root };
        site.config("alpha", "membership-tag = \"tag:mesh\"\n");
        site
    }

    fn config(&self, host: &str, text: &str) {
        std::fs::write(self.root.join(format!("hosts/{host}/config/dojo/mesh.toml")), text).unwrap();
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }

    fn command(&self, origin: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_dojo-mesh"));
        let path = format!("{}:/usr/bin:/bin", self.root.join("bin").display());
        command
            .env_clear()
            .env("PATH", path)
            .env("FAKE_ROOT", &self.root)
            .env("HOME", self.root.join("hosts/alpha/home"))
            .env("XDG_CONFIG_HOME", self.root.join("hosts/alpha/config"))
            .env("XDG_STATE_HOME", self.root.join("hosts/alpha/state"))
            .env("XDG_DATA_HOME", self.root.join("data"))
            .env("DOJO_MESH_ORIGIN", origin)
            .env("DOJO_MESH_SESSION", "test-session");
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command("operator").args(args).output().unwrap()
    }

    fn run_as_agent(&self, args: &[&str]) -> Output {
        self.command("agent").args(args).output().unwrap()
    }

    fn ssh_log(&self) -> String {
        std::fs::read_to_string(self.path("ssh.log")).unwrap_or_default()
    }
}

impl Drop for Site {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn code(output: &Output) -> i32 {
    output.status.code().unwrap()
}

fn out(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into()
}

fn err(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into()
}

#[test]
fn exec_runs_the_command_on_the_peer_with_provenance_and_literal_arguments() {
    let site = Site::new();
    let result = site.run(&["exec", "beta", "--", "sh", "-c", "printf '%s|%s|%s|%s' \"$DOJO_FROM\" \"$DOJO_ORIGIN\" \"$DOJO_SESSION\" \"$1\"", "sh", "a b'c $HOME"]);
    assert_eq!(code(&result), 0, "{}", err(&result));
    assert_eq!(out(&result), "alpha|operator|test-session|a b'c $HOME");
    assert_eq!(site.ssh_log(), "beta\n");
}

#[test]
fn a_failing_remote_command_exits_1_and_names_its_status() {
    let site = Site::new();
    let result = site.run(&["exec", "beta", "--", "sh", "-c", "exit 7"]);
    assert_eq!(code(&result), 1);
    assert!(err(&result).contains("beta exited with status 7"), "{}", err(&result));
}

#[test]
fn offline_unknown_and_unadmitted_peers_fail_fast_without_connecting() {
    let site = Site::new();
    for (peer, reason) in [("gone", "offline"), ("stranger", "not a member"), ("nobody", "not a member"), ("alpha", "this node")] {
        let result = site.run(&["exec", peer, "--", "true"]);
        assert_eq!(code(&result), 4, "{peer}: {}", err(&result));
        assert!(err(&result).contains(reason), "{peer}: {}", err(&result));
    }
    assert_eq!(site.ssh_log(), "", "nothing was sent");
}

#[test]
fn a_sign_in_request_from_tailscale_exits_3_with_the_page() {
    let site = Site::new();
    let started = std::time::Instant::now();
    let result = site.run(&["exec", "checky", "--", "true"]);
    assert_eq!(code(&result), 3, "{}", err(&result));
    assert!(err(&result).contains("https://login.tailscale.com/a/abc123"), "{}", err(&result));
    assert!(started.elapsed().as_secs() < 20, "the call must not wait for the sign-in");
}

#[test]
fn a_timeout_ends_a_slow_call() {
    let site = Site::new();
    let result = site.run(&["exec", "--timeout", "1", "beta", "--", "sleep", "20"]);
    assert_eq!(code(&result), 4);
    assert!(err(&result).contains("timed out"), "{}", err(&result));
}

#[test]
fn pausing_stops_the_agent_but_not_the_operator() {
    let site = Site::new();
    assert_eq!(code(&site.run(&["pause"])), 0);
    let agent = site.run_as_agent(&["exec", "beta", "--", "true"]);
    assert_eq!(code(&agent), 4);
    assert!(err(&agent).contains("paused"), "{}", err(&agent));
    assert_eq!(code(&site.run(&["exec", "beta", "--", "true"])), 0);
    assert!(out(&site.run(&["status"])).contains("agent peer access: paused"));
    assert_eq!(code(&site.run(&["resume"])), 0);
    assert_eq!(code(&site.run_as_agent(&["exec", "beta", "--", "true"])), 0);
}

#[test]
fn every_call_is_in_the_audit_log_with_who_made_it() {
    let site = Site::new();
    site.run_as_agent(&["exec", "beta", "--", "true"]);
    site.run(&["exec", "gone", "--", "true"]);
    let log = site.run(&["audit", "--json"]);
    let entries: serde_json::Value = serde_json::from_str(&out(&log)).unwrap();
    let entries = entries.as_array().unwrap();
    let results: Vec<(&str, &str, &str)> = entries
        .iter()
        .map(|e| (e["origin"].as_str().unwrap(), e["peer"].as_str().unwrap(), e["result"].as_str().unwrap()))
        .collect();
    assert_eq!(results[0], ("agent", "beta", "started"));
    assert_eq!(results[1], ("agent", "beta", "exit 0"));
    assert_eq!(results[2].1, "gone");
    assert!(results[2].2.starts_with("refused:"), "{results:?}");
    assert_eq!(entries[0]["from"], "alpha");
    assert_eq!(entries[0]["session"], "test-session");
    assert!(out(&site.run(&["audit"])).contains("agent test-session@alpha -> beta exec true [started]"));
}

#[test]
fn the_agent_cannot_call_a_peer_if_the_call_cannot_be_logged() {
    let site = Site::new();
    // A file where the state directory should be.
    std::fs::remove_dir_all(site.path("hosts/alpha/state")).unwrap();
    std::fs::write(site.path("hosts/alpha/state"), "").unwrap();
    let agent = site.run_as_agent(&["exec", "beta", "--", "true"]);
    assert_eq!(code(&agent), 4, "{}", err(&agent));
    assert!(err(&agent).contains("logged"), "{}", err(&agent));
    assert_eq!(site.ssh_log(), "");
    let operator = site.run(&["exec", "beta", "--", "true"]);
    assert_eq!(code(&operator), 0, "the operator is warned, not stopped: {}", err(&operator));
    assert!(err(&operator).contains("warning"), "{}", err(&operator));
}

#[test]
fn cp_pushes_a_file_with_its_permissions_and_replaces_atomically() {
    use std::os::unix::fs::PermissionsExt;
    let site = Site::new();
    let source = site.path("source.sh");
    std::fs::write(&source, "echo hi\n").unwrap();
    std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o750)).unwrap();
    let dest = site.path("hosts/beta/home/copied.sh");
    std::fs::write(&dest, "old").unwrap();

    let result = site.run(&["cp", source.to_str().unwrap(), &format!("beta:{}", dest.display())]);
    assert_eq!(code(&result), 0, "{}", err(&result));
    assert_eq!(std::fs::read_to_string(&dest).unwrap(), "echo hi\n");
    assert_eq!(std::fs::metadata(&dest).unwrap().permissions().mode() & 0o777, 0o750);

    // Into a directory keeps the file's name.
    let directory = site.path("hosts/beta/home/inbox");
    std::fs::create_dir(&directory).unwrap();
    let result = site.run(&["cp", source.to_str().unwrap(), &format!("beta:{}/", directory.display())]);
    assert_eq!(code(&result), 0, "{}", err(&result));
    assert!(directory.join("source.sh").exists());
    let leftovers: Vec<_> = std::fs::read_dir(site.path("hosts/beta/home")).unwrap().filter_map(Result::ok).filter(|e| e.file_name().to_string_lossy().contains("copied.sh.")).collect();
    assert!(leftovers.is_empty(), "no temporary files remain");
}

#[test]
fn cp_pulls_a_file_and_leaves_nothing_behind_on_failure() {
    let site = Site::new();
    let remote = site.path("hosts/beta/home/notes.txt");
    std::fs::write(&remote, "remote notes").unwrap();
    let local_dir = site.path("hosts/alpha/home/in");
    std::fs::create_dir(&local_dir).unwrap();

    let result = site.run(&["cp", &format!("beta:{}", remote.display()), local_dir.to_str().unwrap()]);
    assert_eq!(code(&result), 0, "{}", err(&result));
    assert_eq!(std::fs::read_to_string(local_dir.join("notes.txt")).unwrap(), "remote notes");

    let missing = site.run(&["cp", &format!("beta:{}/nope", site.path("hosts/beta/home").display()), local_dir.join("x").to_str().unwrap()]);
    assert_eq!(code(&missing), 1);
    assert!(!local_dir.join("x").exists());
    let names: Vec<_> = std::fs::read_dir(&local_dir).unwrap().filter_map(Result::ok).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(names, ["notes.txt"], "no staging file remains");
}

#[test]
fn cp_refuses_directories_and_ambiguous_endpoints() {
    let site = Site::new();
    let directory = site.path("hosts/alpha/home");
    let result = site.run(&["cp", directory.to_str().unwrap(), "beta:/tmp/x"]);
    assert_eq!(code(&result), 1);
    assert!(err(&result).contains("not a regular file"), "{}", err(&result));
    assert_eq!(code(&site.run(&["cp", "a", "b"])), 2);
    assert_eq!(code(&site.run(&["cp", "beta:a", "beta:b"])), 2);
}

#[test]
fn find_asks_each_peer_what_it_offers() {
    let site = Site::new();
    std::fs::write(site.path("hosts/beta/home/public-key"), "cache-key-1:abc\n").unwrap();
    site.config(
        "beta",
        &format!("[capability.nix-cache]\nport = 5000\npublic-key-file = \"{}\"\n", site.path("hosts/beta/home/public-key").display()),
    );
    let result = site.run(&["find", "nix-cache", "--peers-only", "--json"]);
    assert_eq!(code(&result), 0, "{}", err(&result));
    let value: serde_json::Value = serde_json::from_str(&out(&result)).unwrap();
    let offers = value["offers"].as_array().unwrap();
    assert_eq!(offers.len(), 1);
    assert_eq!(offers[0]["node"], "beta");
    assert_eq!(offers[0]["host"], "beta.example.ts.net");
    assert_eq!(offers[0]["attributes"]["port"], "5000");
    assert_eq!(offers[0]["attributes"]["public-key"], "cache-key-1:abc");
    let unreachable: Vec<&str> = value["unreachable"].as_array().unwrap().iter().map(|entry| entry[0].as_str().unwrap()).collect();
    assert!(unreachable.contains(&"gone") && unreachable.contains(&"checky"), "{unreachable:?}");
}

#[test]
fn find_includes_this_node_unless_asked_not_to_and_reports_when_nobody_offers() {
    let site = Site::new();
    site.config("alpha", "membership-tag = \"tag:mesh\"\n[capability.voice-agent]\ncommand = \"/x/dojo-voice\"\n");
    let with_self = site.run(&["find", "voice-agent"]);
    assert_eq!(code(&with_self), 0);
    assert!(out(&with_self).contains("voice-agent offered by alpha (alpha.example.ts.net): command=/x/dojo-voice"), "{}", out(&with_self));
    let without = site.run(&["find", "voice-agent", "--peers-only"]);
    assert_eq!(code(&without), 1);
    assert!(out(&without).contains("No node offers voice-agent."));
}

#[test]
fn task_hands_the_text_to_the_peers_declared_agent() {
    let site = Site::new();
    let record = site.path("task-record");
    let agent = site.path("hosts/beta/home/fake-agent");
    std::fs::write(&agent, format!("#!/bin/sh\nprintf '%s|%s|%s' \"$1\" \"$2\" \"$DOJO_ORIGIN\" > {}\n", record.display())).unwrap();
    Command::new("chmod").arg("+x").arg(&agent).status().unwrap();
    site.config("beta", &format!("[capability.voice-agent]\ncommand = \"{}\"\n", agent.display()));
    let result = site.run_as_agent(&["task", "beta", "open", "the", "logs"]);
    assert_eq!(code(&result), 0, "{}", err(&result));
    assert_eq!(std::fs::read_to_string(&record).unwrap(), "send|open the logs|agent");
    let log = out(&site.run(&["audit"]));
    assert!(log.contains("task open the logs [exit 0]"), "{log}");
}

#[test]
fn task_needs_a_peer_that_declares_an_agent() {
    let site = Site::new();
    let result = site.run(&["task", "beta", "do", "it"]);
    assert_eq!(code(&result), 1);
    assert!(err(&result).contains("does not offer a voice-agent"), "{}", err(&result));
}

#[test]
fn describe_publishes_declared_capabilities_and_file_contents() {
    let site = Site::new();
    std::fs::write(site.path("hosts/alpha/home/key"), "pub\n").unwrap();
    site.config("alpha", &format!("[capability.c]\nplain = \"v\"\nkey-file = \"{}\"\n", site.path("hosts/alpha/home/key").display()));
    let value: serde_json::Value = serde_json::from_str(&out(&site.run(&["describe"]))).unwrap();
    assert_eq!(value["capabilities"]["c"]["plain"], "v");
    assert_eq!(value["capabilities"]["c"]["key"], "pub");
}
