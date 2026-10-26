//! `dojo-mesh remote` (the sending side) and `dojo-mesh inject` (the receiving side) of remote input.

use crate::cli::{self, Env, FAILED, Failure, NEEDS_HUMAN, UNAVAILABLE, USAGE_ERROR, fail};
use crate::remote::{self, Action, Cursor, Event};
use crate::channel::{Input, Output};
use crate::method::Connection;
use crate::transport::Origin;
use crate::{niri, session};
use std::io::BufRead;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

const REMOTE_INJECT: &str = "exec \"${XDG_DATA_HOME:-$HOME/.local/share}/dojo/bin/dojo-mesh\" inject";
/// The same, asking only whether the peer would take input. A forced command ignores the client's
/// arguments but sees the client's command in `SSH_ORIGINAL_COMMAND`, which is how `--check` gets through.
const REMOTE_CHECK: &str = "exec \"${XDG_DATA_HOME:-$HOME/.local/share}/dojo/bin/dojo-mesh\" inject --check";

/// How long the receiver gathers a burst of events into one cursor movement.
const FRAME: Duration = Duration::from_millis(8);

fn capture_binary(env: &Env) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("DOJO_INPUT_CAPTURE").map(PathBuf::from) {
        return Some(path);
    }
    let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|path| path.is_absolute()).or_else(|| env.home.as_ref().map(|home| home.join(".local/share")))?;
    Some(data.join("dojo/bin/dojo-input-capture"))
}

/// `dojo-mesh remote PEER`: the pointer and keyboard drive the peer until Super+Alt+F11 (or
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
    // Connect once before the keyboard is taken. A key approval (1Password asks the first time, and again
    // after it locks) and every refusal on the other side need the person's own keyboard and eyes, and
    // the helper has the keyboard exclusively. The peer's gate runs on this call as it does on the real
    // one, and it ends at once without taking the seat.
    let check: Vec<String> = ["sh", "-c", REMOTE_CHECK].iter().map(|part| part.to_string()).collect();
    let checked = cli::call_peer(env, &roster, peer, "remote", "check before taking the cursor and keyboard", &check, Input::Null, Output::Inherit, Some(std::time::Duration::from_secs(60)))?;
    cli::outcome(peer, &checked.ended)?;
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

/// Who is on the other end of this SSH session, according to the mesh method.
fn sender(env: &Env, named: Option<String>) -> Result<String, String> {
    let address = remote::caller_address(std::env::var("SSH_CONNECTION").ok().as_deref()).ok_or("this was not started over a peer's connection")?;
    env.method.identity.who(&Connection { address, named })
}

/// `dojo-mesh inject`: replay events from standard input through an input seat, for a peer this
/// node has named in `accept-input` (ADR-0007). The peer's identity is Tailscale's answer about the
/// connection, never anything the sender says.
pub fn inject(env: &Env, rest: &[String]) -> Result<u8, Failure> {
    // `--as NAME` is how a peer's input key says who it is (its forced command in authorized_keys);
    // the method checks it against the roster, so it is only a claim until then.
    let (words, values) = cli::split(rest, &["--check"], &["--as"])?;
    if !words.positional.is_empty() {
        return Err(fail(USAGE_ERROR, "usage: dojo-mesh inject [--as NAME] [--check] (events as JSON lines on standard input)"));
    }
    let named = values.iter().find(|(flag, _)| flag == "--as").map(|(_, name)| name.clone());
    // `--check` asks whether input would be taken, runs every test the real thing does, and stops.
    let check = cli::has(&words, "--check") || std::env::var("SSH_ORIGINAL_COMMAND").is_ok_and(|command| command.contains(" inject --check"));
    if std::env::var("DOJO_ORIGIN").as_deref() == Ok("agent") {
        return Err(refuse("input from the agent is never accepted"));
    }
    let config = cli::load_config(env)?;
    let from = sender(env, named).map_err(|reason| refuse(&reason))?;
    if !remote::accepts(&config.accept_input, &from) {
        return Err(refuse(&format!("this machine does not accept input from {from}; add it to accept-input in mesh.toml")));
    }
    if session::locked() == Some(true) {
        return Err(refuse("the screen is locked here; unlock it first"));
    }
    let socket = niri::socket().ok_or_else(|| refuse("there is no desktop session on this machine"))?;

    // The real keyboard and pointer, where Niri offers them: binds fire, there is one cursor, and
    // every monitor works the way the hardware's would. An older Niri only has the seat below.
    let mut connection = niri::AgentSeat::connect(&socket).map_err(|message| fail(UNAVAILABLE, message))?;
    match connection.remote_input(Vec::new()) {
        Ok(()) if check => {
            eprintln!("dojo-mesh: input from {from} would be taken here");
            return Ok(0);
        }
        Ok(()) => return inject_as_hardware(connection, &from),
        Err(message) if message.contains("error parsing request") || message.contains("unknown variant") => {
            eprintln!("dojo-mesh: this Niri cannot take input as the real keyboard, so the peer's shortcuts will not work and there will be a second cursor until Niri is updated");
        }
        Err(message) => return Err(fail(FAILED, message)),
    }
    drop(connection);
    if check {
        return Ok(0);
    }

    inject_through_seat(&socket, &from)
}

