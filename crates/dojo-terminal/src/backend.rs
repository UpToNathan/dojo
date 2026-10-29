//! What the terminal view talks to: a Rex session, or with `--demo` one fake pane that
//! shows sample output and echoes what is typed.
use dojo_rex::{KeyInput, Layout, Screen, Session, SessionView, Terminal, WindowView};
use serde_json::Value;

use crate::bindings::Action;

/// The configured default cursor shape (`settings`), set once at start.
pub static CURSOR: std::sync::OnceLock<dojo_rex::CursorStyle> = std::sync::OnceLock::new();

pub enum Backend {
    Rex(Session),
    Demo(Terminal),
}

const DEMO_BLOCK: &str = "demo";

const SAMPLE: &str = "\x1b[1;32mdojo-terminal\x1b[0m demo (no Rex session)\r\n\
\x1b[31mred\x1b[0m \x1b[32mgreen\x1b[0m \x1b[33myellow\x1b[0m \x1b[34mblue\x1b[0m \x1b[35mmagenta\x1b[0m \x1b[36mcyan\x1b[0m\r\n\
\x1b[1mbold\x1b[0m \x1b[3mitalic\x1b[0m \x1b[2mfaint\x1b[0m \x1b[4munderline\x1b[0m \x1b[9mstrike\x1b[0m \x1b[7mreverse\x1b[0m\r\n\
\x1b[41m red bg \x1b[0m\x1b[42m green bg \x1b[0m\x1b[44m blue bg \x1b[0m \x1b[38;2;255;160;40mtruecolor\x1b[0m\r\n\
wide: \u{6f22}\u{5b57}\u{304b}\u{306a} after\r\n\
0123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789\r\n\
type here: ";

impl Backend {
    pub fn demo(cols: u16, rows: u16) -> anyhow::Result<Self> {
        let mut terminal = Terminal::new(cols, rows)?;
        terminal.write(SAMPLE.as_bytes());
        Ok(Self::Demo(terminal))
    }

    pub fn view(&self) -> SessionView {
        match self {
            Self::Rex(session) => session.view(),
            Self::Demo(_) => SessionView {
                active_window: Some("demo".into()),
                windows: vec![WindowView {
                    id: "demo".into(),
                    label: "Window demo".into(),
                    focused_block: Some(DEMO_BLOCK.into()),
                    zoomed_block: None,
                    layout: Some(Layout::Block(DEMO_BLOCK.into())),
                }],
                ..SessionView::default()
            },
        }
    }

    /// The pane's screen if it changed since the last call.
    /// How long until an animated image in the pane shows its next frame.
    pub fn next_frame_in(&self, block: &str) -> Option<std::time::Duration> {
        match self {
            Self::Rex(session) => session.pane(block)?.terminal.lock().unwrap().as_ref()?.next_frame_in(),
            Self::Demo(terminal) => terminal.next_frame_in(),
        }
    }

    pub fn screen(&mut self, block: &str, force: bool) -> Option<Screen> {
        let cursor = CURSOR.get().copied();
        let result = match self {
            Self::Rex(session) => {
                let pane = session.pane(block)?;
                let mut guard = pane.terminal.lock().unwrap();
                let terminal = guard.as_mut()?;
                terminal.set_default_cursor(cursor);
                terminal.screen(force)
            }
            Self::Demo(terminal) => {
                terminal.set_default_cursor(cursor);
                terminal.screen(force)
            }
        };
        result.ok().flatten()
    }

    pub fn key(&mut self, block: &str, key: &KeyInput) {
        match self {
            Self::Rex(session) => {
                let _ = session.key(block, key);
            }
            Self::Demo(terminal) => {
                let bytes = terminal.encode_key(key);
                terminal.write(&echo(&bytes));
            }
        }
    }

    pub fn write(&mut self, block: &str, bytes: &[u8]) {
        match self {
            Self::Rex(session) => {
                let _ = session.write(block, bytes);
            }
            Self::Demo(terminal) => terminal.write(bytes),
        }
    }

    pub fn resize(&mut self, block: &str, cols: u16, rows: u16, cell_width_px: u32, cell_height_px: u32) {
        match self {
            Self::Rex(session) => {
                let _ = session.resize(block, cols, rows, cell_width_px, cell_height_px);
            }
            Self::Demo(terminal) => {
                let _ = terminal.resize(cols, rows, cell_width_px, cell_height_px);
            }
        }
    }

    pub fn set_theme(&self, block: &str, theme: Value) {
        if let Self::Rex(session) = self {
            if let Err(error) = session.set_theme(block, theme) {
                eprintln!("dojo-terminal: applying the theme: {error:#}");
            }
        }
    }

    pub fn focus_tab(&self, window: &str) {
        self.report(|session| session.focus_tab(window));
    }

    pub fn focus_pane(&self, block: &str) {
        self.report(|session| session.focus_pane(block));
    }

    /// Runs a binding that is the session's to carry out.
    pub fn act(&self, action: Action) {
        self.report(|session| match action {
            Action::NewTab => session.new_tab(),
            Action::CloseTab => session.close_tab(),
            Action::NextTab => session.next_tab(true),
            Action::PreviousTab => session.next_tab(false),
            Action::GoToTab(number) => match session.view().windows.get(number - 1) {
                Some(window) => session.focus_tab(&window.id.clone()),
                None => Ok(()),
            },
            Action::SplitRight => session.split(true),
            Action::SplitDown => session.split(false),
            Action::Focus(direction) => session.focus_direction(direction.name()),
            Action::Zoom => session.toggle_zoom(),
            Action::ClosePane => session.close_pane(),
            Action::Picker | Action::Paste => Ok(()),
        });
    }

    fn report(&self, call: impl FnOnce(&Session) -> anyhow::Result<()>) {
        if let Self::Rex(session) = self {
            if let Err(error) = call(session) {
                eprintln!("dojo-terminal: {error:#}");
            }
        }
    }

    /// The session is gone: destroyed, or its last tab closed.
    pub fn ended(&self) -> Option<()> {
        matches!(self, Self::Rex(session) if session.ended()).then_some(())
    }
}

/// What a demo terminal shows for typed bytes: Enter starts a new line.
fn echo(bytes: &[u8]) -> Vec<u8> {
    bytes.iter().flat_map(|&b| if b == b'\r' { vec![b'\r', b'\n'] } else { vec![b] }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enter_echoes_as_a_new_line() {
        assert_eq!(echo(b"a\rb"), b"a\r\nb");
    }
}
