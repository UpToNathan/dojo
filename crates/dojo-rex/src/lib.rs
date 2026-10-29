//! A client for a Rex server: the control connection, terminal data connections, and the
//! libghostty-vt terminal that turns a data stream into a screen.
//!
//! The protocol is the one the server documents at `/llms.txt` on its socket (API revision 7).
//! Everything here is independent of any UI toolkit.

pub mod attach;
pub mod control;
pub mod data;
pub mod ffi;
pub mod keys;
pub mod layout;
pub mod recent;
pub mod session;
pub mod sessions;
pub mod stream;
pub mod terminal;
mod ws;

pub use attach::{Attachment, Options};
pub use control::{Control, Event};
pub use layout::{Layout, Rect, SessionView, WindowView};
pub use session::{Session, SessionOptions};
pub use sessions::{SessionSummary, Status, list_sessions};
pub use data::{DataStream, Welcome};
pub use stream::{Demux, Message, Piece};
pub use terminal::{Cell, Cursor, CursorStyle, KeyAction, KeyInput, Rgb, Screen, Terminal};

use std::path::PathBuf;

/// The personal server's socket: `$XDG_RUNTIME_DIR/rex/server.sock`.
pub fn default_socket() -> PathBuf {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", unsafe { libc_getuid() })));
    runtime.join("rex/server.sock")
}

unsafe extern "C" {
    #[link_name = "getuid"]
    fn libc_getuid() -> u32;
}
