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
    let (mut copy_missing, mut json) = (false, false);
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
            other => return Err(fail(USAGE_ERROR, format!("unknown option {other}; usage: dojo-mesh send [--window ID] --to PEER [--copy-missing] [--json]"))),
        }
    }
    let peer = peer.ok_or_else(|| fail(USAGE_ERROR, "send needs --to PEER"))?;

    let socket = local_niri()?;
    let listed = niri::windows(&socket).map_err(|message| fail(FAILED, message))?;
    let window = match window_id {
        Some(id) => listed.iter().find(|window| window.id == id).ok_or_else(|| fail(FAILED, format!("there is no window {id}")))?,
        None => listed.iter().find(|window| window.is_focused).ok_or_else(|| fail(FAILED, "no window has focus"))?,
    };
    let captured = adapter::capture(window, &desktop::entries(&desktop::from_process())).map_err(|message| fail(FAILED, message))?;

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
                    Some(socket) => match niri::spawn(&socket, &command) {
                        Ok(()) => Reply::Opened { notes },
                        Err(message) => Reply::Refused { reason: message },
                    },
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
