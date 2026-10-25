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
    Sent { peer: String, sent: String, notes: Vec<String>, source: Source },
}

/// What happens to the original after a send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// Closes soon; Esc keeps it.
    Pending { window: u64, tab_title: Option<String> },
    Kept,
}

/// What `dojo-mesh send --dry-run` said a window would send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preview {
    pub sent: String,
    pub notes: Vec<String>,
}

/// What the panel asks its owner to do after a key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    Nothing,
    Close,
    Send { window: u64, peer: String, take_input: bool, copy_missing: bool },
    /// Close the original now, then the panel.
    CloseSource { window: u64, tab_title: Option<String> },
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
    /// What each window would send, as far as it has been asked.
    pub previews: HashMap<u64, Result<Preview, String>>,
}

/// What the panel remembers between uses.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Memory {
    pub peer: Option<String>,
    #[serde(default)]
    pub take_input: bool,
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
        Self { windows: Vec::new(), peers: Vec::new(), window: 0, peer: 0, list: List::Windows, take_input: memory.take_input, phase: Phase::Loading, previews: HashMap::new() }
    }

    /// The windows (focused first is selected) and online peers (the remembered one selected).
    pub fn loaded(&mut self, windows: Vec<WindowChoice>, focused: Option<u64>, peers: Vec<String>, memory: &Memory) {
        self.window = focused.and_then(|id| windows.iter().position(|window| window.id == id)).unwrap_or(0);
        self.peer = memory.peer.as_ref().and_then(|name| peers.iter().position(|peer| peer == name)).unwrap_or(0);
        self.windows = windows;
        self.peers = peers;
        self.phase = Phase::Choosing;
    }

    pub fn failed(&mut self, message: String) {
        self.phase = Phase::Failed(message);
    }

    pub fn missing(&mut self, files: Vec<String>) {
        self.phase = Phase::Missing(files);
    }

    /// `closes_tab` is the window's title when only a tab is to close, `None` for the whole window.
    pub fn sent(&mut self, peer: String, sent: String, notes: Vec<String>, window: u64, tab_title: Option<String>) {
        self.phase = Phase::Sent { peer, sent, notes, source: Source::Pending { window, tab_title } };
    }

    /// The window selected, if any.
    pub fn selected_window(&self) -> Option<u64> {
        self.windows.get(self.window).map(|window| window.id)
    }

    /// The selected window, when what it would send has not been asked yet.
    pub fn needs_preview(&self) -> Option<u64> {
        self.selected_window().filter(|id| !self.previews.contains_key(id))
    }

    pub fn preview(&self) -> Option<&Result<Preview, String>> {
        self.selected_window().and_then(|id| self.previews.get(&id))
    }

    pub fn set_preview(&mut self, window: u64, preview: Result<Preview, String>) {
        self.previews.insert(window, preview);
    }

    /// Why Enter does nothing right now, if it does nothing.
    pub fn blocked(&self) -> Option<&'static str> {
        match self.phase {
            Phase::Loading => Some("Reading windows and machines…"),
            Phase::Sending => Some("Sending…"),
            _ if self.windows.is_empty() => Some("There are no windows to send"),
            _ if self.peers.is_empty() => Some("No other machine is online"),
            _ => None,
        }
    }

    pub fn memory(&self) -> Memory {
        Memory { peer: self.peers.get(self.peer).cloned(), take_input: self.take_input }
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
        if let Phase::Sent { source, .. } = &mut self.phase {
            return match (key, source.clone()) {
                // Esc keeps the original; anything else closes it now.
                ("escape", Source::Pending { .. }) => {
                    *source = Source::Kept;
                    Request::Close
                }
                (_, Source::Pending { window, tab_title }) => Request::CloseSource { window, tab_title },
                (_, Source::Kept) => Request::Close,
            };
        }
        match key {
            "escape" => {
                if matches!(self.phase, Phase::Missing(_) | Phase::Failed(_)) {
                    self.phase = Phase::Choosing;
                    return Request::Nothing;
                }
                Request::Close
            }
            "tab" | "left" | "right" => {
                self.list = if self.list == List::Windows { List::Peers } else { List::Windows };
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
            "space" => {
                self.take_input = !self.take_input;
                Request::Nothing
            }
            "enter" => self.enter(),
            _ => Request::Nothing,
        }
    }

    pub fn enter(&mut self) -> Request {
        if self.blocked().is_some() {
            return Request::Nothing;
        }
        let copy_missing = matches!(self.phase, Phase::Missing(_));
        self.phase = Phase::Sending;
        Request::Send { window: self.windows[self.window].id, peer: self.peers[self.peer].clone(), take_input: self.take_input, copy_missing }
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
        let model = ready(&Memory { peer: Some("beta".into()), take_input: true });
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
        assert_eq!(model.key("enter"), Request::Send { window: 9, peer: "beta".into(), take_input: true, copy_missing: false });
        assert_eq!(model.phase, Phase::Sending);
        assert_eq!(model.key("enter"), Request::Nothing, "one send at a time");
        assert_eq!(model.memory(), Memory { peer: Some("beta".into()), take_input: true });
    }

    #[test]
    fn missing_files_turn_enter_into_copy_and_send_and_escape_backs_out() {
        let mut model = ready(&Memory::default());
        model.key("enter");
        model.missing(vec!["/w/a.rs".into()]);
        assert_eq!(model.key("enter"), Request::Send { window: 9, peer: "alpha".into(), take_input: false, copy_missing: true });
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
        model.key("right");
        model.key("up");
        assert_eq!((model.list, model.peer), (List::Peers, 1));
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
        model.sent("alpha".into(), "the page https://x".into(), vec![], 9, Some("Page - Helium".into()));
        assert_eq!(model.key("enter"), Request::CloseSource { window: 9, tab_title: Some("Page - Helium".into()) });
        assert_eq!(model.key("escape"), Request::Close, "Esc closes the panel and leaves the original alone");
        match &model.phase {
            Phase::Sent { source, .. } => assert_eq!(*source, Source::Kept),
            other => panic!("{other:?}"),
        }
        assert_eq!(model.key("enter"), Request::Close);
    }

    #[test]
    fn each_window_is_previewed_once_when_selected() {
        let mut model = ready(&Memory::default());
        assert_eq!(model.needs_preview(), Some(9));
        model.set_preview(9, Ok(Preview { sent: "a terminal in /w".into(), notes: vec![] }));
        assert_eq!(model.needs_preview(), None);
        assert!(matches!(model.preview(), Some(Ok(_))));
        model.key("down");
        assert_eq!(model.needs_preview(), Some(3));
        model.set_preview(3, Err("no desktop entry".into()));
        assert_eq!(model.preview(), Some(&Err("no desktop entry".to_string())));
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
        let memory = Memory { peer: Some("beta".into()), take_input: true };
        memory.save(&path).unwrap();
        assert_eq!(Memory::load(&path), memory);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(Memory::path(None, Some("/h".into())), Some(PathBuf::from("/h/.local/state/dojo/send-window.json")));
    }
}
