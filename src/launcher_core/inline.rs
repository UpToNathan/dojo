//! Inline rows: results an explicit trigger in the query produces (a bang, a calculation, a `?`
//! prompt). Each engine adds its provider here; ordinary text produces nothing.

use super::bangs::{BangMatch, BangSet};
use super::rows::{Row, RowAction, RowIcon};

/// What the providers need, loaded once per launcher opening so typing never reads files.
#[derive(Clone, Debug)]
pub struct InlineState {
    bangs: BangSet,
    /// Why the user's bangs file was not applied, if it was not.
    pub bangs_error: Option<String>,
}

impl InlineState {
    /// Reads the user's configuration. A broken file falls back to the tracked defaults.
    pub fn load() -> Self {
        let loaded = BangSet::for_user();
        Self {
            bangs: loaded.set,
            bangs_error: loaded.error,
        }
    }

    /// The tracked defaults only, with no user files.
    pub fn tracked() -> Self {
        Self {
            bangs: BangSet::tracked(),
            bangs_error: None,
        }
    }
}

fn bang_row(found: BangMatch) -> Row {
    Row {
        id: format!("bang:{}", found.bang.trigger),
        title: found.title,
        subtitle: "Open in the browser".into(),
        icon: RowIcon::Glyph("!".into()),
        action: RowAction::OpenUrl(found.url),
    }
}

/// Rows for `query`, in the order they should lead the list.
pub fn inline_rows(query: &str, state: &InlineState) -> Vec<Row> {
    state
        .bangs
        .resolve(query)
        .map(bang_row)
        .into_iter()
        .collect()
}

/// What to offer when nothing else matched: a web search in the default engine.
pub fn fallback_rows(query: &str, state: &InlineState) -> Vec<Row> {
    state
        .bangs
        .web_search(query)
        .map(|found| Row {
            id: String::new(),
            ..bang_row(found)
        })
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_text_produces_no_inline_rows() {
        let state = InlineState::tracked();
        assert!(inline_rows("firefox", &state).is_empty());
        assert!(inline_rows("hello!", &state).is_empty());
        assert!(inline_rows("", &state).is_empty());
    }

    #[test]
    fn a_bang_becomes_one_row_that_opens_the_search() {
        let state = InlineState::tracked();
        for query in ["!yt cats", "cats !yt"] {
            let rows = inline_rows(query, &state);
            assert_eq!(rows.len(), 1, "{query}");
            assert_eq!(rows[0].title, "Search YouTube for “cats”");
            assert_eq!(rows[0].id, "bang:yt");
            assert_eq!(
                rows[0].action,
                RowAction::OpenUrl("https://www.youtube.com/results?search_query=cats".into())
            );
        }
        let bare = inline_rows("!yt", &state);
        assert_eq!(bare[0].title, "Open YouTube");
        assert_eq!(
            bare[0].action,
            RowAction::OpenUrl("https://www.youtube.com/".into())
        );
    }

    #[test]
    fn the_fallback_searches_the_default_engine_without_remembering_it() {
        let state = InlineState::tracked();
        let rows = fallback_rows("rust lifetimes", &state);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "Search DuckDuckGo for “rust lifetimes”");
        assert!(rows[0].id.is_empty());
        assert_eq!(
            rows[0].action,
            RowAction::OpenUrl("https://duckduckgo.com/?q=rust+lifetimes".into())
        );
        assert!(fallback_rows("  ", &state).is_empty());
    }
}
