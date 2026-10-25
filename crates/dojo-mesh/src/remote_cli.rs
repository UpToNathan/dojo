//! `dojo-mesh remote` (the sending side) and `dojo-mesh inject` (the receiving side) of remote input.

use crate::cli::{self, Env, FAILED, Failure, NEEDS_HUMAN, UNAVAILABLE, USAGE_ERROR, fail};
use crate::remote::{self, Action, Cursor, Event};
use crate::ssh::{Input, Output};
use crate::transport::Origin;
use crate::{niri, session};
use std::io::BufRead;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

const REMOTE_INJECT: &str = "exec \"${XDG_DATA_HOME:-$HOME/.local/share}/dojo/bin/dojo-mesh\" inject";

/// How long the receiver gathers a burst of events into one cursor movement.
const FRAME: Duration = Duration::from_millis(8);

fn capture_binary(env: &Env) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("DOJO_INPUT_CAPTURE").map(PathBuf::from) {
        return Some(path);
    }
    let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|path| path.is_absolute()).or_else(|| env.home.as_ref().map(|home| home.join(".local/share")))?;
    Some(data.join("dojo/bin/dojo-input-capture"))
}

/// `dojo-mesh remote PEER`: the pointer and keyboard drive the peer until Super+F11 (or
/// Ctrl+Alt+Shift+Esc), or until the connection ends. The capture helper reads local input and the
/// peer replays it; see `inject`.
pub fn remote(env: &Env, rest: &[String]) -> Result<u8, Failure> {
    let [peer] = rest else { return Err(fail(USAGE_ERROR, "usage: dojo-mesh remote PEER")) };
    if env.origin == Origin::Agent {
        return Err(fail(NEEDS_HUMAN, "taking the cursor and keyboard is for the operator; the agent has no way to drive a peer's input"));
    }
    let capture = capture_binary(env).filter(|path| path.exists()).ok_or_else(|| fail(UNAVAILABLE, "dojo-input-capture is not installed (scripts/install-helpers.sh dojo-input-capture)"))?;
    let config = cli::load_config(env)?;
    let roster = cli::load_roster(env, &config);
    // The helper takes the whole keyboard and pointer, so it starts only for a call that can go ahead.
    if let Err(why) = crate::transport::resolve(&roster, peer) {
        return Err(cli::refuse(env, &roster.this().name, peer, "remote", "cursor and keyboard", &why));
    }
    let mut helper = Command::new(&capture)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|error| fail(UNAVAILABLE, format!("could not start {}: {error}", capture.display())))?;
    let events = helper.stdout.take().expect("stdout was piped");
    let argv: Vec<String> = ["sh", "-c", REMOTE_INJECT].iter().map(|part| part.to_string()).collect();
    let result = cli::call_peer(env, &roster, peer, "remote", "cursor and keyboard", &argv, Input::Stdio(Stdio::from(events)), Output::Inherit, None);
    // However the session ended, local input comes back the moment the helper is gone.
    let _ = helper.kill();
    let _ = helper.wait();
    cli::outcome(peer, &result?.ended)
}

fn refuse(reason: &str) -> Failure {
    eprintln!("dojo-mesh: remote input refused: {reason}");
    fail(NEEDS_HUMAN, reason)
}

/// Who is on the other end of this SSH session, according to Tailscale.
fn sender() -> Result<String, String> {
    let address = remote::caller_address(std::env::var("SSH_CONNECTION").ok().as_deref()).ok_or("this was not started over a peer's connection")?;
    let output = Command::new("tailscale").args(["whois", "--json", &address]).stdin(Stdio::null()).stderr(Stdio::null()).output().map_err(|error| format!("could not ask Tailscale who is connected: {error}"))?;
    if !output.status.success() {
        return Err("Tailscale did not know the connection".into());
    }
    remote::node_name(&String::from_utf8_lossy(&output.stdout))
}

