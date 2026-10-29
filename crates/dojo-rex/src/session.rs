//! A whole Rex session for one view: its tabs (Rex windows) and panes (the blocks of the active
//! window's layout), each pane mirrored from its own data stream.
use std::collections::{HashMap, HashSet};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

use anyhow::{Context as _, Result};
use serde_json::{Value, json};

use crate::attach::{Shared, run_stream};
use crate::control::Control;
use crate::data::{self, DataStream};
use crate::layout::SessionView;
use crate::terminal::KeyInput;

const FLAVOR: &str = "com.superlogical.terminal.shell";

/// Which session a view shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// That session, by ID; a new one when it is gone.
    Id(String),
    /// The session with that label, created with it when there is none.
    Label(String),
    /// A new session, named by the server (such as `quiet-meadow`).
    New,
}

pub struct SessionOptions {
    pub socket: PathBuf,
    /// Which session to show.
    pub target: Target,
    /// The program a new session's first terminal runs, without a shell. Empty runs the
    /// account's login shell.
    pub command: Vec<String>,
    /// The program a new tab or split runs, the same way.
    pub pane_command: Vec<String>,
    pub cwd: Option<String>,
    pub cols: u16,
    pub rows: u16,
    pub client_name: String,
}

struct Pane {
    shared: Arc<Shared>,
    stop: Option<UnixStream>,
    size: Option<(u16, u16, u32, u32)>,
}

struct Inner {
    control: Control,
    socket: PathBuf,
    id: String,
    pane_command: Vec<String>,
    view: Mutex<SessionView>,
    panes: Mutex<HashMap<String, Pane>>,
    ended: AtomicBool,
    wake: Box<dyn Fn() + Send + Sync>,
}

/// Cheap to clone; every clone is the same session.
#[derive(Clone)]
pub struct Session(Arc<Inner>);

pub(crate) fn block_options(command: &[String], cwd: Option<&str>, size: Option<(u16, u16)>) -> Value {
    let mut options = json!({});
    if let Some((cols, rows)) = size {
        options["size"] = json!({"columns": cols.max(1), "rows": rows.max(1)});
    }
    if !command.is_empty() {
        options["shell"] = json!("none");
        options["command"] = json!(command);
    }
    if let Some(cwd) = cwd.filter(|cwd| !cwd.is_empty()) {
        options["cwd"] = json!(cwd);
    }
    options
}

impl Session {
    /// Opens the session `options.target` names, creating it when there is none, and starts
    /// streaming its panes. `wake` runs whenever something to draw changed.
    pub fn open(options: SessionOptions, wake: impl Fn() + Send + Sync + 'static) -> Result<Self> {
        let control = Control::connect(&options.socket, &options.client_name)?;
        let events = control.take_events().context("control events already taken")?;
        let mut id = match &options.target {
            Target::Id(id) => exists(&control, id)?.then(|| id.clone()),
            Target::Label(label) => find(&control, label)?,
            Target::New => None,
        };
        if let Some(existing) = &id {
            control.call("session.attach", json!({"session_id": existing}))?;
            let view = SessionView::parse(&control.call("session.view", json!({"session_id": existing}))?);
            if view.windows.is_empty() {
                // Its programs exited and left it empty: start it again.
                control.call("session.destroy", json!({"session_id": existing}))?;
                id = None;
            }
        }
        let id = match id {
            Some(id) => id,
            None => {
                let created = control.call(
                    "session.create",
                    create_payload(&options),
                )?;
                let id = created["session_id"].as_str().context("session.create gave no session_id")?.to_owned();
                control.call("session.attach", json!({"session_id": id}))?;
                id
            }
        };
        let session = Self(Arc::new(Inner {
            control,
            socket: options.socket,
            id,
            pane_command: options.pane_command,
            view: Mutex::new(SessionView::default()),
            panes: Mutex::new(HashMap::new()),
            ended: AtomicBool::new(false),
            wake: Box::new(wake),
        }));
        session.refresh()?;
        // The threads hold the session weakly: dropping the last `Session` closes the control
        // connection, which ends the event stream, and shuts the panes' streams.
        let weak = Arc::downgrade(&session.0);
        std::thread::Builder::new().name("rex-session".into()).spawn(move || {
            for event in events {
                let Some(inner) = weak.upgrade() else { return };
                let watcher = Session(inner);
                if event.session.as_deref().is_some_and(|session| session != watcher.0.id) {
                    continue;
                }
                let result = match event.name.as_str() {
                    // It names the session in its payload, and comes for every session this
                    // connection attached, such as an empty one replaced on open.
                    "session_destroyed" if event.payload["session_id"] == watcher.0.id.as_str() => {
                        watcher.end();
                        return;
                    }
                    "session_destroyed" => Ok(()),
                    "session_view_changed" => {
                        let newer = event.payload["revision"].as_u64().unwrap_or(u64::MAX)
                            > watcher.0.view.lock().unwrap().revision;
                        if newer { watcher.refresh() } else { Ok(()) }
                    }
                    "block_closed" => {
                        if let Some(block) = event.payload["block_id"].as_str() {
                            watcher.drop_pane(block);
                        }
                        Ok(())
                    }
                    _ => Ok(()),
                };
                match result {
                    Err(error) if format!("{error:#}").contains("session_not_found") => {
                        watcher.end();
                        return;
                    }
                    Err(error) => eprintln!("dojo-rex: following the session: {error:#}"),
                    Ok(()) => {}
                }
            }
            // The control connection closed.
            if let Some(inner) = weak.upgrade() {
                Session(inner).end();
            }
        })?;
        Ok(session)
    }

