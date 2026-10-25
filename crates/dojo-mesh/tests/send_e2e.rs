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

/// A Niri that lists `windows`, accepts every request and remembers each one. A spawn adds a
/// window, as a program that starts would, unless the file `silent` exists next to the socket,
/// which makes it behave like a locked Niri: it answers and does nothing. An agent seat's `Focus`
/// is answered with Niri's "human owns this application" pause while the file `paused` exists and
/// no `YieldHumanInput` has come in. Each connection is served for as long as it stays open, as a
/// seat's is.
fn fake_niri(path: &Path, windows: &'static str) -> Seen {
    let listener = UnixListener::bind(path).unwrap();
    let dir = path.parent().unwrap().to_path_buf();
    let seen: Seen = Arc::default();
    let record = seen.clone();
    let state = Arc::new(Mutex::new((serde_json::from_str::<Vec<serde_json::Value>>(windows).unwrap(), false)));
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (record, state, dir) = (record.clone(), state.clone(), dir.clone());
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    let line = line.trim().to_string();
                    let mut state = state.lock().unwrap();
                    let (listed, yielded) = &mut *state;
                    let ok = |body: &str| format!("{{\"Ok\":{body}}}");
                    let reply = if line == "\"Windows\"" {
                        ok(&format!("{{\"Windows\":{}}}", serde_json::Value::Array(listed.clone())))
                    } else if line == "\"YieldHumanInput\"" {
                        *yielded = true;
                        ok("{\"InputCoordination\":{\"revision\":1,\"human_owned_windows\":[]}}")
                    } else if line.contains("\"Create\"") {
                        ok("{\"AgentInputCreated\":{\"peer_id\":1,\"seat_name\":\"seat\"}}")
                    } else if line.contains("\"AgentInput\":{\"Focus\"") && dir.join("paused").exists() && (!*yielded || dir.join("never-yields").exists()) {
                        ok("{\"AgentInputPaused\":{\"reason\":\"human owns this application\",\"coordination\":{\"revision\":1,\"human_owned_windows\":[9]}}}")
                    } else {
                        if let Some(rest) = line.split("\"FocusWindow\":{\"id\":").nth(1) {
                            let id: u64 = rest.trim_end_matches(|c: char| !c.is_ascii_digit()).parse().unwrap_or(0);
                            for window in listed.iter_mut() {
                                window["is_focused"] = serde_json::Value::Bool(window["id"] == id);
                            }
                        }
                        if line.contains("\"Spawn\"") && !dir.join("silent").exists() {
                            let id = 100 + listed.len();
                            listed.push(serde_json::json!({"id": id, "title": "new", "app_id": "new", "pid": 1, "is_focused": true}));
                        }
                        "{\"Ok\":\"Handled\"}".to_string()
                    };
                    record.lock().unwrap().push(line);
                    if (&stream).write_all(format!("{reply}\n").as_bytes()).is_err() {
                        return;
                    }
                }
            });
        }
    });
    seen
}

const FAKE_LOGINCTL: &str = "#!/bin/sh\ncase \"$1\" in\nshow-user) echo Display=c1 ;;\nshow-session) if [ -f \"$HOME/locked\" ]; then echo LockedHint=yes; else echo LockedHint=no; fi ;;\nesac\n";

struct Site {
    root: PathBuf,
    alpha: Seen,
    beta: Seen,
}

const ALPHA_WINDOWS: &str = r#"[{"id": 7, "title": "Example", "app_id": "org.example.App", "pid": 1, "is_focused": true},
  {"id": 8, "title": "Other", "app_id": "mystery", "pid": 2, "is_focused": false},
  {"id": 9, "title": "The Page - Helium", "app_id": "helium", "pid": 3, "is_focused": false}]"#;

impl Site {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("dojo-send-{}-{}", std::process::id(), SITES.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&root);
        for dir in ["bin", "data/dojo/bin", "hosts/alpha/home", "hosts/alpha/config/dojo", "hosts/alpha/share/applications", "hosts/beta/home", "hosts/beta/config/dojo", "hosts/beta/share/applications"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        std::fs::write(root.join("bin/tailscale"), FAKE_TAILSCALE).unwrap();
        std::fs::write(root.join("bin/loginctl"), FAKE_LOGINCTL).unwrap();
        Command::new("chmod").arg("+x").arg(root.join("bin/tailscale")).arg(root.join("bin/loginctl")).status().unwrap();
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_dojo-mesh"), root.join("data/dojo/bin/dojo-mesh")).unwrap();
        std::fs::write(root.join("status.json"), STATUS).unwrap();
        std::fs::write(root.join("hosts/alpha/config/dojo/mesh.toml"), "membership-tag = \"tag:mesh\"\n").unwrap();
        for host in ["alpha", "beta"] {
            std::fs::write(root.join(format!("hosts/{host}/share/applications/helium.desktop")), "[Desktop Entry]\nName=Helium\nExec=helium\n").unwrap();
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
            .env("NIRI_SOCKET", self.root.join("hosts/alpha/niri.sock"))
            .env("USER", "tester")
            .env("DOJO_MESH_WINDOW_WAIT_MS", "600");
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).stdin(Stdio::null()).output().unwrap()
    }

