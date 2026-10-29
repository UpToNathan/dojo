//! What the terminal view talks to: a Rex session, or with `--demo` a local terminal that
//! shows sample output and echoes what is typed.
use dojo_rex::{Attachment, KeyInput, Screen, Terminal};
use std::sync::atomic::Ordering;

/// The configured default cursor shape (`settings`), set once at start.
pub static CURSOR: std::sync::OnceLock<dojo_rex::CursorStyle> = std::sync::OnceLock::new();

pub enum Backend {
    Rex(Attachment),
    Demo(Terminal),
}

/// Why the view should go away.
pub enum Ended {
    Clean,
    Failed(String),
}

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

    /// The screen if it changed since the last call.
    pub fn screen(&mut self, force: bool) -> Option<Screen> {
        let cursor = CURSOR.get().copied();
        let result = match self {
            Self::Rex(attachment) => {
                let mut guard = attachment.shared.terminal.lock().unwrap();
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

    pub fn key(&mut self, key: &KeyInput) {
        match self {
            Self::Rex(attachment) => {
                let _ = attachment.key(key);
            }
            Self::Demo(terminal) => {
                let bytes = terminal.encode_key(key);
                self.write(&echo(&bytes));
            }
        }
    }

    pub fn write(&mut self, bytes: &[u8]) {
        match self {
            Self::Rex(attachment) => {
                let _ = attachment.write(bytes);
            }
            Self::Demo(terminal) => terminal.write(bytes),
        }
    }

    pub fn resize(&mut self, cols: u16, rows: u16, cell_width_px: u32, cell_height_px: u32) {
        match self {
            Self::Rex(attachment) => {
                let _ = attachment.resize(cols, rows, cell_width_px, cell_height_px);
            }
            Self::Demo(terminal) => {
                let _ = terminal.resize(cols, rows, cell_width_px, cell_height_px);
            }
        }
    }

    pub fn ended(&self) -> Option<Ended> {
        match self {
            Self::Rex(attachment) if attachment.shared.ended.load(Ordering::SeqCst) => {
                Some(match attachment.shared.error.lock().unwrap().clone() {
                    Some(error) => Ended::Failed(error),
                    None => Ended::Clean,
                })
            }
            _ => None,
        }
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
