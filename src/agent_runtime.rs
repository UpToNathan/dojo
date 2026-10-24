//! A client for the Dojo agent runtime's private socket, for sending it a typed message.
//!
//! The runtime is `dojo-voice.service`. It runs with or without a microphone, and a message sent
//! here takes the same path as a transcript: it joins the active conversation and the shell shows
//! it and the reply as speech bubbles.

use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

/// The runtime accepts messages up to this many bytes.
const MAX_MESSAGE_BYTES: usize = 16 * 1024;

pub fn socket_path() -> Result<PathBuf, String> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").ok_or("XDG_RUNTIME_DIR is required")?;
    Ok(PathBuf::from(runtime).join("dojo-voice.sock"))
}

/// Sends `text` to the agent as a new message in the active conversation.
pub fn send_text(text: &str) -> Result<(), String> {
    send_text_at(&socket_path()?, text)
}

pub fn send_text_at(path: &Path, text: &str) -> Result<(), String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("Type a message first".into());
    }
    if text.len() > MAX_MESSAGE_BYTES {
        return Err("That message is too long to send".into());
    }
    if text.contains('\0') {
        return Err("A message cannot contain a NUL character".into());
    }
    let mut socket = UnixStream::connect(path).map_err(|error| {
        format!("Could not reach the Dojo agent runtime (dojo-voice.service): {error}")
    })?;
    socket
        .set_write_timeout(Some(Duration::from_secs(5)))
        .and_then(|()| socket.set_read_timeout(Some(Duration::from_secs(45))))
        .map_err(|error| error.to_string())?;
    let request = serde_json::to_string(&json!({"action": "send", "text": text}))
        .map_err(|error| error.to_string())?;
    writeln!(socket, "{request}").map_err(|error| error.to_string())?;
    let mut response = String::new();
    BufReader::new(socket)
        .read_line(&mut response)
        .map_err(|error| format!("The Dojo agent runtime did not respond: {error}"))?;
    let value: Value = serde_json::from_str(&response)
        .map_err(|error| format!("Invalid Dojo agent runtime response: {error}"))?;
    match value.get("error").and_then(Value::as_str) {
        Some(error) => Err(error.to_owned()),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    /// A runtime that answers one request with `reply` and returns what it was sent.
    fn serve_once(path: &Path, reply: &'static str) -> std::thread::JoinHandle<String> {
        let listener = UnixListener::bind(path).unwrap();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            writeln!(&stream, "{reply}").unwrap();
            line
        })
    }

    #[test]
    fn a_message_is_sent_as_a_send_request_and_success_is_silent() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("dojo-voice.sock");
        let server = serve_once(&path, r#"{"threadId":"t1"}"#);
        send_text_at(&path, "  what time is it?  ").unwrap();
        let request: Value = serde_json::from_str(&server.join().unwrap()).unwrap();
        assert_eq!(
            request,
            json!({"action": "send", "text": "what time is it?"})
        );
    }

    #[test]
    fn runtime_errors_and_a_missing_runtime_are_reported() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("dojo-voice.sock");
        let server = serve_once(&path, r#"{"error":"No model is selected"}"#);
        assert_eq!(
            send_text_at(&path, "hi"),
            Err("No model is selected".into())
        );
        server.join().unwrap();
        let missing = directory.path().join("absent.sock");
        let error = send_text_at(&missing, "hi").unwrap_err();
        assert!(
            error.contains("Could not reach the Dojo agent runtime"),
            "{error}"
        );
    }

    #[test]
    fn unusable_messages_are_refused_before_connecting() {
        let path = Path::new("/nonexistent/never-connected.sock");
        assert_eq!(
            send_text_at(path, "  \n "),
            Err("Type a message first".into())
        );
        assert!(
            send_text_at(path, &"x".repeat(MAX_MESSAGE_BYTES + 1))
                .unwrap_err()
                .contains("too long")
        );
        assert!(send_text_at(path, "a\0b").unwrap_err().contains("NUL"));
    }
}
