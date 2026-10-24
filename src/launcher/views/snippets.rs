//! The Snippets command: saved text, copied with its placeholders filled in.

use std::path::PathBuf;

use dojo::launcher_core::rows::{Row, RowAction, RowIcon};
use dojo::launcher_core::snippets::{self, LoadedSnippets, Snippet, SystemContext};
use nucleo_matcher::Matcher;

use super::{Host, Notify, Outcome, View};

pub struct SnippetsView {
    loaded: LoadedSnippets,
    /// The file to read and save to; the user's by default, a temporary one in tests.
    path: Option<PathBuf>,
    matcher: Matcher,
}

/// One line of the snippet's text for the subtitle.
fn preview(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    let more = text.trim().lines().count() > 1;
    let mut shown = line.chars().take(60).collect::<String>();
    if shown.chars().count() < line.chars().count() || more {
        shown.push('…');
    }
    shown
}

impl SnippetsView {
    pub fn new() -> Self {
        Self::from_loaded(
            LoadedSnippets::for_user(),
            snippets::user_config_path().ok(),
        )
    }

    fn from_loaded(loaded: LoadedSnippets, path: Option<PathBuf>) -> Self {
        Self {
            loaded,
            path,
            matcher: Matcher::new(nucleo_matcher::Config::DEFAULT),
        }
    }

    fn reload(&mut self) {
        self.loaded = match &self.path {
            Some(path) => LoadedSnippets::from_path(path),
            None => LoadedSnippets::for_user(),
        };
    }

    fn snippet_row(&self, index: usize) -> Row {
        let snippet = &self.loaded.snippets[index];
        Row {
            id: format!("snippet:{}", snippet.name),
            title: snippet.name.clone(),
            subtitle: preview(&snippet.text),
            icon: RowIcon::Glyph("✂️".into()),
            action: RowAction::Item(index),
        }
    }

    /// The row that saves the clipboard under the typed name. Its position is past the snippets.
    fn save_row(&self, name: &str) -> Row {
        Row {
            id: String::new(),
            title: format!("Save clipboard as “{name}”"),
            subtitle: "Adds a snippet".into(),
            icon: RowIcon::Glyph("＋".into()),
            action: RowAction::Item(self.loaded.snippets.len()),
        }
    }
}

impl View for SnippetsView {
    fn placeholder(&self) -> &str {
        "Search snippets…"
    }

    fn open(&mut self, _notify: Notify) {
        self.reload();
    }

    fn rows(&mut self, query: &str) -> Vec<Row> {
        let query = query.trim();
        let order = if query.is_empty() {
            (0..self.loaded.snippets.len()).collect::<Vec<_>>()
        } else {
            snippets::search(&self.loaded.snippets, query, &mut self.matcher)
        };
        let mut rows = order
            .into_iter()
            .map(|index| self.snippet_row(index))
            .collect::<Vec<_>>();
        let named = self
            .loaded
            .snippets
            .iter()
            .any(|snippet| snippet.name.eq_ignore_ascii_case(query));
        if !query.is_empty() && !named && self.loaded.error.is_none() {
            rows.push(self.save_row(query));
        }
        rows
    }

    fn activate(&mut self, row: &Row, query: &str, host: &Host) -> Outcome {
        let RowAction::Item(index) = row.action else {
            return Outcome::Stay;
        };
        if let Some(snippet) = self.loaded.snippets.get(index) {
            let context = SystemContext {
                clipboard: host.clipboard.clone().unwrap_or_default(),
            };
            return Outcome::Copy(snippets::expand(snippet, &context));
        }
        let name = query.trim();
        let text = host.clipboard.clone().unwrap_or_default();
        let Some(path) = self.path.clone() else {
            return Outcome::Error("Could not find where snippets are stored".into());
        };
        if text.trim().is_empty() {
            return Outcome::Error("Copy some text first, then save it as a snippet".into());
        }
        let snippet = Snippet {
            name: name.to_owned(),
            keywords: Vec::new(),
            text,
        };
        match snippets::append(&path, &snippet) {
            Ok(()) => {
                self.reload();
                Outcome::Notice(format!("Saved snippet “{name}”"))
            }
            Err(error) => Outcome::Error(format!("Could not save the snippet: {error:#}")),
        }
    }

