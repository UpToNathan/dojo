//! Attaching a view to one terminal of a session: find or create the session by label, claim
//! its terminal, and keep a libghostty mirror of it current from the data stream.
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow};
use serde_json::{Value, json};

use crate::control::Control;
use crate::data::{self, Chunk, DataStream};
use crate::stream::{Demux, Message, Piece};
use crate::terminal::{KeyInput, Terminal};

pub struct Options {
    pub socket: PathBuf,
    /// The session's label; an existing session with it is reused.
    pub label: String,
    /// The program a new session runs, started without a shell. Empty runs the login shell.
    pub command: Vec<String>,
    pub cwd: Option<String>,
    pub cols: u16,
    pub rows: u16,
    /// Shown to other clients, such as `dojo-terminal`.
    pub client_name: String,
}

/// What the stream thread shares with the view.
pub struct Shared {
    pub terminal: Mutex<Option<Terminal>>,
    /// The program exited or the block closed: the view should go away.
    pub ended: AtomicBool,
    /// Why the stream stopped, if it failed.
    pub error: Mutex<Option<String>>,
}

pub struct Attachment {
    control: Control,
    pub session: String,
    pub block: String,
    pub shared: Arc<Shared>,
}

impl Attachment {
    /// Opens the session and starts streaming its terminal. `wake` runs on the stream thread
    /// whenever the terminal changed or ended.
    pub fn open(options: Options, wake: impl Fn() + Send + 'static) -> Result<Self> {
        let control = Control::connect(&options.socket, &options.client_name)?;
        let sessions = control.call("session.list", json!({}))?;
        let existing = sessions["sessions"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|session| session["label"] == options.label.as_str())
            .and_then(|session| session["session_id"].as_str())
            .map(str::to_owned);
        let create = |control: &Control| -> Result<String> {
            let created = control.call("session.create", create_payload(&options))?;
            Ok(created["session_id"].as_str().context("session.create gave no session_id")?.to_owned())
        };
        let mut session = match existing {
            Some(session) => session,
            None => create(&control)?,
        };
        control.call("session.attach", json!({"session_id": session}))?;
        let mut view = control.call("session.view", json!({"session_id": session}))?;
        if terminal_block(&view).is_none() {
            // Its program exited and left the session empty: start the session again.
            control.call("session.destroy", json!({"session_id": session}))?;
            session = create(&control)?;
            control.call("session.attach", json!({"session_id": session}))?;
            view = control.call("session.view", json!({"session_id": session}))?;
        }
        let block = terminal_block(&view).context("the session shows no terminal")?;
        control.call("session.own_block", json!({"session_id": session, "block_ids": [block], "mode": "take"}))?;
        control.block_call(
            "com.superlogical.terminal.resize",
            &session,
            &block,
            json!({"columns": options.cols.max(1), "rows": options.rows.max(1)}),
        )?;
        let ticket = control.call(
            "data.connect",
            json!({"session_id": session, "block_id": block, "data_ref_id": "terminal", "subprotocol": data::SUBPROTOCOL}),
        )?;
        let ticket = data::decode_ticket(ticket["ticket"].as_str().context("data.connect gave no ticket")?)?;
        let stream = DataStream::open(&options.socket, &ticket)?;

        let shared = Arc::new(Shared { terminal: Mutex::new(None), ended: AtomicBool::new(false), error: Mutex::new(None) });
        let thread_shared = shared.clone();
        std::thread::Builder::new().name("rex-data".into()).spawn(move || {
            if let Err(error) = run_stream(stream, &thread_shared, &wake) {
                *thread_shared.error.lock().unwrap() = Some(format!("{error:#}"));
            }
            thread_shared.ended.store(true, Ordering::SeqCst);
            wake();
        })?;
        Ok(Self { control, session, block, shared })
    }

    /// Types bytes into the terminal.
    pub fn write(&self, bytes: &[u8]) -> Result<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        let data = String::from_utf8_lossy(bytes);
        // Typing does not wait for the server's answer; the echo comes back on the stream.
        self.control.send(
            "com.superlogical.terminal.write",
            json!({"session_id": self.session, "block_id": self.block, "args": {"data": data}}),
        )
    }

    /// Encodes a key with the terminal's current modes and types it.
    pub fn key(&self, key: &KeyInput) -> Result<()> {
        let bytes = match self.shared.terminal.lock().unwrap().as_mut() {
            Some(terminal) => terminal.encode_key(key),
            None => return Ok(()),
        };
        self.write(&bytes)
    }

    pub fn resize(&self, cols: u16, rows: u16, cell_width_px: u32, cell_height_px: u32) -> Result<()> {
        let mut args = json!({"columns": cols.max(1), "rows": rows.max(1)});
        if cell_width_px > 0 && cell_height_px > 0 {
            args["cell_width_px"] = json!(cell_width_px);
            args["cell_height_px"] = json!(cell_height_px);
        }
        self.control
            .block_call("com.superlogical.terminal.resize", &self.session, &self.block, args)
            .map(drop)
    }
}

