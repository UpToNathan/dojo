//! Inline rows: results an explicit trigger in the query produces (a bang, a calculation, a `?`
//! prompt). Each engine adds its provider here; ordinary text produces nothing.

use super::bangs::{BangMatch, BangSet};
use super::calculator;
use super::color;
use super::devutil::{self, DevEnv, SystemEnv};
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

/// The rows engines produced for one query.
#[derive(Debug, Default, PartialEq)]
pub struct InlineRows {
    /// Rows for unambiguous triggers; they lead the list.
    pub leading: Vec<Row>,
    /// Rows for triggers that are also ordinary words (`title`, `lower`): they follow the results.
    pub trailing: Vec<Row>,
}

fn copy_row(glyph: &str, title: String, subtitle: String, text: String) -> Row {
    Row {
        id: String::new(),
        title,
        subtitle,
        icon: RowIcon::Glyph(glyph.to_owned()),
        action: RowAction::Copy(text),
    }
}

/// `? <message>` sends the message to the Dojo agent. A bare `?` only says how.
fn agent_row(query: &str) -> Option<Row> {
    let message = query.trim_start().strip_prefix('?')?.trim();
    if message.is_empty() {
        return Some(Row::header(
            "Ask the Dojo agent: type your message after the ?",
        ));
    }
    Some(Row {
        id: String::new(),
        title: format!("Ask the Dojo agent: {message}"),
        subtitle: "Sends it as a message".into(),
        icon: RowIcon::Glyph("?".into()),
        action: RowAction::AskAgent(message.to_owned()),
    })
}

/// Words that start dev-utility rows and also start everyday searches.
const CASE_WORDS: [&str; 7] = [
    "snake", "camel", "kebab", "pascal", "upper", "lower", "title",
];

/// A colour written with syntax (`#ff8800`, `rgb(…)`) as opposed to a bare name like `orange`,
/// which is also a word people search for.
fn is_color_syntax(query: &str) -> bool {
    let query = query.trim();
    query.starts_with('#') || query.contains('(')
}

/// Rows for `query`: bangs, calculations, colours and dev utilities.
pub fn inline_rows(query: &str, state: &InlineState) -> InlineRows {
    inline_rows_with(query, state, &SystemEnv)
}