/// `dojo-mesh inject`: replay events from standard input through an input seat, for a peer this
/// node has named in `accept-input` (ADR-0007). The peer's identity is Tailscale's answer about the
/// connection, never anything the sender says.
pub fn inject(env: &Env, rest: &[String]) -> Result<u8, Failure> {
    if !rest.is_empty() {
        return Err(fail(USAGE_ERROR, "usage: dojo-mesh inject (events as JSON lines on standard input)"));
    }
    if std::env::var("DOJO_ORIGIN").as_deref() == Ok("agent") {
        return Err(refuse("input from the agent is never accepted"));
    }
    let config = cli::load_config(env)?;
    let from = sender().map_err(|reason| refuse(&reason))?;
    if !remote::accepts(&config.accept_input, &from) {
        return Err(refuse(&format!("this machine does not accept input from {from}; add it to accept-input in mesh.toml")));
    }
    if session::locked() == Some(true) {
        return Err(refuse("the screen is locked here; unlock it first"));
    }
    let socket = niri::socket().ok_or_else(|| refuse("there is no desktop session on this machine"))?;
    let (output, width, height) = niri::outputs(&socket).map_err(|message| fail(FAILED, message))?.into_iter().next().ok_or_else(|| fail(FAILED, "Niri has no output"))?;
    let focused = niri::windows(&socket).ok().and_then(|windows| windows.into_iter().find(|window| window.is_focused));

    let mut seat = niri::AgentSeat::connect(&socket).map_err(|message| fail(UNAVAILABLE, message))?;
    seat.yield_human_input().map_err(|message| fail(FAILED, message))?;
    seat.create("dojo-remote").map_err(|message| fail(FAILED, message))?;
    if let Some(window) = &focused {
        seat.focus(window.id).map_err(|message| fail(FAILED, message))?;
    }
    eprintln!("dojo-mesh: taking input from {from}");

    let (lines, incoming) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if lines.send(line).is_err() {
                break;
            }
        }
    });

    let mut cursor = Cursor::new(width, height);
    let mut ended: Result<(), String> = Ok(());
    'session: loop {
        // A burst becomes one cursor movement; the stream ending (the sender let go, or the
        // connection dropped) ends the session.
        let Ok(first) = incoming.recv() else { break };
        let mut burst = vec![first];
        let deadline = std::time::Instant::now() + FRAME;
        while let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) {
            match incoming.recv_timeout(left) {
                Ok(line) => burst.push(line),
                Err(_) => break,
            }
        }
        let events: Vec<Event> = burst.iter().filter_map(|line| Event::parse(line).ok()).collect();
        for event in remote::coalesce(events) {
            for action in cursor.plan(&event) {
                if let Err(message) = apply(&mut seat, &output, &action) {
                    ended = Err(message);
                    break 'session;
                }
            }
        }
    }
    // Held keys and buttons are let go, and the seat goes with the connection.
    let _ = seat.input(serde_json::json!("Reset"));
    let _ = seat.input(serde_json::json!("Destroy"));
    match ended {
        Ok(()) => Ok(0),
        Err(message) => {
            eprintln!("dojo-mesh: remote input ended: {message}");
            Err(fail(FAILED, message))
        }
    }
}

fn apply(seat: &mut niri::AgentSeat, output: &str, action: &Action) -> Result<(), String> {
    let state = |pressed: &bool| if *pressed { "Pressed" } else { "Released" };
    seat.input(match action {
        Action::Move { x, y } => serde_json::json!({"Move": {"output": output, "x": x, "y": y}}),
        Action::Wait => serde_json::json!("Wait"),
        Action::Button { code, pressed } => serde_json::json!({"Button": {"button": code, "state": state(pressed)}}),
        Action::Axis { h, v } => serde_json::json!({"Axis": {"horizontal": h, "vertical": v}}),
        Action::Key { code, pressed } => serde_json::json!({"Key": {"evdev": code, "state": state(pressed)}}),
    })
}
