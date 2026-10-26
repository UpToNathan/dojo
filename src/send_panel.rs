//! The send panel's state: which window, which peer, whether the cursor and keyboard go along,
//! and what happened. Plain data and logic; `send_window_ui` draws it and runs the send.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowChoice {
    pub id: u64,
    pub app_id: String,
    pub title: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum List {
    Windows,
    Peers,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    /// The windows and peers are still being read.
    Loading,
    Choosing,
    Sending,
    /// The peer lacks these files; Enter copies them and sends again.
    Missing(Vec<String>),
    Failed(String),
    /// Sent. What closes here is still pending for a moment, so it can be kept.
    Sent { peer: String, sent: String, notes: Vec<String>, source: Source, take_input: bool },
}

/// What happens to the original after a send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// Closes soon; Esc keeps it. `closes` is what `dojo-mesh send --json` said closes, as JSON.
    Pending { window: u64, closes: String },
    Kept,
}

/// What `dojo-mesh send --dry-run` said a window would send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preview {
    pub sent: String,
    pub notes: Vec<String>,
    /// The window could be sent whole (a browser window the extension reports): W switches to that.
    pub can_send_window: bool,
    pub tab_count: Option<usize>,
}

/// What the panel asks its owner to do after a key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    Nothing,
    Close,
    Send { window: u64, peer: String, take_input: bool, copy_missing: bool, all_tabs: bool },
    /// The send is over: close the original (unless it is kept), then the panel, and then start
    /// taking the cursor and keyboard to the peer if that was asked for.
    Finish { close: Option<(u64, String)>, remote_to: Option<String> },
    /// No window: only the cursor and keyboard go to the peer.
    RemoteOnly { peer: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Model {
    pub windows: Vec<WindowChoice>,
    pub peers: Vec<String>,
    pub window: usize,
    pub peer: usize,
    pub list: List,
    pub take_input: bool,
    pub phase: Phase,
    /// Send every tab of a browser window, not just the one in front.
    pub all_tabs: bool,
    /// Send no window: only take the cursor and keyboard to the peer.
    pub input_only: bool,
    /// What each window would send, by window and whether all its tabs, as far as it has been asked.
    pub previews: HashMap<(u64, bool), Result<Preview, String>>,
}

/// What the panel remembers between uses.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Memory {
    pub peer: Option<String>,
    #[serde(default)]
    pub take_input: bool,
    /// The panel was last left in control-only mode (no window sent).
    #[serde(default)]
    pub input_only: bool,
}

impl Memory {
    pub fn path(state_home: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
        let base = state_home.filter(|path| path.is_absolute()).or_else(|| home.map(|home| home.join(".local/state")))?;
        Some(base.join("dojo/send-window.json"))
    }

    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_string(self).map_err(std::io::Error::other)?)
    }
}

impl Model {
    pub fn loading(memory: &Memory) -> Self {
        Self { windows: Vec::new(), peers: Vec::new(), window: 0, peer: 0, list: List::Windows, take_input: memory.take_input, phase: Phase::Loading, all_tabs: false, input_only: memory.input_only, previews: HashMap::new() }
    }

    /// The windows (focused first is selected) and online peers (the remembered one selected).
    pub fn loaded(&mut self, windows: Vec<WindowChoice>, focused: Option<u64>, peers: Vec<String>, memory: &Memory) {
        self.window = focused.and_then(|id| windows.iter().position(|window| window.id == id)).unwrap_or(0);
        self.peer = memory.peer.as_ref().and_then(|name| peers.iter().position(|peer| peer == name)).unwrap_or(0);
        self.windows = windows;
        self.peers = peers;
        self.phase = Phase::Choosing;
        if self.input_only {
            self.list = List::Peers;
        }
    }

    pub fn failed(&mut self, message: String) {
        self.phase = Phase::Failed(message);
    }

    pub fn missing(&mut self, files: Vec<String>) {
        self.phase = Phase::Missing(files);
    }