fn inline_rows_with(query: &str, state: &InlineState, env: &impl DevEnv) -> InlineRows {
    let mut rows = InlineRows::default();
    rows.leading.extend(agent_row(query));
    rows.leading
        .extend(state.bangs.resolve(query).map(bang_row));
    if let Some(answer) = calculator::evaluate(query) {
        rows.leading
            .push(copy_row("=", answer.display, answer.label, answer.copy));
    }
    if is_color_syntax(query) {
        if let Some(answer) = color::evaluate(query) {
            let swatch = [answer.color.r, answer.color.g, answer.color.b];
            for format in &answer.formats {
                rows.leading.push(Row {
                    id: String::new(),
                    title: format.text.clone(),
                    subtitle: format.label.to_owned(),
                    icon: RowIcon::Swatch(swatch),
                    action: RowAction::Copy(format.text.clone()),
                });
            }
            rows.leading.push(Row {
                id: String::new(),
                title: format!(
                    "Contrast {} on white · {} on black",
                    answer.on_white.text(),
                    answer.on_black.text()
                ),
                subtitle: "WCAG contrast".into(),
                icon: RowIcon::Swatch(swatch),
                action: RowAction::Copy(answer.on_white.ratio_text()),
            });
        }
    }
    if let Some(answers) = devutil::evaluate(query, env) {
        let trailing = query
            .split_whitespace()
            .next()
            .is_some_and(|word| CASE_WORDS.contains(&word.to_ascii_lowercase().as_str()));
        let target = if trailing {
            &mut rows.trailing
        } else {
            &mut rows.leading
        };
        target.extend(
            answers
                .into_iter()
                .map(|answer| copy_row("⌘", answer.display, answer.label, answer.copy)),
        );
    }
    rows
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

    struct FixedEnv;

    impl DevEnv for FixedEnv {
        fn now_ms(&self) -> u64 {
            1_800_000_000_000
        }

        fn fill_random(&self, buffer: &mut [u8]) -> bool {
            buffer.fill(7);
            true
        }
    }

    fn leading(query: &str) -> Vec<Row> {
        inline_rows_with(query, &InlineState::tracked(), &FixedEnv).leading
    }

    #[test]
    fn calculations_colours_and_dev_utilities_become_copy_rows() {
        let sum = leading("2 + 2 * 10");
        assert_eq!(sum.len(), 1);
        assert_eq!(sum[0].title, "= 22");
        assert_eq!(sum[0].action, RowAction::Copy("22".into()));

        let colour = leading("#ff8800");
        let titles = colour
            .iter()
            .map(|row| row.title.as_str())
            .collect::<Vec<_>>();
        assert!(titles.contains(&"#ff8800") && titles.iter().any(|t| t.starts_with("rgb(")));
        assert!(titles.last().unwrap().starts_with("Contrast "));
        assert_eq!(colour[0].icon, RowIcon::Swatch([255, 136, 0]));
        assert_eq!(colour[0].action, RowAction::Copy("#ff8800".into()));

        let hash = leading("sha256 hello");
        assert_eq!(
            hash[0].action,
            RowAction::Copy(
                "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824".into()
            )
        );
        assert_eq!(leading("uuid4")[0].subtitle, "UUID v4");
    }

    #[test]
    fn bare_colour_names_and_case_words_never_displace_results() {
        // "orange" is a colour name but also a search; only syntax makes a colour row.
        assert!(leading("orange").is_empty());
        assert!(!leading("rgb(255, 136, 0)").is_empty());
        // "title" and friends follow the results instead of leading them.
        let rows = inline_rows_with("snake hello world", &InlineState::tracked(), &FixedEnv);
        assert!(rows.leading.is_empty());
        assert_eq!(rows.trailing.len(), 1);
        assert_eq!(
            rows.trailing[0].action,
            RowAction::Copy("hello_world".into())
        );
    }

    #[test]
    fn a_question_mark_prefix_asks_the_agent_and_a_bare_one_explains() {
        let rows = leading("? what is on my calendar");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "Ask the Dojo agent: what is on my calendar");
        assert_eq!(
            rows[0].action,
            RowAction::AskAgent("what is on my calendar".into())
        );
        let spaced = leading("  ?hello");
        assert_eq!(spaced[0].action, RowAction::AskAgent("hello".into()));
        let bare = leading("?");
        assert_eq!(bare.len(), 1);
        assert!(!bare[0].is_selectable(), "a bare ? only explains");
        // A question mark elsewhere is ordinary text.
        assert!(leading("what is this?").is_empty());
    }

    #[test]
    fn ordinary_text_produces_no_inline_rows() {
        let state = InlineState::tracked();
        for query in [
            "firefox",
            "hello!",
            "",
            "firefox 2",
            "42",
            "orange",
            "title bar",
            "ts",
        ] {
            let rows = inline_rows_with(query, &state, &FixedEnv);
            // "ts" and "title bar" are real triggers; everything else is silent.
            if !matches!(query, "ts" | "title bar") {
                assert_eq!(rows, InlineRows::default(), "{query}");
            }
        }
    }

    #[test]
    fn a_bang_becomes_one_row_that_opens_the_search() {
        let state = InlineState::tracked();
        for query in ["!yt cats", "cats !yt"] {
            let rows = inline_rows_with(query, &state, &FixedEnv).leading;
            assert_eq!(rows.len(), 1, "{query}");
            assert_eq!(rows[0].title, "Search YouTube for “cats”");
            assert_eq!(rows[0].id, "bang:yt");
            assert_eq!(
                rows[0].action,
                RowAction::OpenUrl("https://www.youtube.com/results?search_query=cats".into())
            );
        }
        let bare = inline_rows_with("!yt", &state, &FixedEnv).leading;
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