/// The peer's input as this machine's own hardware (`Request::RemoteInput`). Niri holds what is
/// pressed, and lets go of it all when this connection closes.
fn inject_as_hardware(mut connection: niri::AgentSeat, from: &str) -> Result<u8, Failure> {
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
    let mut ended: Result<(), String> = Ok(());
    let mut told = std::collections::HashSet::new();
    'session: loop {
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
        let batch: Vec<serde_json::Value> = remote::coalesce(events).iter().map(remote_input).collect();
        if batch.is_empty() {
            continue;
        }
        if let Err(message) = connection.remote_input(batch) {
            // Niri turning down a batch it dislikes is not the end; Niri ending the session, or the
            // connection failing, is.
            if message.starts_with("Niri refused:") && !message.contains("remote input ended") && !message.contains("remote input refused") {
                if told.insert(message.clone()) {
                    eprintln!("dojo-mesh: skipped input: {message}");
                }
                continue;
            }
            ended = Err(message);
            break 'session;
        }
    }
    drop(connection);
    match ended {
        Ok(()) => Ok(0),
        Err(message) => {
            eprintln!("dojo-mesh: remote input ended: {message}");
            Err(fail(FAILED, message))
        }
    }
}

/// An event as Niri's `RemoteInput`.
fn remote_input(event: &Event) -> serde_json::Value {
    let state = |pressed: &bool| if *pressed { "Pressed" } else { "Released" };
    match event {
        Event::Key { code, pressed } => serde_json::json!({"Key": {"evdev": code, "state": state(pressed)}}),
        Event::Button { code, pressed } => serde_json::json!({"Button": {"button": code, "state": state(pressed)}}),
        Event::Motion { dx, dy } => serde_json::json!({"Motion": {"dx": dx, "dy": dy}}),
        Event::Axis { h, v } => serde_json::json!({"Axis": {"horizontal": h, "vertical": v}}),
    }
}

