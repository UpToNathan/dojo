//! `dojo-mesh windows`, `send` and `receive`: sending a window to a peer.

use crate::adapter;
use crate::cli::{self, Env, FAILED, Failure, UNAVAILABLE, USAGE_ERROR, fail};
use crate::handoff::{self, Handoff, Reply};
use crate::{desktop, niri};
use serde::Serialize;
use std::io::Read;
use std::time::Duration;

/// The command found some of the window's files missing on the peer; nothing was sent.
pub const NEEDS_COPY: u8 = 5;

const REMOTE_RECEIVE: &str = "exec \"${XDG_DATA_HOME:-$HOME/.local/share}/dojo/bin/dojo-mesh\" receive";

fn local_niri() -> Result<std::path::PathBuf, Failure> {
    niri::socket().ok_or_else(|| fail(UNAVAILABLE, "Niri is not running here, so there are no windows to send"))
}

pub fn windows(rest: &[String]) -> Result<u8, Failure> {
    let json = match rest {
        [] => false,
        [flag] if flag == "--json" => true,
        _ => return Err(fail(USAGE_ERROR, "usage: dojo-mesh windows [--json]")),
    };
    let listed = niri::windows(&local_niri()?).map_err(|message| fail(FAILED, message))?;
    if json {
        println!("{}", serde_json::to_string_pretty(&listed).map_err(|error| fail(FAILED, error.to_string()))?);
    } else {
        for window in &listed {
            println!("{}{}  {}  {}", if window.is_focused { "*" } else { " " }, window.id, window.app_id.as_deref().unwrap_or("?"), window.title.as_deref().unwrap_or(""));
        }
    }
    Ok(0)
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
enum Outcome {
    Sent { peer: String, window: u64, notes: Vec<String> },
    Missing { peer: String, window: u64, files: Vec<String> },
}

pub fn send(env: &Env, rest: &[String]) -> Result<u8, Failure> {
    let mut window_id: Option<u64> = None;
    let mut peer: Option<String> = None;
    let (mut copy_missing, mut json, mut dry_run) = (false, false, false);
    let mut args = rest.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--window" => {
                let text = args.next().ok_or_else(|| fail(USAGE_ERROR, "--window needs a window id"))?;
                window_id = Some(text.parse().map_err(|_| fail(USAGE_ERROR, format!("--window needs a number, not {text:?}")))?);
            }
            "--to" => peer = Some(args.next().ok_or_else(|| fail(USAGE_ERROR, "--to needs a peer"))?.clone()),
            "--copy-missing" => copy_missing = true,
            "--json" => json = true,
            "--dry-run" => dry_run = true,
            other => return Err(fail(USAGE_ERROR, format!("unknown option {other}; usage: dojo-mesh send [--window ID] (--to PEER | --dry-run) [--copy-missing] [--json]"))),
        }
    }
    if peer.is_none() && !dry_run {
        return Err(fail(USAGE_ERROR, "send needs --to PEER (or --dry-run)"));
    }

    let socket = local_niri()?;
    let listed = niri::windows(&socket).map_err(|message| fail(FAILED, message))?;
    let window = match window_id {
        Some(id) => listed.iter().find(|window| window.id == id).ok_or_else(|| fail(FAILED, format!("there is no window {id}")))?,
        None => listed.iter().find(|window| window.is_focused).ok_or_else(|| fail(FAILED, "no window has focus"))?,
    };
    let entries = desktop::entries(&desktop::from_process());
    let var = |name: &str| std::env::var_os(name).map(std::path::PathBuf::from).filter(|path| path.is_absolute());
    let context = adapter::Context {
        probe: &adapter::System,
        entries: &entries,
        runtime_dir: var("XDG_RUNTIME_DIR"),
        config_dir: var("XDG_CONFIG_HOME").or_else(|| var("HOME").map(|home| home.join(".config"))),
    };
    let captured = adapter::capture(window, &context).map_err(|message| fail(FAILED, message))?;
    if dry_run {
        let preview = serde_json::json!({ "window": window.id, "handoff": captured.handoff, "notes": captured.notes });
        if json {
            println!("{preview}");
        } else {
            println!("{}", serde_json::to_string(&captured.handoff).map_err(|error| fail(FAILED, error.to_string()))?);
            for note in &captured.notes {
                println!("  {note}");
            }
        }
        return Ok(0);
    }
    let peer = peer.expect("checked above");

    let config = cli::load_config(env)?;
    let roster = cli::load_roster(env, &config);
    let request = serde_json::to_vec(&captured.handoff).map_err(|error| fail(FAILED, error.to_string()))?;
    let detail = format!("window {} ({}) as {}", window.id, window.app_id.as_deref().unwrap_or("?"), String::from_utf8_lossy(&request));
    let argv: Vec<String> = ["sh", "-c", REMOTE_RECEIVE].iter().map(|part| part.to_string()).collect();
    let mut copied = false;
    let reply = loop {
        let done = cli::call_peer(env, &roster, &peer, "send", &detail, &argv, crate::ssh::Input::Bytes(request.clone()), crate::ssh::Output::Capture, Some(Duration::from_secs(30)))?;
        let reply: Option<Reply> = serde_json::from_slice(&done.stdout).ok();
        let reply = match reply {
            Some(reply) => reply,
            None => {
                cli::outcome(&peer, &done.ended)?;
                return Err(fail(FAILED, format!("{peer} answered in a way that could not be read; is dojo-mesh up to date there?")));
            }
        };
        match reply {
            Reply::Missing { files } if copy_missing && !copied => {
                for file in &files {
                    cli::push(env, &roster, file, &peer, file)?;
                }
                copied = true;
            }
            other => break other,
        }
    };
    match reply {
        Reply::Opened { notes: peer_notes } => {
            let mut notes = captured.notes;
            notes.extend(peer_notes);
            if let Err(message) = niri::close_window(&socket, window.id) {
                notes.push(format!("the window here could not be closed: {message}"));
            }
            print_outcome(json, &Outcome::Sent { peer: peer.clone(), window: window.id, notes })?;
            Ok(0)
        }
        Reply::Missing { files } => {
            print_outcome(json, &Outcome::Missing { peer: peer.clone(), window: window.id, files })?;
            Ok(NEEDS_COPY)
        }
        Reply::Refused { reason } => Err(fail(FAILED, format!("{peer} did not open it: {reason}"))),
    }
}

