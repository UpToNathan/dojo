//! Views a launcher command pushes: a search box, a list of rows and an action on Enter.
//!
//! A view is plain data plus logic. It never touches the window, so it can be tested alone; the
//! launcher draws its rows and carries out the [`Outcome`] of an activation.

mod emoji;
mod snippets;

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

/// What the launcher does after a row of a view is activated.
// No view ships with the foundation; each command's view constructs the variants it needs.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, PartialEq, Eq)]
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
        _ => None,
    })
}