    pub fn id(&self) -> &str {
        &self.0.id
    }

    /// The session's label, which the server chose unless one was given.
    pub fn label(&self) -> String {
        self.0.view.lock().unwrap().label.clone()
    }

    pub fn view(&self) -> SessionView {
        self.0.view.lock().unwrap().clone()
    }

    /// The session was destroyed or lost its last tab.
    pub fn ended(&self) -> bool {
        self.0.ended.load(Ordering::SeqCst)
    }

    /// A pane's terminal mirror, once its stream has started.
    pub fn pane(&self, block: &str) -> Option<Arc<Shared>> {
        self.0.panes.lock().unwrap().get(block).map(|pane| pane.shared.clone())
    }

    fn end(&self) {
        self.0.ended.store(true, Ordering::SeqCst);
        (self.0.wake)();
    }

    /// Reads the view again and streams exactly the session's panes.
    fn refresh(&self) -> Result<()> {
        let view = SessionView::parse(&self.0.control.call("session.view", json!({"session_id": self.0.id}))?);
        // Every tab's panes stream, so switching tabs shows them at once. A stream closed and
        // reopened soon after is refused while the server still holds the old one.
        let wanted: HashSet<String> = view
            .windows
            .iter()
            .filter_map(|window| window.layout.as_ref())
            .flat_map(|layout| layout.blocks())
            .collect();
        let no_windows = view.windows.is_empty();
        *self.0.view.lock().unwrap() = view;
        if no_windows {
            // The last tab closed: nothing is left to show, so the empty session goes too.
            let _ = self.0.control.call("session.destroy", json!({"session_id": self.0.id}));
            self.end();
            return Ok(());
        }
        let current: Vec<String> = self.0.panes.lock().unwrap().keys().cloned().collect();
        for block in current.iter().filter(|block| !wanted.contains(*block)) {
            self.drop_pane(block);
        }
        for block in wanted.iter().filter(|block| !current.contains(block)) {
            if let Err(error) = self.stream(block) {
                eprintln!("dojo-rex: streaming {block}: {error:#}");
            }
        }
        (self.0.wake)();
        Ok(())
    }