fn print_outcome(json: bool, outcome: &Outcome) -> Result<(), Failure> {
    if json {
        println!("{}", serde_json::to_string(outcome).map_err(|error| fail(FAILED, error.to_string()))?);
        return Ok(());
    }
    match outcome {
        Outcome::Sent { peer, notes, .. } => {
            println!("Sent to {peer}.");
            for note in notes {
                println!("  {note}");
            }
        }
        Outcome::Missing { peer, files, .. } => {
            println!("{peer} does not have:");
            for file in files {
                println!("  {file}");
            }
            println!("Nothing was sent. Run again with --copy-missing to copy them across first.");
        }
    }
    Ok(())
}

/// How long the receiver waits for the new window before calling the handoff failed.
const WINDOW_WAIT: Duration = Duration::from_secs(10);

/// Starts the command, then waits for a new window, because Niri accepts a request to start a
/// program even when it then does nothing with it. A page opens as a tab in a browser that is
/// already running, which makes no new window, so it is trusted to the browser.
fn open(socket: &std::path::Path, handoff: &Handoff, command: &[String], notes: Vec<String>) -> Reply {
    let before: Vec<u64> = match niri::windows(socket) {
        Ok(windows) => windows.iter().map(|window| window.id).collect(),
        Err(message) => return Reply::Refused { reason: message },
    };
    if let Err(message) = niri::spawn(socket, command) {
        return Reply::Refused { reason: message };
    }
    if matches!(handoff, Handoff::Page { .. }) {
        return Reply::Opened { notes };
    }
    // Tests shorten the wait; nothing else sets it.
    let wait = std::env::var("DOJO_MESH_WINDOW_WAIT_MS").ok().and_then(|ms| ms.parse().ok()).map(Duration::from_millis).unwrap_or(WINDOW_WAIT);
    let started = std::time::Instant::now();
    while started.elapsed() < wait {
        std::thread::sleep(Duration::from_millis(150));
        if let Ok(windows) = niri::windows(socket)
            && windows.iter().any(|window| !before.contains(&window.id))
        {
            return Reply::Opened { notes };
        }
    }
    Reply::Refused { reason: format!("{} did not open a window within {} seconds", command[0], wait.as_secs()) }
}

/// The peer's side: read one handoff from stdin, open it in the operator's session, and answer.
pub fn receive(rest: &[String]) -> Result<u8, Failure> {
    if !rest.is_empty() {
        return Err(fail(USAGE_ERROR, "usage: dojo-mesh receive (a handoff as JSON on standard input)"));
    }
    let mut text = String::new();
    std::io::stdin().take(1 << 20).read_to_string(&mut text).map_err(|error| fail(USAGE_ERROR, error.to_string()))?;
    let reply = match serde_json::from_str::<Handoff>(&text) {
        Err(error) => Reply::Refused { reason: format!("not a handoff: {error}") },
        Ok(handoff) => {
            let dirs = desktop::from_process();
            match handoff::plan(&handoff, |path| path.exists(), |entry| desktop::has(entry, &dirs)) {
                Err(reply) => reply,
                Ok((command, notes)) => match niri::socket() {
                    None => Reply::Refused { reason: "there is no desktop session on this machine".into() },
                    Some(_) if crate::session::locked() == Some(true) => Reply::Refused { reason: "the screen is locked here; unlock it, then send again".into() },
                    Some(socket) => open(&socket, &handoff, &command, notes),
                },
            }
        }
    };
    println!("{}", serde_json::to_string(&reply).map_err(|error| fail(FAILED, error.to_string()))?);
    Ok(match reply {
        Reply::Opened { .. } => 0,
        Reply::Missing { .. } => NEEDS_COPY,
        Reply::Refused { .. } => FAILED,
    })
}