    /// `closes` is the close rule `dojo-mesh send --json` reported, as JSON text.
    pub fn sent(&mut self, peer: String, sent: String, notes: Vec<String>, window: u64, closes: String, take_input: bool) {
        self.phase = Phase::Sent { peer, sent, notes, source: Source::Pending { window, closes }, take_input };
    }

    /// What ends the Sent phase now. Esc keeps the original; the cursor and keyboard still go if asked.
    pub fn finish(&mut self, keep: bool) -> Request {
        let Phase::Sent { peer, source, take_input, .. } = &mut self.phase else { return Request::Close };
        let remote_to = take_input.then(|| peer.clone());
        let close = match (keep, source.clone()) {
            (false, Source::Pending { window, closes }) => Some((window, closes)),
            _ => None,
        };
        if keep {
            *source = Source::Kept;
        }
        Request::Finish { close, remote_to }
    }

    /// The window selected, if any.
    pub fn selected_window(&self) -> Option<u64> {
        self.windows.get(self.window).map(|window| window.id)
    }

    /// The selected window, when what it would send has not been asked yet.
    pub fn needs_preview(&self) -> Option<(u64, bool)> {
        self.selected_window().map(|id| (id, self.all_tabs)).filter(|key| !self.previews.contains_key(key))
    }

    pub fn preview(&self) -> Option<&Result<Preview, String>> {
        self.selected_window().and_then(|id| self.previews.get(&(id, self.all_tabs)))
    }

    pub fn set_preview(&mut self, window: u64, all_tabs: bool, preview: Result<Preview, String>) {
        self.previews.insert((window, all_tabs), preview);
    }

