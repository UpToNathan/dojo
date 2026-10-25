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
