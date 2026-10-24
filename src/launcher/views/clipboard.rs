//! The Clipboard History command: search what clipvault recorded and copy an entry again.

use std::path::PathBuf;

use dojo::launcher_core::clipboard_history::{self, ClipEntry, Clipvault, Pins, default_pins_path};
use dojo::launcher_core::rows::{Row, RowAction, RowIcon};
use nucleo_matcher::Matcher;

use super::loader::{LoadState, Loader};
use super::{Host, Notify, Outcome, Secondary, View};

/// The history store behind the view: clipvault in production, a fake in tests.
trait Vault: Send + Sync {
    fn list(&self) -> anyhow::Result<Vec<ClipEntry>>;
    fn copy(&self, entry: &ClipEntry) -> anyhow::Result<()>;
    fn delete(&self, id: u64) -> anyhow::Result<()>;
}

impl Vault for Clipvault {
    fn list(&self) -> anyhow::Result<Vec<ClipEntry>> {
        Clipvault::list(self)
    }

    fn copy(&self, entry: &ClipEntry) -> anyhow::Result<()> {
        Clipvault::copy(self, entry)
    }

    fn delete(&self, id: u64) -> anyhow::Result<()> {
        Clipvault::delete(self, id)
    }
}

pub struct ClipboardView {
    vault: std::sync::Arc<dyn Vault>,
    loader: Option<Loader<Vec<ClipEntry>>>,
    notify: Option<Notify>,
    /// The entries behind the rows last returned, by the rows' `Item` position.
    shown: Vec<ClipEntry>,
    pins: Pins,
    pins_path: Option<PathBuf>,
    matcher: Matcher,
    now: fn() -> u64,
}

fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// "just now", "5 min ago", "3 h ago", "2 d ago".
fn ago(now: u64, then: u64) -> String {
    let seconds = now.saturating_sub(then);
    match seconds {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", seconds / 60),
        3600..=86_399 => format!("{} h ago", seconds / 3600),
        _ => format!("{} d ago", seconds / 86_400),
    }
}

impl ClipboardView {
    pub fn new() -> Self {
        let pins_path = default_pins_path().ok();
        Self::with(
            std::sync::Arc::new(Clipvault::new()),
            Pins::default(),
            pins_path,
            now_seconds,
        )
    }

    fn with(
        vault: std::sync::Arc<dyn Vault>,
        pins: Pins,
        pins_path: Option<PathBuf>,
        now: fn() -> u64,
    ) -> Self {
        Self {
            vault,
            loader: None,
            notify: None,
            shown: Vec::new(),
            pins,
            pins_path,
            matcher: Matcher::new(nucleo_matcher::Config::DEFAULT),
            now,
        }
    }

    fn load(&mut self) {
        let Some(notify) = self.notify.clone() else {
            return;
        };
        let vault = self.vault.clone();
        self.loader = Some(Loader::start(notify, move || {
            vault
                .list()
                .map_err(|error| format!("Could not read the clipboard history: {error:#}"))
        }));
    }

    fn save_pins(&self) -> Result<(), String> {
        match &self.pins_path {
            Some(path) => self
                .pins
                .save(path)
                .map_err(|error| format!("Could not save pins: {error:#}")),
            None => Ok(()),
        }
    }
}

impl View for ClipboardView {
    fn placeholder(&self) -> &str {
        "Search clipboard history…"
    }

    fn open(&mut self, notify: Notify) {
        if let Some(path) = &self.pins_path {
            self.pins = Pins::load(path);
        }
        self.notify = Some(notify);
        self.load();
    }

