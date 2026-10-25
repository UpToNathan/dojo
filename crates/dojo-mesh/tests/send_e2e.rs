//! Sending a window end to end: the real binary, a fake `tailscale` that runs "remote" commands
//! here as another host, and a fake Niri socket for each host that records what it was asked.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

static SITES: AtomicUsize = AtomicUsize::new(0);

const STATUS: &str = r#"{"BackendState": "Running",
  "Self": {"HostName": "alpha", "DNSName": "alpha.example.ts.net.", "Tags": ["tag:mesh"], "Online": true},
  "Peer": {"k1": {"HostName": "beta", "DNSName": "beta.example.ts.net.", "Tags": ["tag:mesh"], "Online": true}}}"#;

const FAKE_TAILSCALE: &str = r#"#!/bin/sh
case "$1" in
status) cat "$FAKE_ROOT/status.json" ;;
ssh)
  short=${2%%.*}
  export HOME="$FAKE_ROOT/hosts/$short/home"
  export XDG_CONFIG_HOME="$FAKE_ROOT/hosts/$short/config"
  export XDG_STATE_HOME="$FAKE_ROOT/hosts/$short/state"
  export XDG_DATA_DIRS="$FAKE_ROOT/hosts/$short/share"
  export NIRI_SOCKET="$FAKE_ROOT/hosts/$short/niri.sock"
  exec sh -c "$3" ;;
esac
"#;

type Seen = Arc<Mutex<Vec<String>>>;

/// A Niri that lists `windows` and accepts every action, remembering each request.
fn fake_niri(path: &Path, windows: &'static str) -> Seen {
    let listener = UnixListener::bind(path).unwrap();
    let seen: Seen = Arc::default();
    let record = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut line = String::new();
            if BufReader::new(&stream).read_line(&mut line).is_err() {
                continue;
            }
            let line = line.trim().to_string();
            let reply = if line == "\"Windows\"" { format!("{{\"Ok\":{{\"Windows\":{}}}}}", windows.replace('\n', " ")) } else { "{\"Ok\":\"Handled\"}".to_string() };
            record.lock().unwrap().push(line);
            let _ = (&stream).write_all(format!("{reply}\n").as_bytes());
        }
    });
    seen
}

struct Site {
    root: PathBuf,
    alpha: Seen,
    beta: Seen,
}

const ALPHA_WINDOWS: &str = r#"[{"id": 7, "title": "Example", "app_id": "org.example.App", "pid": 1, "is_focused": true},
  {"id": 8, "title": "Other", "app_id": "mystery", "pid": 2, "is_focused": false}]"#;

impl Site {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("dojo-send-{}-{}", std::process::id(), SITES.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&root);
        for dir in ["bin", "data/dojo/bin", "hosts/alpha/home", "hosts/alpha/config/dojo", "hosts/alpha/share/applications", "hosts/beta/home", "hosts/beta/config/dojo", "hosts/beta/share/applications"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        std::fs::write(root.join("bin/tailscale"), FAKE_TAILSCALE).unwrap();
        Command::new("chmod").arg("+x").arg(root.join("bin/tailscale")).status().unwrap();
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_dojo-mesh"), root.join("data/dojo/bin/dojo-mesh")).unwrap();
        std::fs::write(root.join("status.json"), STATUS).unwrap();
        std::fs::write(root.join("hosts/alpha/config/dojo/mesh.toml"), "membership-tag = \"tag:mesh\"\n").unwrap();
        for host in ["alpha", "beta"] {
            std::fs::write(root.join(format!("hosts/{host}/share/applications/org.example.App.desktop")), "[Desktop Entry]\nName=Example\nExec=example\n").unwrap();
        }
        let alpha = fake_niri(&root.join("hosts/alpha/niri.sock"), ALPHA_WINDOWS);
        let beta = fake_niri(&root.join("hosts/beta/niri.sock"), "[]");
        Self { root, alpha, beta }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_dojo-mesh"));
        command
            .env_clear()
            .env("PATH", format!("{}:/usr/bin:/bin", self.root.join("bin").display()))
            .env("FAKE_ROOT", &self.root)
            .env("HOME", self.root.join("hosts/alpha/home"))
            .env("XDG_CONFIG_HOME", self.root.join("hosts/alpha/config"))
            .env("XDG_STATE_HOME", self.root.join("hosts/alpha/state"))
            .env("XDG_DATA_HOME", self.root.join("data"))
            .env("XDG_DATA_DIRS", self.root.join("hosts/alpha/share"))
            .env("NIRI_SOCKET", self.root.join("hosts/alpha/niri.sock"));
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).stdin(Stdio::null()).output().unwrap()
    }

    fn receive(&self, handoff: &str) -> Output {
        let mut child = self.command().arg("receive").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        child.stdin.take().unwrap().write_all(handoff.as_bytes()).unwrap();
        child.wait_with_output().unwrap()
    }

    fn requests(seen: &Seen) -> Vec<String> {
        seen.lock().unwrap().clone()
    }
}