    fn stream(&self, block: &str) -> Result<()> {
        let ticket = self.0.control.call(
            "data.connect",
            json!({"session_id": self.0.id, "block_id": block, "data_ref_id": "terminal", "subprotocol": data::SUBPROTOCOL}),
        )?;
        let ticket = data::decode_ticket(ticket["ticket"].as_str().context("data.connect gave no ticket")?)?;
        let stream = DataStream::open(&self.0.socket, &ticket)?;
        let stop = stream.shutdown_handle().ok();
        let shared = Arc::new(Shared { terminal: Mutex::new(None), ended: AtomicBool::new(false), error: Mutex::new(None) });
        self.0.panes.lock().unwrap().insert(block.to_owned(), Pane { shared: shared.clone(), stop, size: None });
        let session: Weak<Inner> = Arc::downgrade(&self.0);
        std::thread::Builder::new().name("rex-pane".into()).spawn(move || {
            let wake = || {
                if let Some(inner) = session.upgrade() {
                    (inner.wake)();
                }
            };
            if let Err(error) = run_stream(stream, &shared, &wake) {
                if !shared.ended.load(Ordering::SeqCst) {
                    *shared.error.lock().unwrap() = Some(format!("{error:#}"));
                }
            }
            shared.ended.store(true, Ordering::SeqCst);
            wake();
        })?;
        Ok(())
    }

    fn drop_pane(&self, block: &str) {
        if let Some(pane) = self.0.panes.lock().unwrap().remove(block) {
            pane.shared.ended.store(true, Ordering::SeqCst);
            if let Some(stop) = pane.stop {
                let _ = stop.shutdown(std::net::Shutdown::Both);
            }
        }
    }

    fn call(&self, method: &str, mut payload: Value) -> Result<Value> {
        payload["session_id"] = json!(self.0.id);
        self.0.control.call(method, payload)
    }

    fn block_call(&self, method: &str, block: &str, args: Value) -> Result<Value> {
        self.0.control.block_call(method, &self.0.id, block, args)
    }

    /// Types bytes into a pane without waiting for the server's answer.
    pub fn write(&self, block: &str, bytes: &[u8]) -> Result<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        let data = String::from_utf8_lossy(bytes);
        self.0.control.send(
            "com.superlogical.terminal.write",
            json!({"session_id": self.0.id, "block_id": block, "args": {"data": data}}),
        )
    }

    /// Encodes a key with the pane's current modes and types it.
    pub fn key(&self, block: &str, key: &KeyInput) -> Result<()> {
        let Some(shared) = self.pane(block) else { return Ok(()) };
        let bytes = match shared.terminal.lock().unwrap().as_mut() {
            Some(terminal) => terminal.encode_key(key),
            None => return Ok(()),
        };
        self.write(block, &bytes)
    }

    /// Claims the pane and sizes it to the view's rectangle; the same size again does nothing.
    pub fn resize(&self, block: &str, cols: u16, rows: u16, cell_width_px: u32, cell_height_px: u32) -> Result<()> {
        let size = (cols.max(1), rows.max(1), cell_width_px, cell_height_px);
        let first = {
            let mut panes = self.0.panes.lock().unwrap();
            let Some(pane) = panes.get_mut(block) else { return Ok(()) };
            if pane.size == Some(size) {
                return Ok(());
            }
            let first = pane.size.is_none();
            pane.size = Some(size);
            first
        };
        if first {
            self.call("session.own_block", json!({"block_ids": [block], "mode": "take"}))?;
        }
        let mut args = json!({"columns": size.0, "rows": size.1});
        if cell_width_px > 0 && cell_height_px > 0 {
            args["cell_width_px"] = json!(cell_width_px);
            args["cell_height_px"] = json!(cell_height_px);
        }
        self.block_call("com.superlogical.terminal.resize", block, args).map(drop)
    }

    /// Gives a pane new default colours (see `Terminal::apply_theme` for the shape).
    pub fn set_theme(&self, block: &str, theme: Value) -> Result<()> {
        self.block_call("com.superlogical.terminal.set_theme", block, theme).map(drop)
    }

    /// The directory the focused pane's shell last reported, so new panes start there.
    fn focused_directory(&self) -> Option<String> {
        let block = self.view().focused_block()?;
        let result = self.block_call("com.superlogical.terminal.pwd", &block, json!({})).ok()?;
        result["pwd"].as_str().map(crate::sessions::path_from_pwd)
    }

    fn new_block(&self) -> Value {
        json!({"block": {"flavor": FLAVOR, "options": block_options(&self.0.pane_command, self.focused_directory().as_deref(), None)}})
    }

    pub fn new_tab(&self) -> Result<()> {
        self.call("session.new_window", json!({"layout": self.new_block(), "focus": true})).map(drop)
    }

    /// Closes the active tab and everything in it.
    pub fn close_tab(&self) -> Result<()> {
        self.call("session.close_window", json!({})).map(drop)
    }

    pub fn focus_tab(&self, window: &str) -> Result<()> {
        self.call("session.focus_window", json!({"window_id": window})).map(drop)
    }

    pub fn next_tab(&self, forward: bool) -> Result<()> {
        let method = if forward { "session.focus_next_window" } else { "session.focus_previous_window" };
        self.call(method, json!({})).map(drop)
    }

    /// Splits the focused pane: `horizontal` puts the new one on the right, otherwise below.
    pub fn split(&self, horizontal: bool) -> Result<()> {
        let Some(block) = self.view().focused_block() else { return Ok(()) };
        self.call(
            "session.new_split",
            json!({"anchor_id": block, "direction": if horizontal { "horizontal" } else { "vertical" },
                "side": "after", "layout": self.new_block(), "focus": true}),
        )
        .map(drop)
    }

    pub fn focus_pane(&self, block: &str) -> Result<()> {
        self.call("session.focus_block", json!({"block_id": block})).map(drop)
    }

    /// `left`, `right`, `up` or `down`.
    pub fn focus_direction(&self, direction: &str) -> Result<()> {
        self.call("session.focus_direction", json!({"direction": direction})).map(drop)
    }

    pub fn toggle_zoom(&self) -> Result<()> {
        let Some(window) = self.view().active().map(|window| window.id.clone()) else { return Ok(()) };
        self.call("session.zoom_block", json!({"window_id": window})).map(drop)
    }

    /// Closes the focused pane, ending its program.
    pub fn close_pane(&self) -> Result<()> {
        let Some(block) = self.view().focused_block() else { return Ok(()) };
        self.call("block.close", json!({"block_id": block})).map(drop)
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        for (_, pane) in self.panes.lock().unwrap().drain() {
            if let Some(stop) = pane.stop {
                let _ = stop.shutdown(std::net::Shutdown::Both);
            }
        }
    }
}