    fn rows(&mut self, query: &str) -> Vec<Row> {
        self.shown.clear();
        let Some(loader) = &self.loader else {
            return Vec::new();
        };
        let (pins, matcher, shown, now) =
            (&self.pins, &mut self.matcher, &mut self.shown, (self.now)());
        loader.with(|state| match state {
            LoadState::Ready(entries) => clipboard_history::search(entries, pins, query, matcher)
                .into_iter()
                .map(|index| {
                    let entry = &entries[index];
                    shown.push(entry.clone());
                    let pinned = pins.is_pinned(entry);
                    let glyph = if pinned { "📌" } else { "📋" };
                    Row {
                        id: String::new(),
                        title: entry.preview.clone(),
                        subtitle: format!("{} · {}", ago(now, entry.copied_at), entry.size_label),
                        icon: RowIcon::Glyph(glyph.into()),
                        action: RowAction::Item(shown.len() - 1),
                    }
                })
                .collect(),
            _ => Vec::new(),
        })
    }

    fn activate(&mut self, row: &Row, _query: &str, _host: &Host) -> Outcome {
        let RowAction::Item(index) = row.action else {
            return Outcome::Stay;
        };
        let Some(entry) = self.shown.get(index) else {
            return Outcome::Stay;
        };
        match self.vault.copy(entry) {
            Ok(()) => Outcome::Close,
            Err(error) => Outcome::Error(format!("Could not copy that: {error:#}")),
        }
    }

    fn secondary(&mut self, key: Secondary, row: &Row, _host: &Host) -> Option<Outcome> {
        let RowAction::Item(index) = row.action else {
            return None;
        };
        let entry = self.shown.get(index)?.clone();
        match key {
            Secondary::Pin => {
                let pinned = self.pins.toggle(&entry);
                Some(match self.save_pins() {
                    Ok(()) if pinned => Outcome::Notice("Pinned".into()),
                    Ok(()) => Outcome::Notice("Unpinned".into()),
                    Err(message) => Outcome::Error(message),
                })
            }
            Secondary::Delete => Some(match self.vault.delete(entry.id) {
                Ok(()) => {
                    self.pins.unpin(&entry);
                    let _ = self.save_pins();
                    self.load();
                    Outcome::Notice("Deleted from the history".into())
                }
                Err(error) => Outcome::Error(format!("Could not delete that: {error:#}")),
            }),
            Secondary::Alternate => None,
        }
    }

    fn hint(&self) -> &str {
        "Ctrl+P pin · Ctrl+D delete"
    }