fn create_payload(options: &Options) -> Value {
    let mut block_options = json!({"size": {"columns": options.cols.max(1), "rows": options.rows.max(1)}});
    if !options.command.is_empty() {
        block_options["shell"] = json!("none");
        block_options["command"] = json!(options.command);
    }
    if let Some(cwd) = &options.cwd {
        block_options["cwd"] = json!(cwd);
    }
    json!({
        "label": options.label,
        "initial_windows": [{"layout": {"block": {"flavor": "com.superlogical.terminal.shell", "options": block_options}}}],
    })
}

/// The block a view shows: the active window's focused block, or its first tiled block.
fn terminal_block(view: &Value) -> Option<String> {
    let windows = view["windows"].as_array()?;
    let active = view["active_window_id"].as_str();
    let window = windows
        .iter()
        .find(|window| active.is_some() && window["window_id"].as_str() == active)
        .or_else(|| windows.first())?;
    if let Some(block) = window["focused_block_id"].as_str() {
        return Some(block.to_owned());
    }
    window["layers"]
        .as_array()?
        .iter()
        .flat_map(|layer| layer["blocks"].as_array().into_iter().flatten())
        .find_map(|block| block["block_id"].as_str())
        .map(str::to_owned)
}

fn run_stream(mut stream: DataStream, shared: &Shared, wake: &dyn Fn()) -> Result<()> {
    let (terminal, rest) = Terminal::from_snapshot(&mut || match stream.next()? {
        Chunk::Bytes(bytes) => Ok(Some(bytes)),
        Chunk::End => Ok(None),
    })?;
    *shared.terminal.lock().unwrap() = Some(terminal);
    let mut demux = Demux::new(stream.welcome.nonce);
    let mut pending = rest;
    loop {
        if !pending.is_empty() {
            let mut guard = shared.terminal.lock().unwrap();
            let terminal = guard.as_mut().expect("terminal restored");
            for piece in demux.feed(&pending) {
                match piece {
                    Piece::Bytes(bytes) => terminal.write(&bytes),
                    Piece::Message(Message::ChildExited { .. }) => {
                        drop(guard);
                        return Ok(());
                    }
                    Piece::Message(message) => terminal.apply(&message)?,
                }
            }
            drop(guard);
            wake();
        }
        pending = match next_with_resume(&mut stream) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => return Ok(()),
            Err(error) => return Err(error),
        };
    }
}

/// The next chunk; a dropped connection resumes while the server still holds the stream.
fn next_with_resume(stream: &mut DataStream) -> Result<Option<Vec<u8>>> {
    let mut failure = match stream.next() {
        Ok(Chunk::Bytes(bytes)) => return Ok(Some(bytes)),
        Ok(Chunk::End) => return Ok(None),
        Err(error) => error,
    };
    let deadline = Instant::now() + Duration::from_millis(stream.welcome.resume_window_ms.min(30_000) as u64);
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(250));
        match stream.reconnect() {
            Ok(()) => return next_with_resume(stream),
            Err(error) => failure = error,
        }
    }
    Err(anyhow!(failure).context("the data connection dropped and could not resume"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_view_names_the_focused_block() {
        let view = json!({"active_window_id": "w2", "windows": [
            {"window_id": "w1", "layers": [{"blocks": [{"block_id": "b1"}]}]},
            {"window_id": "w2", "focused_block_id": "b3", "layers": [{"blocks": [{"block_id": "b2"}, {"block_id": "b3"}]}]},
        ]});
        assert_eq!(terminal_block(&view).as_deref(), Some("b3"));
        let unfocused = json!({"windows": [{"window_id": "w1", "layers": [{"blocks": [{"block_id": "b1"}]}]}]});
        assert_eq!(terminal_block(&unfocused).as_deref(), Some("b1"));
    }

    #[test]
    fn a_command_runs_without_a_shell() {
        let options = Options {
            socket: PathBuf::new(),
            label: "dojo-quick".into(),
            command: vec!["env".into(), "A=1".into(), "rush".into()],
            cwd: Some("/tmp".into()),
            cols: 90,
            rows: 20,
            client_name: "test".into(),
        };
        let payload = create_payload(&options);
        let block = &payload["initial_windows"][0]["layout"]["block"];
        assert_eq!(block["options"]["shell"], "none");
        assert_eq!(block["options"]["command"][2], "rush");
        assert_eq!(block["options"]["size"]["columns"], 90);
        assert_eq!(payload["label"], "dojo-quick");
    }
}
