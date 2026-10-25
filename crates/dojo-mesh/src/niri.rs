//! Niri's IPC socket: the list of windows, closing one, and starting a program in the operator's
//! session. A command that arrives over SSH has no session environment, so the socket is found
//! through the user's systemd environment when it is not inherited.

use serde::Deserialize;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, serde::Serialize)]
pub struct Window {
    pub id: u64,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub app_id: Option<String>,
    #[serde(default)]
    pub pid: Option<i64>,
    #[serde(default)]
    pub is_focused: bool,
}

/// The value of `NAME=value` in `systemctl --user show-environment` output.
pub fn environment_value(listing: &str, name: &str) -> Option<String> {
    listing.lines().find_map(|line| line.strip_prefix(name)?.strip_prefix('=')).map(|value| value.trim().to_string()).filter(|value| !value.is_empty())
}

/// Where this user's Niri listens, if it runs.
pub fn socket() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("NIRI_SOCKET").map(PathBuf::from).filter(|path| path.exists()) {
        return Some(path);
    }
    let listing = Command::new("systemctl").args(["--user", "show-environment"]).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    environment_value(&String::from_utf8_lossy(&listing.stdout), "NIRI_SOCKET").map(PathBuf::from).filter(|path| path.exists())
}

/// Sends one request and returns the `Ok` part of the reply.
pub fn request(socket: &Path, request: &serde_json::Value) -> Result<serde_json::Value, String> {
    let mut stream = UnixStream::connect(socket).map_err(|error| format!("cannot reach Niri at {}: {error}", socket.display()))?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let mut line = request.to_string();
    line.push('\n');
    stream.write_all(line.as_bytes()).map_err(|error| format!("cannot talk to Niri: {error}"))?;
    let mut reply = String::new();
    BufReader::new(&stream).read_line(&mut reply).map_err(|error| format!("no answer from Niri: {error}"))?;
    let value: serde_json::Value = serde_json::from_str(&reply).map_err(|error| format!("unreadable answer from Niri: {error}"))?;
    match (value.get("Ok"), value.get("Err")) {
        (Some(ok), _) => Ok(ok.clone()),
        (_, Some(err)) => Err(format!("Niri refused: {}", err.as_str().map(str::to_string).unwrap_or_else(|| err.to_string()))),
        _ => Err("Niri gave an answer without a result".into()),
    }
}

pub fn windows(socket: &Path) -> Result<Vec<Window>, String> {
    let ok = request(socket, &serde_json::json!("Windows"))?;
    serde_json::from_value(ok.get("Windows").cloned().ok_or("Niri did not list windows")?).map_err(|error| format!("unreadable window list: {error}"))
}

pub fn close_window(socket: &Path, id: u64) -> Result<(), String> {
    request(socket, &serde_json::json!({"Action": {"CloseWindow": {"id": id}}})).map(drop)
}

pub fn focus_window(socket: &Path, id: u64) -> Result<(), String> {
    request(socket, &serde_json::json!({"Action": {"FocusWindow": {"id": id}}})).map(drop)
}

/// A seat of its own on a persistent connection, for sending keys to one window without moving the
/// human's focus. It goes away when dropped, because Niri ties a seat to its connection.
pub struct AgentSeat {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

const KEY_LEFT_CTRL: u32 = 29;
const KEY_W: u32 = 17;

impl AgentSeat {
    pub fn connect(socket: &Path) -> Result<Self, String> {
        let stream = UnixStream::connect(socket).map_err(|error| format!("cannot reach Niri at {}: {error}", socket.display()))?;
        stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
        Ok(Self { reader: BufReader::new(stream.try_clone().map_err(|error| error.to_string())?), writer: stream })
    }

    fn call(&mut self, request: &serde_json::Value) -> Result<serde_json::Value, String> {
        let mut line = request.to_string();
        line.push('\n');
        self.writer.write_all(line.as_bytes()).map_err(|error| format!("cannot talk to Niri: {error}"))?;
        let mut reply = String::new();
        self.reader.read_line(&mut reply).map_err(|error| format!("no answer from Niri: {error}"))?;
        let value: serde_json::Value = serde_json::from_str(&reply).map_err(|error| format!("unreadable answer from Niri: {error}"))?;
        if let Some(err) = value.get("Err") {
            return Err(format!("Niri refused: {}", err.as_str().map(str::to_string).unwrap_or_else(|| err.to_string())));
        }
        let ok = value.get("Ok").cloned().ok_or("Niri gave an answer without a result")?;
        if let Some(paused) = ok.get("AgentInputPaused") {
            return Err(format!("Niri paused the input: {}", paused["reason"].as_str().unwrap_or("someone else owns it")));
        }
        Ok(ok)
    }

    /// Ends the pause Niri puts on agent input into windows the person has just used. This is the
    /// operator's own command (`niri msg yield-human-input`); callers must not use it for the agent.
    pub fn yield_human_input(&mut self) -> Result<(), String> {
        self.call(&serde_json::json!("YieldHumanInput")).map(drop)
    }

    /// Keys, buttons, motion and scrolling for this desktop's real keyboard and pointer
    /// (`Request::RemoteInput`). Niri vets the caller before the first batch; an empty batch just
    /// asks it to.
    pub fn remote_input(&mut self, events: Vec<serde_json::Value>) -> Result<(), String> {
        self.call(&serde_json::json!({"RemoteInput": events})).map(drop)
    }