    /// Why Enter does nothing right now, if it does nothing.
    pub fn blocked(&self) -> Option<&'static str> {
        match self.phase {
            Phase::Loading => Some("Reading windows and machines…"),
            Phase::Sending => Some("Sending…"),
            _ if self.windows.is_empty() && !self.input_only => Some("There are no windows to send"),
            _ if self.peers.is_empty() => Some("No other machine is online"),
            _ => None,
        }
    }

    pub fn memory(&self) -> Memory {
        Memory { peer: self.peers.get(self.peer).cloned(), take_input: self.take_input, input_only: self.input_only }
    }

    /// Selects a row by click.
    pub fn select(&mut self, list: List, index: usize) {
        self.list = list;
        match list {
            List::Windows if index < self.windows.len() => self.window = index,
            List::Peers if index < self.peers.len() => self.peer = index,
            _ => {}
        }
        if matches!(self.phase, Phase::Failed(_) | Phase::Missing(_)) {
            self.phase = Phase::Choosing;
        }
    }

    /// A key, by its GPUI name.
    pub fn key(&mut self, key: &str) -> Request {
        if self.phase == Phase::Sending {
            return Request::Nothing;
        }
        if matches!(self.phase, Phase::Sent { .. }) {
            // Esc keeps the original; anything else closes it now.
            return self.finish(key == "escape");
        }
        match key {
            "escape" => {
                if matches!(self.phase, Phase::Missing(_) | Phase::Failed(_)) {
                    self.phase = Phase::Choosing;
                    return Request::Nothing;
                }
                Request::Close
            }
            // With no window in play there is only one list to be in.
            "tab" => {
                if !self.input_only {
                    self.list = if self.list == List::Windows { List::Peers } else { List::Windows };
                }
                Request::Nothing
            }
            // The machines are laid out side by side, so the sideways arrows move among them.
            "left" | "right" => {
                let length = self.peers.len();
                if length > 0 {
                    self.peer = if key == "left" { self.peer.checked_sub(1).unwrap_or(length - 1) } else { (self.peer + 1) % length };
                }
                if matches!(self.phase, Phase::Failed(_) | Phase::Missing(_)) {
                    self.phase = Phase::Choosing;
                }
                Request::Nothing
            }
            "up" | "down" => {
                let (index, length) = match self.list {
                    List::Windows => (&mut self.window, self.windows.len()),
                    List::Peers => (&mut self.peer, self.peers.len()),
                };
                if length > 0 {
                    *index = if key == "up" { index.checked_sub(1).unwrap_or(length - 1) } else { (*index + 1) % length };
                }
                if matches!(self.phase, Phase::Failed(_) | Phase::Missing(_)) {
                    self.phase = Phase::Choosing;
                }
                Request::Nothing
            }
            "i" => {
                // Just the cursor and keyboard, no window.
                self.set_control_only(!self.input_only);
                Request::Nothing
            }
            "w" => {
                // All tabs or the front one. Whether the window supports it is the preview's to say.
                self.all_tabs = !self.all_tabs;
                if matches!(self.phase, Phase::Failed(_) | Phase::Missing(_)) {
                    self.phase = Phase::Choosing;
                }
                Request::Nothing
            }
            "space" => {
                self.take_input = !self.take_input;
                Request::Nothing
            }
            "enter" => self.enter(),
            _ => Request::Nothing,
        }
    }

    /// Switches between sending a window and taking only the cursor and keyboard. In the second there is
    /// no window to choose, so the machines are the only list.
    pub fn set_control_only(&mut self, on: bool) {
        self.input_only = on;
        if on {
            self.list = List::Peers;
        }
        if matches!(self.phase, Phase::Failed(_) | Phase::Missing(_)) {
            self.phase = Phase::Choosing;
        }
    }

    pub fn enter(&mut self) -> Request {
        if self.blocked().is_some() {
            return Request::Nothing;
        }
        if self.input_only {
            return Request::RemoteOnly { peer: self.peers[self.peer].clone() };
        }
        let copy_missing = matches!(self.phase, Phase::Missing(_));
        self.phase = Phase::Sending;
        Request::Send { window: self.windows[self.window].id, peer: self.peers[self.peer].clone(), take_input: self.take_input, copy_missing, all_tabs: self.all_tabs }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn windows() -> Vec<WindowChoice> {
        vec![
            WindowChoice { id: 3, app_id: "helium".into(), title: "Page".into() },
            WindowChoice { id: 9, app_id: "kitty".into(), title: "~/w".into() },
        ]
    }

    fn ready(memory: &Memory) -> Model {
        let mut model = Model::loading(memory);
        model.loaded(windows(), Some(9), vec!["alpha".into(), "beta".into()], memory);
        model
    }

    #[test]
    fn opens_on_the_focused_window_and_the_remembered_peer() {
        let model = ready(&Memory { peer: Some("beta".into()), take_input: true, ..Default::default() });
        assert_eq!((model.window, model.peer, model.take_input), (1, 1, true));
        let fresh = ready(&Memory::default());
        assert_eq!((fresh.window, fresh.peer, fresh.take_input), (1, 0, false));
    }

    #[test]
    fn enter_sends_the_chosen_window_to_the_chosen_peer() {
        let mut model = ready(&Memory::default());
        model.key("tab");
        model.key("down");
        model.key("space");
        assert_eq!(model.key("enter"), Request::Send { window: 9, peer: "beta".into(), take_input: true, copy_missing: false, all_tabs: false });
        assert_eq!(model.phase, Phase::Sending);
        assert_eq!(model.key("enter"), Request::Nothing, "one send at a time");
        assert_eq!(model.memory(), Memory { peer: Some("beta".into()), take_input: true, ..Default::default() });
    }

    #[test]
    fn missing_files_turn_enter_into_copy_and_send_and_escape_backs_out() {
        let mut model = ready(&Memory::default());
        model.key("enter");
        model.missing(vec!["/w/a.rs".into()]);
        assert_eq!(model.key("enter"), Request::Send { window: 9, peer: "alpha".into(), take_input: false, copy_missing: true, all_tabs: false });
        model.missing(vec!["/w/a.rs".into()]);
        assert_eq!(model.key("escape"), Request::Nothing);
        assert_eq!(model.phase, Phase::Choosing);
        assert_eq!(model.key("escape"), Request::Close);
    }

    #[test]
    fn arrows_wrap_within_the_current_list() {
        let mut model = ready(&Memory::default());
        model.key("down");
        assert_eq!(model.window, 0);
        model.key("up");
        assert_eq!(model.window, 1);
        model.key("tab");
        model.key("up");
        assert_eq!((model.list, model.peer), (List::Peers, 1));
    }

    #[test]
    fn the_sideways_arrows_move_among_the_machines_wherever_the_focus_is() {
        let mut model = ready(&Memory::default());
        assert_eq!((model.list, model.peer), (List::Windows, 0));
        model.key("right");
        assert_eq!((model.list, model.peer), (List::Windows, 1), "the window list keeps the focus");
        model.key("right");
        assert_eq!(model.peer, 0, "wraps");
        model.key("left");
        assert_eq!(model.peer, 1);
    }

    #[test]
    fn control_only_leaves_the_machines_as_the_one_list_and_is_remembered() {
        let mut model = ready(&Memory::default());
        model.set_control_only(true);
        assert!(model.input_only);
        assert_eq!(model.list, List::Peers);
        model.key("tab");
        assert_eq!(model.list, List::Peers, "there is no window list to tab to");
        model.key("down");
        assert_eq!(model.peer, 1, "up and down move among the machines too");
        assert_eq!(model.memory().input_only, true);
        // Opening the panel again starts in the mode it was left in, with the machines focused.
        let mut again = Model::loading(&model.memory());
        again.loaded(windows(), None, vec!["alpha".into(), "beta".into()], &model.memory());
        assert!(again.input_only);
        assert_eq!((again.list, again.peer), (List::Peers, 1));
        again.set_control_only(false);
        assert!(!again.input_only);
    }

    #[test]
    fn nothing_is_sent_without_a_window_or_a_peer() {
        let mut lonely = Model::loading(&Memory::default());
        assert_eq!(lonely.key("enter"), Request::Nothing, "still loading");
        lonely.loaded(windows(), None, vec![], &Memory::default());
        assert_eq!(lonely.blocked(), Some("No other machine is online"));
        assert_eq!(lonely.key("enter"), Request::Nothing);
        let mut empty = Model::loading(&Memory::default());
        empty.loaded(vec![], None, vec!["alpha".into()], &Memory::default());
        assert_eq!(empty.blocked(), Some("There are no windows to send"));
    }

    #[test]
    fn after_sending_the_original_closes_unless_escape_keeps_it() {
        let mut model = ready(&Memory::default());
        model.key("enter");
        model.sent("alpha".into(), "the page https://x".into(), vec![], 9, r#"{"scope":"tab","title":"Page - Helium"}"#.into(), false);
        assert_eq!(model.finish(false), Request::Finish { close: Some((9, r#"{"scope":"tab","title":"Page - Helium"}"#.into())), remote_to: None });
        assert_eq!(model.key("escape"), Request::Finish { close: None, remote_to: None }, "Esc leaves the original alone");
        match &model.phase {
            Phase::Sent { source, .. } => assert_eq!(*source, Source::Kept),
            other => panic!("{other:?}"),
        }
        assert_eq!(model.key("enter"), Request::Finish { close: None, remote_to: None });
    }

    #[test]
    fn taking_the_cursor_and_keyboard_starts_after_the_send_whatever_happens_to_the_original() {
        let mut model = ready(&Memory::default());
        model.key("enter");
        model.sent("alpha".into(), "a terminal in /w".into(), vec![], 9, r#"{"scope":"window"}"#.into(), true);
        assert_eq!(model.key("enter"), Request::Finish { close: Some((9, r#"{"scope":"window"}"#.into())), remote_to: Some("alpha".into()) });
        let mut kept = ready(&Memory::default());
        kept.key("enter");
        kept.sent("alpha".into(), "x".into(), vec![], 9, r#"{"scope":"window"}"#.into(), true);
        assert_eq!(kept.key("escape"), Request::Finish { close: None, remote_to: Some("alpha".into()) });
    }

    #[test]
    fn each_window_is_previewed_once_per_scope_when_selected() {
        let mut model = ready(&Memory::default());
        let plain = |sent: &str| Preview { sent: sent.into(), notes: vec![], can_send_window: false, tab_count: None };
        assert_eq!(model.needs_preview(), Some((9, false)));
        model.set_preview(9, false, Ok(Preview { can_send_window: true, tab_count: Some(4), ..plain("the page https://x") }));
        assert_eq!(model.needs_preview(), None);
        assert!(matches!(model.preview(), Some(Ok(preview)) if preview.can_send_window));
        model.key("w");
        assert!(model.all_tabs);
        assert_eq!(model.needs_preview(), Some((9, true)), "all tabs is previewed separately");
        model.set_preview(9, true, Ok(plain("a browser window with 4 tabs")));
        model.key("w");
        assert_eq!(model.needs_preview(), None, "going back needs no new question");
        model.key("down");
        assert_eq!(model.needs_preview(), Some((3, false)));
        model.set_preview(3, false, Err("no desktop entry".into()));
        assert_eq!(model.preview(), Some(&Err("no desktop entry".to_string())));
    }

    #[test]
    fn i_takes_only_the_cursor_and_keyboard_and_needs_no_window() {
        let mut model = ready(&Memory { peer: Some("beta".into()), take_input: false, ..Default::default() });
        model.key("i");
        assert!(model.input_only);
        assert_eq!(model.key("enter"), Request::RemoteOnly { peer: "beta".into() });
        assert_eq!(model.phase, Phase::Choosing, "nothing was sent, so the panel is not mid-send");
        model.key("i");
        assert_eq!(model.key("enter"), Request::Send { window: 9, peer: "beta".into(), take_input: false, copy_missing: false, all_tabs: false });

        let mut windowless = Model::loading(&Memory::default());
        windowless.loaded(vec![], None, vec!["alpha".into()], &Memory::default());
        assert_eq!(windowless.blocked(), Some("There are no windows to send"));
        windowless.key("i");
        assert_eq!(windowless.blocked(), None, "with no window to send the cursor and keyboard can still go");
        assert_eq!(windowless.key("enter"), Request::RemoteOnly { peer: "alpha".into() });

        let mut alone = Model::loading(&Memory::default());
        alone.loaded(windows(), None, vec![], &Memory::default());
        alone.key("i");
        assert_eq!(alone.key("enter"), Request::Nothing, "still needs a machine");
    }

    #[test]
    fn w_sends_all_tabs() {
        let mut model = ready(&Memory::default());
        model.key("w");
        assert_eq!(model.key("enter"), Request::Send { window: 9, peer: "alpha".into(), take_input: false, copy_missing: false, all_tabs: true });
    }

    #[test]
    fn a_failure_is_shown_until_the_next_choice() {
        let mut model = ready(&Memory::default());
        model.key("enter");
        model.failed("the screen is locked".into());
        assert_eq!(model.phase, Phase::Failed("the screen is locked".into()));
        model.select(List::Peers, 1);
        assert_eq!(model.phase, Phase::Choosing);
    }

    #[test]
    fn memory_survives_a_round_trip_and_a_missing_file_is_empty() {
        let dir = std::env::temp_dir().join(format!("dojo-send-memory-{}", std::process::id()));
        let path = dir.join("send-window.json");
        assert_eq!(Memory::load(&path), Memory::default());
        let memory = Memory { peer: Some("beta".into()), take_input: true, ..Default::default() };
        memory.save(&path).unwrap();
        assert_eq!(Memory::load(&path), memory);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(Memory::path(None, Some("/h".into())), Some(PathBuf::from("/h/.local/state/dojo/send-window.json")));
    }
}
