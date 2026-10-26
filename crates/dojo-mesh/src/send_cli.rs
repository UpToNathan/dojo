//! `dojo-mesh windows`, `send` and `receive`: sending a window to a peer.

use crate::adapter;
use crate::cli::{self, Env, FAILED, Failure, UNAVAILABLE, USAGE_ERROR, fail};
use crate::handoff::{self, Closes, Handoff, Reply, Scope};
use crate::transport::Origin;
use crate::{desktop, niri, source};
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
    Sent { peer: String, window: u64, sent: String, closes: Closes, source: String, notes: Vec<String> },
    Missing { peer: String, window: u64, files: Vec<String> },
}

pub fn send(env: &Env, rest: &[String]) -> Result<u8, Failure> {
    let mut window_id: Option<u64> = None;
    let mut peer: Option<String> = None;
    let (mut copy_missing, mut json, mut dry_run, mut keep_source) = (false, false, false, false);
    let mut scope = Scope::Tab;
    let mut thread: Option<String> = None;
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
            "--scope" => {
                let text = args.next().ok_or_else(|| fail(USAGE_ERROR, "--scope needs tab or window"))?;
                scope = Scope::parse(text).ok_or_else(|| fail(USAGE_ERROR, format!("--scope needs tab or window, not {text:?}")))?;
            }
            "--thread" => thread = Some(args.next().ok_or_else(|| fail(USAGE_ERROR, "--thread needs a thread id"))?.clone()),
            "--keep-source" => keep_source = true,
            other => return Err(fail(USAGE_ERROR, format!("unknown option {other}; usage: dojo-mesh send [--window ID] (--to PEER | --dry-run) [--scope tab|window] [--thread ID] [--copy-missing] [--keep-source] [--json]"))),
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
        t3_home: var("T3CODE_HOME").or_else(|| var("HOME").map(|home| home.join(".t3"))),
        thread,
    };
    let captured = adapter::capture(window, &context, scope).map_err(|message| fail(FAILED, message))?;
    if dry_run {
        let sent = handoff::summary(&captured.handoff, &captured.closes);
        let preview = serde_json::json!({ "window": window.id, "handoff": captured.handoff, "closes": captured.closes, "sent": sent, "scopes": captured.scopes, "tab_count": captured.tab_count, "threads": captured.threads, "notes": captured.notes });
        if json {
            println!("{preview}");
        } else {
            println!("{sent}");
            if captured.scopes.contains(&Scope::Window) {
                println!("  (this window can also be sent whole: --scope window)");
            }
            if let Some(thread) = captured.threads.first().filter(|_| context.thread.is_none()) {
                println!("  thread: {}{} (--thread ID picks another of the {} recent)", thread.title, thread.project.as_deref().map(|project| format!(" in {project}")).unwrap_or_default(), captured.threads.len());
            }
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
        let done = cli::call_peer(env, &roster, &peer, "send", &detail, &argv, crate::channel::Input::Bytes(request.clone()), crate::channel::Output::Capture, Some(Duration::from_secs(30)))?;
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
            let sent = handoff::summary(&captured.handoff, &captured.closes);
            let source = if keep_source {
                "left open here".to_string()
            } else {
                match source::close(&source::System { socket: socket.clone(), runtime_dir: context.runtime_dir.clone(), operator: env.origin == Origin::Operator }, window.id, &captured.closes) {
                    Ok(done) => done,
                    Err(message) => {
                        notes.push(message);
                        "left open here".to_string()
                    }
                }
            };
            print_outcome(json, &Outcome::Sent { peer: peer.clone(), window: window.id, sent, closes: captured.closes, source, notes })?;
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
        Outcome::Sent { peer, sent, source, notes, .. } => {
            println!("Sent to {peer}: {sent}.");
            println!("  here: {source}");
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
    if matches!(handoff, Handoff::Page { .. } | Handoff::T3Thread { .. }) {
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

/// `dojo-mesh close-source --window ID [--tab-title TITLE]`: close what was sent, after the peer
/// opened it. With a title only the tab showing it closes (see [`source::close`]).
pub fn close_source(env: &Env, rest: &[String]) -> Result<u8, Failure> {
    let mut window: Option<u64> = None;
    let mut title: Option<String> = None;
    let mut explicit: Option<Closes> = None;
    let mut args = rest.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--window" => {
                let text = args.next().ok_or_else(|| fail(USAGE_ERROR, "--window needs a window id"))?;
                window = Some(text.parse().map_err(|_| fail(USAGE_ERROR, format!("--window needs a number, not {text:?}")))?);
            }
            "--tab-title" => title = Some(args.next().ok_or_else(|| fail(USAGE_ERROR, "--tab-title needs the tab's window title"))?.clone()),
            "--closes" => {
                let text = args.next().ok_or_else(|| fail(USAGE_ERROR, "--closes needs the closes object from `send --json`"))?;
                explicit = Some(serde_json::from_str::<Closes>(text).map_err(|error| fail(USAGE_ERROR, format!("--closes is not understood: {error}")))?);
            }
            other => return Err(fail(USAGE_ERROR, format!("unknown option {other}; usage: dojo-mesh close-source --window ID [--tab-title TITLE | --closes JSON]"))),
        }
    }
    let window = window.ok_or_else(|| fail(USAGE_ERROR, "close-source needs --window ID"))?;
    let closes = match (explicit, title) {
        (Some(closes), _) => closes,
        (None, Some(title)) => Closes::Tab { title },
        (None, None) => Closes::Window,
    };
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from).filter(|dir| dir.is_absolute());
    let done = source::close(&source::System { socket: local_niri()?, runtime_dir, operator: env.origin == Origin::Operator }, window, &closes).map_err(|message| fail(FAILED, message))?;
    println!("{done}");
    Ok(0)
}

/// The peer's side: read one handoff from stdin, open it in the operator's session, and answer.
pub fn receive(env: &Env, rest: &[String]) -> Result<u8, Failure> {
    if !rest.is_empty() {
        return Err(fail(USAGE_ERROR, "usage: dojo-mesh receive (a handoff as JSON on standard input)"));
    }
    let mut text = String::new();
    std::io::stdin().take(1 << 20).read_to_string(&mut text).map_err(|error| fail(USAGE_ERROR, error.to_string()))?;
    let reply = match serde_json::from_str::<Handoff>(&text) {
        Err(error) => Reply::Refused { reason: format!("not a handoff: {error}") },
        Ok(handoff) => {
            let dirs = desktop::from_process();
            let planned = match &handoff {
                // Who sent it is what the mesh method says about the connection. Under the roster method a
                // node key names nobody, so the sender's own statement (`DOJO_FROM`) is checked against the
                // signed roster instead, as `inject --as` is; a member holds a full shell here already, so a
                // false claim gains nothing but a window connected to another member.
                Handoff::RemoteTerminal { directory, program } => match (crate::remote_cli::sender(env, std::env::var("DOJO_FROM").ok()), std::env::current_exe()) {
                    (Ok(origin), Ok(mesh)) => handoff::plan_remote_terminal(directory, program, &origin, &mesh),
                    (Err(reason), _) => Err(Reply::Refused { reason: format!("cannot tell who sent this: {reason}") }),
                    (_, Err(error)) => Err(Reply::Refused { reason: format!("cannot find this installation of dojo-mesh: {error}") }),
                },
                other => handoff::plan_with(other, |path| path.exists(), |entry| desktop::has(entry, &dirs), |entry| desktop::exec_of(entry, &dirs)),
            };
            match planned {
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