    fn empty_text(&self, query: &str) -> String {
        match self.loaded.empty_hint() {
            Some(hint) if query.trim().is_empty() => hint,
            _ => "No snippets found".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(directory: &std::path::Path, file: &str) -> SnippetsView {
        let path = directory.join("dojo/snippets.toml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, file).unwrap();
        SnippetsView::from_loaded(LoadedSnippets::from_path(&path), Some(path))
    }

    const FILE: &str = r#"
[[snippet]]
name = "Sign-off"
keywords = ["sig", "regards"]
text = "Kind regards,\nNathan"

[[snippet]]
name = "Today"
text = "{date}"

[[snippet]]
name = "Pasted"
text = "was: {clipboard}"
"#;

    fn host(clipboard: Option<&str>) -> Host {
        Host {
            clipboard: clipboard.map(str::to_owned),
        }
    }

    #[test]
    fn rows_list_snippets_with_a_one_line_preview_and_filter_by_name_and_keyword() {
        let directory = tempfile::tempdir().unwrap();
        let mut view = view(directory.path(), FILE);
        let rows = view.rows("");
        assert_eq!(
            rows.iter()
                .map(|row| row.title.as_str())
                .collect::<Vec<_>>(),
            ["Sign-off", "Today", "Pasted"]
        );
        assert_eq!(rows[0].subtitle, "Kind regards,…");
        assert_eq!(rows[0].id, "snippet:Sign-off");
        assert_eq!(view.rows("sig")[0].title, "Sign-off");
        assert_eq!(view.rows("regards")[0].title, "Sign-off");
        let none = view.rows("zzz");
        assert_eq!(none.len(), 1, "only the save row");
        assert_eq!(none[0].title, "Save clipboard as “zzz”");
    }

    #[test]
    fn activating_a_snippet_expands_its_placeholders() {
        let directory = tempfile::tempdir().unwrap();
        let mut view = view(directory.path(), FILE);
        let rows = view.rows("");
        assert_eq!(
            view.activate(&rows[2], "", &host(Some("hello"))),
            Outcome::Copy("was: hello".into())
        );
        let Outcome::Copy(date) = view.activate(&rows[1], "", &host(None)) else {
            panic!("a snippet copies");
        };
        assert!(
            date.len() == 10 && date.chars().nth(4) == Some('-'),
            "{date}"
        );
        assert_eq!(
            view.activate(&rows[0], "", &host(None)),
            Outcome::Copy("Kind regards,\nNathan".into())
        );
    }

    #[test]
    fn the_save_row_adds_the_clipboard_under_the_typed_name() {
        let directory = tempfile::tempdir().unwrap();
        let mut view = view(directory.path(), FILE);
        let rows = view.rows("Address");
        let save = rows.last().unwrap().clone();
        assert_eq!(
            view.activate(&save, "Address", &host(None)),
            Outcome::Error("Copy some text first, then save it as a snippet".into())
        );
        assert_eq!(
            view.activate(&save, "Address", &host(Some("1 Main St"))),
            Outcome::Notice("Saved snippet “Address”".into())
        );
        let rows = view.rows("");
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[3].title, "Address");
        assert_eq!(
            view.activate(&rows[3], "", &host(None)),
            Outcome::Copy("1 Main St".into())
        );
        // The file keeps its earlier entries, and an existing name offers no second save row.
        let written = std::fs::read_to_string(directory.path().join("dojo/snippets.toml")).unwrap();
        assert!(written.contains("Sign-off") && written.contains("Address"));
        assert!(
            view.rows("address")
                .iter()
                .all(|row| !row.title.starts_with("Save"))
        );
    }

    #[test]
    fn a_missing_file_explains_how_to_add_snippets_and_a_broken_one_says_so() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("dojo/snippets.toml");
        let mut view =
            SnippetsView::from_loaded(LoadedSnippets::from_path(&path), Some(path.clone()));
        assert!(view.rows("").is_empty());
        assert!(view.empty_text("").contains("No snippets yet. Add some to"));
        assert_eq!(view.empty_text("x"), "No snippets found");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "[[snippet]]\nname = ").unwrap();
        view.reload();
        assert!(
            view.rows("typed").is_empty(),
            "no save row over a broken file"
        );
        assert!(view.empty_text("").contains("could not be read"));
    }

    #[test]
    fn long_previews_are_cut() {
        assert_eq!(preview("short"), "short");
        assert_eq!(preview("a\nb"), "a…");
        assert_eq!(preview(&"x".repeat(80)), format!("{}…", "x".repeat(60)));
        assert_eq!(preview("\n\n  first\nsecond"), "first…");
        assert_eq!(preview(""), "");
    }
}
