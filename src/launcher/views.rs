//! Views a launcher command pushes: a search box, a list of rows and an action on Enter.
//!
//! A view is plain data plus logic. It never touches the window, so it can be tested alone; the
//! launcher draws its rows and carries out the [`Outcome`] of an activation.

mod clipboard;
mod color;
mod emoji;
mod loader;
mod processes;
mod services;
mod snippets;
mod windows;

use std::sync::Arc;

use dojo::launcher_core::commands::CommandId;
use dojo::launcher_core::rows::Row;

/// Tells the launcher that a view's data changed (a background load finished) so it asks the view
/// for its rows again. Safe to call from any thread.
pub type Notify = Arc<dyn Fn() + Send + Sync>;

/// What the launcher knows that a view may need when a row is activated.
#[derive(Clone, Debug, Default)]
pub struct Host {
    /// The text on the clipboard, if any.
    pub clipboard: Option<String>,
}

/// Keys a view can claim besides Enter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Secondary {
    /// Ctrl+P: pin or favourite the row.
    Pin,
    /// Ctrl+D: delete the row, or the stronger form of its action.
    Delete,
    /// Shift+Enter: the alternative to Enter (for example, show a unit's log).
    Alternate,
}

/// What the launcher does after a row of a view is activated.
pub enum Outcome {
    /// Keep the view open (the row changed state, or was refused with nothing to say).
    Stay,
    /// Close the launcher.
    Close,
    /// Copy the text to the clipboard and close.
    Copy(String),
    /// Show the message in the status line and keep the view open.
    Error(String),
    /// Say something in the status line without it being an error, and ask for the rows again.
    Notice(String),
    /// Close the launcher, then run the job on another thread. For work that needs the screen
    /// clear, such as picking a colour from it.
    After(Box<dyn FnOnce() + Send>),
}

impl std::fmt::Debug for Outcome {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stay => formatter.write_str("Stay"),
            Self::Close => formatter.write_str("Close"),
            Self::Copy(text) => formatter.debug_tuple("Copy").field(text).finish(),
            Self::Error(message) => formatter.debug_tuple("Error").field(message).finish(),
            Self::Notice(message) => formatter.debug_tuple("Notice").field(message).finish(),
            Self::After(_) => formatter.write_str("After(..)"),
        }
    }
}

/// Outcomes compare by what they say. A job to run afterwards equals nothing, itself included.
impl PartialEq for Outcome {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Stay, Self::Stay) | (Self::Close, Self::Close) => true,
            (Self::Copy(left), Self::Copy(right))
            | (Self::Error(left), Self::Error(right))
            | (Self::Notice(left), Self::Notice(right)) => left == right,
            _ => false,
        }
    }
}

pub trait View {
    /// Text shown in the empty search box.
    fn placeholder(&self) -> &str;

    /// Called once when the view is pushed. A view that loads slowly starts its work here and calls
    /// `notify` when it has something new.
    fn open(&mut self, notify: Notify) {
        let _ = notify;
    }

    /// The rows for `query`. Called on every change to the search and after a notification.
    fn rows(&mut self, query: &str) -> Vec<Row>;

    /// Enter (or a click) on `row`, one of the rows last returned.
    fn activate(&mut self, row: &Row, query: &str, host: &Host) -> Outcome;

    /// Tab. A view that has a mode to cycle (a skin tone) does it and returns the status text
    /// that says so; the rows are then requested again.
    fn tab(&mut self) -> Option<String> {
        None
    }

    /// A claimed secondary key on `row`. `None` means the view does not use that key.
    fn secondary(&mut self, key: Secondary, row: &Row, host: &Host) -> Option<Outcome> {
        let _ = (key, row, host);
        None
    }

    /// Key hints appended to the status line, such as "Ctrl+P pin".
    fn hint(&self) -> &str {
        ""
    }

    /// A message the view wants in the status line after its rows were requested, such as the
    /// result of work it did in the background. Taken, so it shows once.
    fn status(&mut self) -> Option<String> {
        None
    }

    /// Shown in the status line when `rows` is empty.
    fn empty_text(&self, query: &str) -> String {
        if query.is_empty() {
            "Nothing here yet".into()
        } else {
            "No results found".into()
        }
    }
}

/// Builds the view for a command, or `None` when that command has no view yet.
pub type ViewFactory = Arc<dyn Fn(CommandId) -> Option<Box<dyn View>> + Send + Sync>;

/// The views Dojo ships. A command joins [`dojo::launcher_core::commands::IMPLEMENTED`] in the
/// same change that adds its arm here.
pub fn default_factory() -> ViewFactory {
    Arc::new(|command| match command {
        CommandId::Emoji => Some(Box::new(emoji::EmojiView::new())),
        CommandId::Snippets => Some(Box::new(snippets::SnippetsView::new())),
        CommandId::ClipboardHistory => Some(Box::new(clipboard::ClipboardView::new())),
        CommandId::Windows => Some(Box::new(windows::WindowsView::new())),
        CommandId::PickColor => Some(Box::new(color::ColorView::new())),
        CommandId::Processes => Some(Box::new(processes::ProcessesView::new())),
        CommandId::Services => Some(Box::new(services::ServicesView::new())),
    })
}
