//! The control connection: `GET /api/control`, subprotocol `rex.control.v1`, one JSON object per
//! text message. One thread owns the socket; callers send requests through a channel and wait
//! for the matching response.
use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail, ensure};
use serde_json::{Value, json};
use tungstenite::Message as WsMessage;

pub const API_REVISION: u64 = 7;
/// How long the worker waits on the socket before looking for requests to send.
const POLL: Duration = Duration::from_millis(15);

/// A server event, such as `block_closed`, with its fields. A session's events name it.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub name: String,
    pub session: Option<String>,
    pub payload: Value,
}

impl Event {
    /// Unwraps an `event` message, including the `session_event` envelope.
    pub fn from_message(message: &Value) -> Option<Self> {
        let event = &message["payload"];
        let name = event["type"].as_str()?;
        if name == "session_event" {
            let inner = &event["payload"]["event"];
            return Some(Self {
                name: inner["type"].as_str()?.to_owned(),
                session: event["payload"]["session_id"].as_str().map(str::to_owned),
                payload: inner["payload"].clone(),
            });
        }
        Some(Self { name: name.to_owned(), session: None, payload: event["payload"].clone() })
    }
}

type Reply = Sender<Result<Value>>;

pub struct Control {
    requests: Sender<(Value, Reply)>,
    pub events: Receiver<Event>,
    next_id: std::cell::Cell<u64>,
}

impl Control {
    pub fn connect(socket: &Path, name: &str) -> Result<Self> {
        let mut ws = crate::ws::open(socket, "/api/control", "rex.control.v1")?;
        let hello = json!({
            "type": "hello",
            "api_revision": API_REVISION,
            "api_min_revision": API_REVISION,
            "client": {"kind": "app", "platform": "linux", "name": name, "version": env!("CARGO_PKG_VERSION")},
        });
        ws.send(WsMessage::text(hello.to_string()))?;
        let welcome: Value = loop {
            match ws.read()? {
                WsMessage::Text(text) => break serde_json::from_str(&text)?,
                WsMessage::Close(frame) => bail!("server closed the control connection: {frame:?}"),
                _ => {}
            }
        };
        ensure!(
            welcome["type"] == "welcome" && welcome["protocol"] == "rex.control.v1",
            "unexpected first message: {welcome}"
        );
        ws.get_ref().set_read_timeout(Some(POLL))?;

        let (requests, inbox) = mpsc::channel::<(Value, Reply)>();
        let (events_tx, events) = mpsc::channel();
        std::thread::Builder::new()
            .name("rex-control".into())
            .spawn(move || {
                let mut pending: HashMap<String, Reply> = HashMap::new();
                let fail = |pending: &mut HashMap<String, Reply>, why: &str| {
                    for (_, reply) in pending.drain() {
                        let _ = reply.send(Err(anyhow!("{why}")));
                    }
                };
                loop {
                    loop {
                        match inbox.try_recv() {
                            Ok((request, reply)) => {
                                pending.insert(request["id"].as_str().unwrap_or_default().to_owned(), reply);
                                if let Err(error) = ws.send(WsMessage::text(request.to_string())) {
                                    fail(&mut pending, &format!("control connection failed: {error}"));
                                    return;
                                }
                            }
                            Err(mpsc::TryRecvError::Empty) => break,
                            Err(mpsc::TryRecvError::Disconnected) => {
                                let _ = ws.close(None);
                                return;
                            }
                        }
                    }
                    match ws.read() {
                        Ok(WsMessage::Text(text)) => {
                            let Ok(message) = serde_json::from_str::<Value>(&text) else { continue };
                            match message["type"].as_str() {
                                Some("response") => {
                                    let id = message["id"].as_str().unwrap_or_default();
                                    if let Some(reply) = pending.remove(id) {
                                        let _ = reply.send(response_result(message));
                                    }
                                }
                                Some("event") => {
                                    if let Some(event) = Event::from_message(&message) {
                                        let _ = events_tx.send(event);
                                    }
                                }
                                _ => {}
                            }
                        }
                        Ok(WsMessage::Close(frame)) => {
                            fail(&mut pending, &format!("server closed the control connection: {frame:?}"));
                            return;
                        }
                        Ok(_) => {}
                        Err(tungstenite::Error::Io(error))
                            if matches!(error.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                        Err(error) => {
                            fail(&mut pending, &format!("control connection failed: {error}"));
                            return;
                        }
                    }
                }
            })?;
        Ok(Self { requests, events, next_id: 1.into() })
    }

    /// Calls `method` and waits for its result payload.
    pub fn call(&self, method: &str, payload: Value) -> Result<Value> {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        let (reply, response) = mpsc::channel();
        self.requests
            .send((json!({"type": "request", "id": id.to_string(), "method": method, "payload": payload}), reply))
            .map_err(|_| anyhow!("control connection is closed"))?;
        response
            .recv_timeout(Duration::from_secs(15))
            .with_context(|| format!("waiting for {method}"))?
            .with_context(|| format!("calling {method}"))
    }

    /// Calls a block method, such as `com.superlogical.terminal.write`.
    pub fn block_call(&self, method: &str, session: &str, block: &str, args: Value) -> Result<Value> {
        self.call(method, json!({"session_id": session, "block_id": block, "args": args}))
    }
}

fn response_result(message: Value) -> Result<Value> {
    if let Some(error) = message.get("error").filter(|error| !error.is_null()) {
        let code = error["code"].as_str().unwrap_or("error");
        let text = error["message"].as_str().unwrap_or_default();
        bail!("{code}: {text}");
    }
    Ok(message.get("payload").cloned().unwrap_or(Value::Null))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_events_are_unwrapped() {
        let message = json!({"type": "event", "seq": 2, "payload": {"type": "session_event", "payload": {
            "session_id": "session:1", "event": {"type": "block_closed", "payload": {"block_id": "block:2"}}}}});
        let event = Event::from_message(&message).unwrap();
        assert_eq!(event.name, "block_closed");
        assert_eq!(event.session.as_deref(), Some("session:1"));
        assert_eq!(event.payload["block_id"], "block:2");
        let plain = json!({"type": "event", "seq": 1, "payload": {"type": "hosts_changed", "payload": {}}});
        assert_eq!(Event::from_message(&plain).unwrap().session, None);
    }

    #[test]
    fn a_response_with_an_error_fails() {
        let error = response_result(json!({"type": "response", "id": "1", "error": {"code": "not_found", "message": "no such session"}}));
        assert_eq!(error.unwrap_err().to_string(), "not_found: no such session");
        let ok = response_result(json!({"type": "response", "id": "1", "payload": {"a": 1}})).unwrap();
        assert_eq!(ok["a"], 1);
    }
}