    fn run_as(&self, origin: &str, args: &[&str]) -> Output {
        self.command().env("DOJO_MESH_ORIGIN", origin).args(args).stdin(Stdio::null()).output().unwrap()
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "*7  org.example.App  Example\n 8  mystery  Other\n 9  helium  The Page - Helium\n");
}

#[test]
fn sending_opens_the_application_on_the_peer_then_closes_it_here() {
    let site = Site::new();
    let result = site.run(&["send", "--to", "beta"]);
    assert_eq!(result.status.code(), Some(0), "{}", text(&result));
    assert!(text(&result).starts_with("Sent to beta: a fresh org.example.App; the window closes here."), "{}", text(&result));
    assert!(text(&result).contains("here: closed the window here"), "{}", text(&result));
    assert!(text(&result).contains("started fresh"), "{}", text(&result));
    let beta: Vec<String> = Site::requests(&site.beta).into_iter().filter(|line| line != "\"Windows\"").collect();
    assert_eq!(beta, [r#"{"Action":{"Spawn":{"command":["gtk-launch","org.example.App"]}}}"#]);
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
    assert!(!Site::requests(&site.beta).iter().any(|line| line.contains("Spawn")));
}

#[test]
fn a_locked_peer_refuses_and_the_window_stays() {
    let site = Site::new();
    std::fs::write(site.root.join("hosts/beta/home/locked"), "").unwrap();
    let result = site.run(&["send", "--to", "beta"]);
    assert_eq!(result.status.code(), Some(1), "{}", text(&result));
    assert!(text(&result).contains("the screen is locked here"), "{}", text(&result));
    assert!(!Site::requests(&site.beta).iter().any(|line| line.contains("Spawn")), "nothing was started");
    assert_eq!(Site::requests(&site.alpha), ["\"Windows\""], "nothing was closed");
}

#[test]
fn a_peer_that_starts_nothing_is_a_failure_and_the_window_stays() {
    let site = Site::new();
    std::fs::write(site.root.join("hosts/beta/silent"), "").unwrap();
    let result = site.run(&["send", "--to", "beta"]);
    assert_eq!(result.status.code(), Some(1), "{}", text(&result));
    assert!(text(&result).contains("gtk-launch did not open a window"), "{}", text(&result));
    assert_eq!(Site::requests(&site.alpha), ["\"Windows\""], "nothing was closed");
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

    let spawned: Vec<String> = Site::requests(&site.alpha).into_iter().filter(|line| line.contains("Spawn")).collect();
    assert_eq!(spawned.len(), 1, "only the editor was opened: {spawned:?}");
    assert!(spawned[0].contains("\"nvim\",\"+call cursor(4, 2)\""), "{spawned:?}");
}

#[test]
fn a_dry_run_says_what_would_be_sent_without_calling_anyone() {
    let site = Site::new();
    let result = site.run(&["send", "--dry-run", "--json"]);
    assert_eq!(result.status.code(), Some(0), "{}", text(&result));
    let value: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(value["window"], 7);
    assert_eq!(value["handoff"]["kind"], "relaunch");
    assert!(value["notes"][0].as_str().unwrap().contains("started fresh"));
    assert!(Site::requests(&site.beta).is_empty());
    assert_eq!(Site::requests(&site.alpha), ["\"Windows\""], "nothing was closed");
}

/// Gives alpha's Helium a history in which the page the window shows is known. Returns false when
/// there is no `sqlite3` to make one, so the browser tests are skipped rather than failed.
fn helium_history(site: &Site) -> bool {
    let dir = site.root.join("hosts/alpha/config/net.imput.helium/Default");
    std::fs::create_dir_all(&dir).unwrap();
    Command::new("sqlite3")
        .arg(dir.join("History"))
        .arg("create table urls(url text, title text, last_visit_time integer); insert into urls values ('https://example.com/page', 'The Page', 5), ('https://example.com/other', 'Other', 9);")
        .status()
        .is_ok_and(|status| status.success())
}

fn alpha_actions(site: &Site) -> Vec<String> {
    Site::requests(&site.alpha).into_iter().filter(|line| line != "\"Windows\"").collect()
}

const CLOSE_TAB_REQUESTS: [&str; 7] = [
    "\"YieldHumanInput\"",
    r#"{"AgentInput":{"Create":{"name":"dojo-mesh"}}}"#,
    r#"{"AgentInput":{"Focus":{"window_id":9}}}"#,
    r#"{"AgentInput":{"Targeted":{"command":{"Key":{"evdev":29,"state":"Pressed"}},"window_id":9}}}"#,
    r#"{"AgentInput":{"Targeted":{"command":{"Key":{"evdev":17,"state":"Pressed"}},"window_id":9}}}"#,
    r#"{"AgentInput":{"Targeted":{"command":{"Key":{"evdev":17,"state":"Released"}},"window_id":9}}}"#,
    r#"{"AgentInput":{"Targeted":{"command":{"Key":{"evdev":29,"state":"Released"}},"window_id":9}}}"#,
];

#[test]
fn sending_a_browser_tab_closes_only_that_tab_not_the_window() {
    let site = Site::new();
    if !helium_history(&site) {
        return;
    }
    let result = site.run(&["send", "--window", "9", "--to", "beta"]);
    assert_eq!(result.status.code(), Some(0), "{}", text(&result));
    assert!(text(&result).contains("Sent to beta: the page https://example.com/page; only this tab closes here, the window's other tabs stay."), "{}", text(&result));
    assert!(text(&result).contains("here: closed that tab here"), "{}", text(&result));
    assert_eq!(alpha_actions(&site), CLOSE_TAB_REQUESTS, "Ctrl+W went to that window through a seat, and the window was never closed");
    let beta: Vec<String> = Site::requests(&site.beta).into_iter().filter(|line| line.contains("Spawn")).collect();
    assert_eq!(beta, [r#"{"Action":{"Spawn":{"command":["xdg-open","https://example.com/page"]}}}"#]);
}

#[test]
fn keeping_the_source_touches_nothing_here() {
    let site = Site::new();
    if !helium_history(&site) {
        return;
    }
    let result = site.run(&["send", "--window", "9", "--to", "beta", "--keep-source", "--json"]);
    assert_eq!(result.status.code(), Some(0), "{}", text(&result));
    let value: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(value["status"], "sent");
    assert_eq!(value["closes"], serde_json::json!({"scope": "tab", "title": "The Page - Helium"}));
    assert_eq!(value["source"], "left open here");
    assert!(alpha_actions(&site).is_empty(), "{:?}", alpha_actions(&site));
}

#[test]
fn close_source_closes_the_tab_only_while_the_window_still_shows_it() {
    let site = Site::new();
    let wrong = site.run(&["close-source", "--window", "9", "--tab-title", "Something else - Helium"]);
    assert_eq!(wrong.status.code(), Some(1), "{}", text(&wrong));
    assert!(text(&wrong).contains("no longer shows"), "{}", text(&wrong));
    assert!(alpha_actions(&site).is_empty(), "no key was sent");

    let right = site.run(&["close-source", "--window", "9", "--tab-title", "The Page - Helium"]);
    assert_eq!(right.status.code(), Some(0), "{}", text(&right));
    assert_eq!(alpha_actions(&site), CLOSE_TAB_REQUESTS);

    let whole = site.run(&["close-source", "--window", "8"]);
    assert_eq!(whole.status.code(), Some(0));
    assert!(alpha_actions(&site).iter().any(|line| line.contains("CloseWindow") && line.contains("\"id\":8")));
}

#[test]
fn a_window_the_person_just_used_is_unpaused_by_the_operators_send_only() {
    let site = Site::new();
    std::fs::write(site.root.join("hosts/alpha/paused"), "").unwrap();
    let result = site.run(&["close-source", "--window", "9", "--tab-title", "The Page - Helium"]);
    assert_eq!(result.status.code(), Some(0), "the operator's call yields the person's input first: {}", text(&result));
}

#[test]
fn the_agent_cannot_override_the_persons_input_to_close_a_tab() {
    let site = Site::new();
    std::fs::write(site.root.join("hosts/alpha/paused"), "").unwrap();
    let result = site.run_as("agent", &["close-source", "--window", "9", "--tab-title", "The Page - Helium"]);
    assert_eq!(result.status.code(), Some(1), "{}", text(&result));
    assert!(text(&result).contains("is for the operator"), "{}", text(&result));
    assert!(alpha_actions(&site).is_empty(), "the agent never reached Niri's input: {:?}", alpha_actions(&site));
}

#[test]
fn a_seat_niri_still_pauses_leaves_the_tab_open_and_says_why() {
    let site = Site::new();
    std::fs::write(site.root.join("hosts/alpha/paused"), "").unwrap();
    std::fs::write(site.root.join("hosts/alpha/never-yields"), "").unwrap();
    let result = site.run(&["close-source", "--window", "9", "--tab-title", "The Page - Helium"]);
    assert_eq!(result.status.code(), Some(1), "{}", text(&result));
    assert!(text(&result).contains("Niri paused the input: human owns this application"), "{}", text(&result));
    assert!(text(&result).contains("close it yourself"), "{}", text(&result));
    assert!(!alpha_actions(&site).iter().any(|line| line.contains("Targeted") || line.contains("CloseWindow")), "no key and no close: {:?}", alpha_actions(&site));
}