/// A new session: labelled when the target names one, otherwise the server picks a name.
fn create_payload(options: &SessionOptions) -> Value {
    let mut payload = json!({"initial_windows": [{"layout": {"block": {"flavor": FLAVOR,
        "options": block_options(&options.command, options.cwd.as_deref(), Some((options.cols, options.rows)))}}}]});
    if let Target::Label(label) = &options.target {
        payload["label"] = json!(label);
    }
    payload
}

fn exists(control: &Control, id: &str) -> Result<bool> {
    let sessions = control.call("session.list", json!({}))?;
    Ok(sessions["sessions"].as_array().into_iter().flatten().any(|session| session["session_id"] == id))
}

fn find(control: &Control, label: &str) -> Result<Option<String>> {
    let sessions = control.call("session.list", json!({}))?;
    Ok(sessions["sessions"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|session| session["label"] == label)
        .and_then(|session| session["session_id"].as_str())
        .map(str::to_owned))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_session_is_named_by_the_server_unless_a_label_is_given() {
        let options = |target| SessionOptions {
            socket: PathBuf::new(),
            target,
            command: vec![],
            pane_command: vec![],
            cwd: None,
            cols: 80,
            rows: 24,
            client_name: "test".into(),
        };
        assert!(create_payload(&options(Target::New)).get("label").is_none());
        assert_eq!(create_payload(&options(Target::Label("work".into())))["label"], "work");
    }

    #[test]
    fn a_block_runs_its_command_without_a_shell_in_the_given_directory() {
        let options = block_options(&["rush".into()], Some("/tmp"), Some((90, 20)));
        assert_eq!(options, json!({"size": {"columns": 90, "rows": 20}, "shell": "none", "command": ["rush"], "cwd": "/tmp"}));
        assert_eq!(block_options(&[], Some(""), None), json!({}));
    }
}