/// The peer's input through a seat of its own: a second cursor that cannot reach Niri's shortcuts.
/// For a Niri older than `RemoteInput`.
fn inject_through_seat(socket: &std::path::Path, from: &str) -> Result<u8, Failure> {
    let layout: Vec<remote::Rect> = niri::outputs(socket)
        .map_err(|message| fail(FAILED, message))?
        .into_iter()
        .map(|output| remote::Rect { name: output.name, x: output.x, y: output.y, width: output.width, height: output.height })
        .collect();
    let focused = niri::windows(socket).ok().and_then(|windows| windows.into_iter().find(|window| window.is_focused));

    let mut seat = niri::AgentSeat::connect(socket).map_err(|message| fail(UNAVAILABLE, message))?;
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

    let mut cursor = Cursor::new(layout).ok_or_else(|| fail(FAILED, "Niri has no output"))?;
    let mut pointer = Pointer { instant: true };
    let mut ended: Result<(), String> = Ok(());
    let mut told = std::collections::HashSet::new();
    // The seat's cursor starts where this one does, so the first movement is relative to something real.
    if let Err(message) = apply(&mut seat, &mut pointer, &cursor.start()) {
        let _ = seat.input(serde_json::json!("Destroy"));
        return Err(fail(FAILED, message));
    }
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
                if let Err(message) = apply_or_recover(&socket, &mut seat, &mut pointer, &action, &mut told) {
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

/// Niri turns down single events that are not a fault in the session: a click on empty desktop has
/// no client surface to land on, a key needs a mapped window to go to, and an application the
/// human is using (a launcher that just opened and took the keyboard) pauses the seat. Each gets one
/// repair, since the person sending is the one at the controls: the focused window becomes the key's
/// target, and the human's claim on the application is yielded again. Whatever still fails is
/// skipped. Only a broken connection to Niri ends the session.
fn apply_or_recover(
    socket: &std::path::Path,
    seat: &mut niri::AgentSeat,
    pointer: &mut Pointer,
    action: &Action,
    told: &mut std::collections::HashSet<String>,
) -> Result<(), String> {
    let Err(message) = apply(seat, pointer, action) else { return Ok(()) };
    if !is_refusal(&message) {
        return Err(message);
    }
    let repaired = if message.contains("human owns") {
        seat.yield_human_input().is_ok()
    } else if matches!(action, Action::Key { .. }) && message.contains("focus a mapped window") {
        let focused = niri::windows(socket).ok().and_then(|windows| windows.into_iter().find(|window| window.is_focused));
        focused.is_some_and(|window| seat.focus(window.id).is_ok())
    } else {
        false
    };
    if repaired {
        match apply(seat, pointer, action) {
            Ok(()) => return Ok(()),
            Err(again) if !is_refusal(&again) => return Err(again),
            Err(_) => {}
        }
    }
    if told.insert(message.clone()) {
        eprintln!("dojo-mesh: skipped an event: {message}");
    }
    Ok(())
}

/// A refusal or pause is Niri answering "not this one"; anything else is the connection failing.
fn is_refusal(message: &str) -> bool {
    message.starts_with("Niri refused:") || message.starts_with("Niri paused the input:")
}

/// How the receiver places its cursor. Niri's `Jump` puts it there at once; an older Niri lacks it
/// and only has `Move`, which animates for a tenth of a second or more and so lags behind a live mouse.
struct Pointer {
    instant: bool,
}

fn apply(seat: &mut niri::AgentSeat, pointer: &mut Pointer, action: &Action) -> Result<(), String> {
    let state = |pressed: &bool| if *pressed { "Pressed" } else { "Released" };
    let command = match action {
        Action::Move { output, x, y } => {
            if pointer.instant {
                match seat.input(serde_json::json!({"Jump": {"output": output, "x": x, "y": y}})) {
                    Err(message) if message.contains("unknown variant") => {
                        eprintln!("dojo-mesh: this Niri cannot place the cursor instantly, so it will lag until Niri is updated");
                        pointer.instant = false;
                    }
                    other => return other,
                }
            }
            serde_json::json!({"Move": {"output": output, "x": x, "y": y}})
        }
        // A cursor placed at once has nothing to wait for.
        Action::Wait if pointer.instant => return Ok(()),
        Action::Wait => serde_json::json!("Wait"),
        Action::Button { code, pressed } => serde_json::json!({"Button": {"button": code, "state": state(pressed)}}),
        Action::Axis { h, v } => serde_json::json!({"Axis": {"horizontal": h, "vertical": v}}),
        Action::Key { code, pressed } => serde_json::json!({"Key": {"evdev": code, "state": state(pressed)}}),
    };
    seat.input(command)
}