    fn empty_text(&self, query: &str) -> String {
        let state = self.loader.as_ref().map(|loader| {
            loader.with(|state| match state {
                LoadState::Loading => Some("Loading the clipboard history…".to_owned()),
                LoadState::Failed(message) => Some(message.clone()),
                LoadState::Ready(_) => None,
            })
        });
        match state {
            Some(Some(message)) => message,
            _ if query.trim().is_empty() => "The clipboard history is empty".into(),
            _ => "No matching entries".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    const NOW: u64 = 1_800_000_000;

    /// `clipvault list` lines: id, preview, size, last-updated.
    fn entries() -> Vec<ClipEntry> {
        clipboard_history::parse_list(
            "3\tcargo test --lib\t17B\t2027-01-15T07:58:30Z\n\
             2\thttps://example.com/docs\t23B\t2027-01-15T06:00:00Z\n\
             1\t[[ binary data 26kB image/png ]]\t26kB\t2027-01-10T06:00:00Z\n",
        )
    }

    #[derive(Default)]
    struct FakeVault {
        copied: Mutex<Vec<u64>>,
        deleted: Mutex<Vec<u64>>,
        fail_copy: bool,
    }

    impl Vault for FakeVault {
        fn list(&self) -> anyhow::Result<Vec<ClipEntry>> {
            let deleted = self.deleted.lock().unwrap().clone();
            Ok(entries()
                .into_iter()
                .filter(|e| !deleted.contains(&e.id))
                .collect())
        }

        fn copy(&self, entry: &ClipEntry) -> anyhow::Result<()> {
            anyhow::ensure!(!self.fail_copy, "wl-copy is missing");
            self.copied.lock().unwrap().push(entry.id);
            Ok(())
        }

        fn delete(&self, id: u64) -> anyhow::Result<()> {
            self.deleted.lock().unwrap().push(id);
            Ok(())
        }
    }

    fn view(vault: Arc<FakeVault>) -> ClipboardView {
        let mut view = ClipboardView::with(vault, Pins::default(), None, || NOW);
        view.loader = Some(Loader::ready(entries()));
        view
    }

    #[test]
    fn text_entries_list_newest_first_with_age_and_size_and_images_stay_out() {
        assert_eq!(entries().len(), 3, "the fixture parses");
        let mut view = view(Arc::new(FakeVault::default()));
        let rows = view.rows("");
        assert_eq!(rows.len(), 2, "the image entry is not listed");
        assert!(rows[0].title.contains("cargo test"));
        assert_eq!(rows[1].title, "https://example.com/docs");
        assert_eq!(rows[0].icon, RowIcon::Glyph("📋".into()));
        assert_eq!(rows[0].subtitle, "1 min ago · 17B");
        assert_eq!(rows[1].subtitle, "2 h ago · 23B");
    }

    #[test]
    fn search_matches_text_and_enter_copies_the_chosen_entry() {
        let vault = Arc::new(FakeVault::default());
        let mut view = view(vault.clone());
        let rows = view.rows("example");
        assert_eq!(rows.len(), 1);
        assert_eq!(
            view.activate(&rows[0], "example", &Host::default()),
            Outcome::Close
        );
        assert_eq!(vault.copied.lock().unwrap().as_slice(), [2]);
        assert!(view.rows("zzzz").is_empty());
        assert_eq!(view.empty_text("zzzz"), "No matching entries");
    }

    #[test]
    fn a_copy_failure_is_reported() {
        let vault = Arc::new(FakeVault {
            fail_copy: true,
            ..FakeVault::default()
        });
        let mut view = view(vault);
        let rows = view.rows("");
        assert_eq!(
            view.activate(&rows[0], "", &Host::default()),
            Outcome::Error("Could not copy that: wl-copy is missing".into())
        );
    }

    #[test]
    fn pinning_moves_an_entry_up_and_deleting_removes_it() {
        let vault = Arc::new(FakeVault::default());
        let mut view = view(vault.clone());
        let rows = view.rows("");
        let older = rows[1].clone();
        assert_eq!(
            view.secondary(Secondary::Pin, &older, &Host::default()),
            Some(Outcome::Notice("Pinned".into()))
        );
        let rows = view.rows("");
        assert_eq!(rows[0].title, older.title, "pinned entries lead");
        assert_eq!(rows[0].icon, RowIcon::Glyph("📌".into()));
        assert_eq!(
            view.secondary(Secondary::Pin, &rows[0], &Host::default()),
            Some(Outcome::Notice("Unpinned".into()))
        );
        let rows = view.rows("");
        assert_eq!(
            view.secondary(Secondary::Delete, &rows[1], &Host::default()),
            Some(Outcome::Notice("Deleted from the history".into()))
        );
        assert_eq!(vault.deleted.lock().unwrap().as_slice(), [2]);
        assert_eq!(
            view.secondary(Secondary::Alternate, &rows[0], &Host::default()),
            None
        );
    }

    #[test]
    fn ages_read_naturally() {
        assert_eq!(ago(1000, 1000), "just now");
        assert_eq!(ago(1000, 1000 - 59), "just now");
        assert_eq!(ago(1000, 1000 - 60), "1 min ago");
        assert_eq!(ago(10_000, 10_000 - 3600), "1 h ago");
        assert_eq!(ago(1_000_000, 1_000_000 - 3 * 86_400), "3 d ago");
        assert_eq!(
            ago(5, 100),
            "just now",
            "a clock behind the entry does not underflow"
        );
    }

    #[test]
    fn an_unloaded_view_says_so() {
        let view = ClipboardView::with(
            Arc::new(FakeVault::default()),
            Pins::default(),
            None,
            || NOW,
        );
        assert_eq!(view.empty_text(""), "The clipboard history is empty");
    }
}