impl Drop for Site {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn text(output: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

#[test]
fn windows_lists_this_nodes_windows_with_the_focused_one_marked() {
    let site = Site::new();
    let result = site.run(&["windows"]);
    assert_eq!(result.status.code(), Some(0), "{}", text(&result));
    assert_eq!(String::from_utf8_lossy(&result.stdout), "*7  org.example.App  Example\n 8  mystery  Other\n");
}

#[test]
fn sending_opens_the_application_on_the_peer_then_closes_it_here() {
    let site = Site::new();
    let result = site.run(&["send", "--to", "beta"]);
    assert_eq!(result.status.code(), Some(0), "{}", text(&result));
    assert!(text(&result).starts_with("Sent to beta."), "{}", text(&result));
    assert!(text(&result).contains("started fresh"), "{}", text(&result));
    assert_eq!(Site::requests(&site.beta), [r#"{"Action":{"Spawn":{"command":["gtk-launch","org.example.App"]}}}"#]);
    assert_eq!(Site::requests(&site.alpha), ["\"Windows\"", r#"{"Action":{"CloseWindow":{"id":7}}}"#]);
    let audit = text(&site.run(&["audit"]));
    assert!(audit.contains("-> beta send window 7 (org.example.App)") && audit.contains("[exit 0]"), "{audit}");
}

#[test]
fn a_peer_without_the_application_refuses_and_the_window_stays() {
    let site = Site::new();
    std::fs::remove_file(site.root.join("hosts/beta/share/applications/org.example.App.desktop")).unwrap();
    let result = site.run(&["send", "--to", "beta", "--json"]);
    assert_eq!(result.status.code(), Some(1), "{}", text(&result));
    assert!(text(&result).contains("beta did not open it: this machine has no org.example.App application"), "{}", text(&result));
    assert_eq!(Site::requests(&site.alpha), ["\"Windows\""], "nothing was closed");
    assert!(Site::requests(&site.beta).is_empty());
}

#[test]
fn a_window_from_an_unknown_application_is_not_sent() {
    let site = Site::new();
    let result = site.run(&["send", "--window", "8", "--to", "beta"]);
    assert_eq!(result.status.code(), Some(1));
    assert!(text(&result).contains("mystery has no desktop entry"), "{}", text(&result));
    let missing = site.run(&["send", "--window", "99", "--to", "beta"]);
    assert!(text(&missing).contains("there is no window 99"));
}

#[test]
fn without_niri_there_is_nothing_to_send() {
    let site = Site::new();
    let result = site.command().args(["send", "--to", "beta"]).env("NIRI_SOCKET", site.root.join("nowhere.sock")).env("PATH", format!("{}:/nonexistent", site.root.join("bin").display())).output().unwrap();
    assert_eq!(result.status.code(), Some(4), "{}", text(&result));
    assert!(text(&result).contains("Niri is not running here"), "{}", text(&result));
}

#[test]
fn receive_opens_known_things_and_reports_missing_files_without_opening() {
    let site = Site::new();
    let present = site.root.join("hosts/alpha/home/notes.txt");
    std::fs::write(&present, "x").unwrap();
    let editor = |file: &Path| format!(r#"{{"kind":"terminal","directory":"{}","program":{{"kind":"editor","file":"{}","line":4,"column":2}}}}"#, site.root.join("hosts/alpha/home").display(), file.display());

    let opened = site.receive(&editor(&present));
    assert_eq!(opened.status.code(), Some(0), "{}", text(&opened));
    assert_eq!(String::from_utf8_lossy(&opened.stdout).trim(), r#"{"status":"opened","notes":[]}"#);

    let absent = site.root.join("hosts/alpha/home/absent.txt");
    let missing = site.receive(&editor(&absent));
    assert_eq!(missing.status.code(), Some(5));
    assert_eq!(String::from_utf8_lossy(&missing.stdout).trim(), format!(r#"{{"status":"missing","files":["{}"]}}"#, absent.display()));

    let refused = site.receive(r#"{"kind":"run","command":"rm -rf ~"}"#);
    assert_eq!(refused.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&refused.stdout).contains("not a handoff"));

    let spawned = Site::requests(&site.alpha);
    assert_eq!(spawned.len(), 1, "only the editor was opened: {spawned:?}");
    assert!(spawned[0].contains("\"nvim\",\"+call cursor(4, 2)\""), "{spawned:?}");
}