    /// An agent-input command that moves the pointer or sends a button, key or scroll.
    pub fn input(&mut self, command: serde_json::Value) -> Result<(), String> {
        self.call(&serde_json::json!({"AgentInput": command})).map(drop)
    }

    pub fn create(&mut self, name: &str) -> Result<(), String> {
        self.call(&serde_json::json!({"AgentInput": {"Create": {"name": name}}})).map(drop)
    }

    pub fn focus(&mut self, window: u64) -> Result<(), String> {
        self.call(&serde_json::json!({"AgentInput": {"Focus": {"window_id": window}}})).map(drop)
    }

    fn key(&mut self, window: u64, evdev: u32, pressed: bool) -> Result<(), String> {
        let state = if pressed { "Pressed" } else { "Released" };
        self.call(&serde_json::json!({"AgentInput": {"Targeted": {"window_id": window, "command": {"Key": {"evdev": evdev, "state": state}}}}})).map(drop)
    }

    /// Ctrl+W in the window, which closes the tab in front in a browser. Releases are always sent.
    pub fn close_tab_keys(&mut self, window: u64) -> Result<(), String> {
        let pressed = self.key(window, KEY_LEFT_CTRL, true).and_then(|()| self.key(window, KEY_W, true)).and_then(|()| self.key(window, KEY_W, false));
        let released = self.key(window, KEY_LEFT_CTRL, false);
        pressed.and(released)
    }
}

/// An output's place in the logical layout.
#[derive(Clone, Debug, PartialEq)]
pub struct OutputRect {
    pub name: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Every output with its position and logical size, from Niri's `Outputs` request, sorted by
/// position (left to right, then top to bottom) so the first is the leftmost.
pub fn outputs(socket: &Path) -> Result<Vec<OutputRect>, String> {
    let ok = request(socket, &serde_json::json!("Outputs"))?;
    let listed = ok.get("Outputs").and_then(|outputs| outputs.as_object()).ok_or("Niri did not list outputs")?;
    let mut found: Vec<OutputRect> = listed
        .iter()
        .filter_map(|(name, output)| {
            let logical = &output["logical"];
            Some(OutputRect { name: name.clone(), x: logical["x"].as_f64().unwrap_or(0.), y: logical["y"].as_f64().unwrap_or(0.), width: logical["width"].as_f64()?, height: logical["height"].as_f64()? })
        })
        .collect();
    found.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)).then(a.name.cmp(&b.name)));
    Ok(found)
}

/// Starts `command` as a child of Niri, so it runs in the operator's session with its environment.
pub fn spawn(socket: &Path, command: &[String]) -> Result<(), String> {
    request(socket, &serde_json::json!({"Action": {"Spawn": {"command": command}}})).map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    #[test]
    fn reads_a_variable_from_the_systemd_environment() {
        let listing = "DISPLAY=:0\nNIRI_SOCKET=/run/user/1/niri.sock\nNIRI_SOCKETX=no\n";
        assert_eq!(environment_value(listing, "NIRI_SOCKET").as_deref(), Some("/run/user/1/niri.sock"));
        assert_eq!(environment_value(listing, "WAYLAND_DISPLAY"), None);
        assert_eq!(environment_value("NIRI_SOCKET=\n", "NIRI_SOCKET"), None);
    }

    fn serve(replies: Vec<&'static str>) -> (PathBuf, std::thread::JoinHandle<Vec<String>>) {
        let dir = std::env::temp_dir().join(format!("dojo-mesh-niri-{}-{}", std::process::id(), replies.len()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("niri.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let handle = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for reply in replies {
                let (stream, _) = listener.accept().unwrap();
                let mut line = String::new();
                BufReader::new(&stream).read_line(&mut line).unwrap();
                seen.push(line.trim().to_string());
                (&stream).write_all(format!("{reply}\n").as_bytes()).unwrap();
            }
            seen
        });
        (path, handle)
    }

    #[test]
    fn speaks_niris_request_and_reply_lines() {
        let (path, server) = serve(vec![
            r#"{"Ok":{"Windows":[{"id":5,"title":"t","app_id":"kitty","pid":9,"is_focused":true,"layout":{}}]}}"#,
            r#"{"Ok":"Handled"}"#,
            r#"{"Ok":"Handled"}"#,
            r#"{"Err":"no such window"}"#,
        ]);
        let listed = windows(&path).unwrap();
        assert_eq!(listed, [Window { id: 5, title: Some("t".into()), app_id: Some("kitty".into()), pid: Some(9), is_focused: true }]);
        spawn(&path, &["kitty".into(), "--directory".into(), "/tmp".into()]).unwrap();
        close_window(&path, 5).unwrap();
        assert_eq!(close_window(&path, 6).unwrap_err(), "Niri refused: no such window");
        let seen = server.join().unwrap();
        assert_eq!(seen[0], "\"Windows\"");
        assert_eq!(seen[1], r#"{"Action":{"Spawn":{"command":["kitty","--directory","/tmp"]}}}"#);
        assert_eq!(seen[2], r#"{"Action":{"CloseWindow":{"id":5}}}"#);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
