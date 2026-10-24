//! The Emoji command: search emoji by name and keyword and copy the one you pick.

use dojo::launch_history::{self, LaunchHistory};
use dojo::launcher_core::emoji::{self, Emoji, Tone};
use dojo::launcher_core::rows::{Row, RowAction, RowIcon};

use super::{Host, Outcome, View};

/// Emoji shown under Recent in the empty search.
const RECENT_LIMIT: usize = 16;
/// How many search results are considered when promoting ones you have used before.
const PROMOTION_WINDOW: usize = 24;
const SEARCH_LIMIT: usize = 100;

pub struct EmojiView {
    history: LaunchHistory,
    /// The skin tone Tab selected, applied to every emoji that has tones.
    tone: Option<Tone>,
    /// What each row copies, indexed by the row's `Item` position.
    copies: Vec<String>,
    now_ms: fn() -> u64,
}

fn history_id(emoji: &Emoji) -> String {
    format!("emoji:{}", emoji.id())
}

impl EmojiView {
    pub fn new() -> Self {
        Self::with_history(LaunchHistory::default(), launch_history::now_ms)
    }

    fn with_history(history: LaunchHistory, now_ms: fn() -> u64) -> Self {
        Self {
            history,
            tone: None,
            copies: Vec::new(),
            now_ms,
        }
    }

    fn glyph<'a>(&self, emoji: &'a Emoji) -> &'a str {
        self.tone
            .map_or(emoji.glyph.as_str(), |tone| emoji.with_tone(tone))
    }

    fn row(&mut self, emoji: &Emoji) -> Row {
        let glyph = self.glyph(emoji).to_owned();
        self.copies.push(glyph.clone());
        Row {
            id: history_id(emoji),
            title: emoji.name.clone(),
            subtitle: emoji.group.label().to_owned(),
            icon: RowIcon::Glyph(glyph),
            action: RowAction::Item(self.copies.len() - 1),
        }
    }

    /// Emoji for a typed query: the search order, with those you have used before moved up. A
    /// learned search term for the same text comes first, then the most used, but only among the
    /// best matches so an unrelated favourite never jumps in.
    fn promoted(&self, query: &str, now: u64) -> Vec<&'static Emoji> {
        let mut found = emoji::search(query, SEARCH_LIMIT);
        let window = found.len().min(PROMOTION_WINDOW);
        found[..window].sort_by(|left, right| {
            let key = |emoji: &Emoji| {
                let id = history_id(emoji);
                (
                    self.history.term_match(&id, query, now) != launch_history::TermMatch::Exact,
                    self.history.frecency(&id, now),
                )
            };
            let (left, right) = (key(left), key(right));
            left.0.cmp(&right.0).then(right.1.total_cmp(&left.1))
        });
        found
    }

    /// The emoji used before, most used first.
    fn recent(&self, now: u64) -> Vec<&'static Emoji> {
        let mut used = emoji::all()
            .iter()
            .map(|emoji| (emoji, self.history.frecency(&history_id(emoji), now)))
            .filter(|(_, weight)| *weight > 1.0)
            .collect::<Vec<_>>();
        used.sort_by(|left, right| right.1.total_cmp(&left.1));
        used.into_iter()
            .take(RECENT_LIMIT)
            .map(|(emoji, _)| emoji)
            .collect()
    }
}

impl View for EmojiView {
    fn placeholder(&self) -> &str {
        "Search emoji…"
    }

    fn open(&mut self, _notify: super::Notify) {
        self.history = launch_history::default_path().map_or_else(
            |_| LaunchHistory::default(),
            |path| LaunchHistory::load(&path),
        );
    }

    fn rows(&mut self, query: &str) -> Vec<Row> {
        self.copies.clear();
        let now = (self.now_ms)();
        let query = query.trim();
        if !query.is_empty() {
            return self
                .promoted(query, now)
                .into_iter()
                .map(|emoji| self.row(emoji))
                .collect();
        }
        let mut rows = Vec::new();
        let recent = self.recent(now);
        if !recent.is_empty() {
            rows.push(Row::header("Recent"));
            rows.extend(recent.into_iter().map(|emoji| self.row(emoji)));
        }
        let mut group = None;
        for emoji in emoji::all() {
            if group != Some(emoji.group) {
                group = Some(emoji.group);
                rows.push(Row::header(emoji.group.label()));
            }
            rows.push(self.row(emoji));
        }
        rows
    }

    fn activate(&mut self, row: &Row, _query: &str, _host: &Host) -> Outcome {
        match row.action {
            RowAction::Item(index) => self
                .copies
                .get(index)
                .cloned()
                .map_or(Outcome::Stay, Outcome::Copy),
            _ => Outcome::Stay,
        }
    }

    fn tab(&mut self) -> Option<String> {
        self.tone = match self.tone {
            None => Some(Tone::ALL[0]),
            Some(tone) => Tone::ALL
                .get(Tone::ALL.iter().position(|t| *t == tone)? + 1)
                .copied(),
        };
        Some(format!(
            "Skin tone: {} · Tab to change",
            self.tone.map_or("default", Tone::label)
        ))
    }

    fn empty_text(&self, _query: &str) -> String {
        "No emoji found".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000_000;

    fn view_with(history: LaunchHistory) -> EmojiView {
        EmojiView::with_history(history, || NOW)
    }

    fn items(rows: &[Row]) -> Vec<&Row> {
        rows.iter().filter(|row| row.is_selectable()).collect()
    }

    #[test]
    fn the_empty_search_lists_every_group_in_unicode_order() {
        let mut view = view_with(LaunchHistory::default());
        let rows = view.rows("");
        let headers = rows
            .iter()
            .filter(|row| !row.is_selectable())
            .map(|row| row.title.as_str())
            .collect::<Vec<_>>();
        assert_eq!(headers.first(), Some(&"Smileys & Emotion"));
        assert_eq!(headers.last(), Some(&"Flags"));
        assert_eq!(headers.len(), 9);
        assert_eq!(items(&rows).len(), emoji::all().len());
        assert_eq!(items(&rows)[0].title, "grinning face");
        assert_eq!(items(&rows)[0].icon, RowIcon::Glyph("😀".into()));
    }

    #[test]
    fn used_emoji_lead_the_empty_search_under_recent() {
        let mut history = LaunchHistory::default();
        history.record("emoji:😂", "", NOW);
        history.record("emoji:😂", "", NOW);
        history.record("emoji:🔥", "", NOW);
        let mut view = view_with(history);
        let rows = view.rows("");
        assert_eq!(rows[0].title, "Recent");
        assert_eq!(rows[1].icon, RowIcon::Glyph("😂".into()));
        assert_eq!(rows[2].icon, RowIcon::Glyph("🔥".into()));
        assert_eq!(rows[3].title, "Smileys & Emotion");
    }

    #[test]
    fn a_search_finds_by_name_and_keyword() {
        let mut view = view_with(LaunchHistory::default());
        let rows = view.rows("grin");
        assert!(
            !rows.iter().any(|row| !row.is_selectable()),
            "no headers in results"
        );
        assert_eq!(rows[0].icon, RowIcon::Glyph("😀".into()));
        assert_eq!(rows[0].subtitle, "Smileys & Emotion");
        assert_eq!(rows[0].id, "emoji:😀");
        assert!(view.rows("zzzzzz").is_empty());
        assert_eq!(view.empty_text("zzzzzz"), "No emoji found");
    }

    #[test]
    fn use_and_learned_terms_promote_among_the_best_matches() {
        let baseline = emoji::search("face", SEARCH_LIMIT);
        let fifth = baseline[4];
        let mut history = LaunchHistory::default();
        history.record(&history_id(fifth), "", NOW);
        let mut view = view_with(history);
        assert_eq!(
            view.rows("face")[0].id,
            history_id(fifth),
            "a used emoji moves up"
        );

        let sixth = baseline[5];
        let mut history = LaunchHistory::default();
        history.record(&history_id(fifth), "", NOW);
        history.record(&history_id(sixth), "face", NOW);
        let mut view = view_with(history);
        assert_eq!(
            view.rows("face")[0].id,
            history_id(sixth),
            "an exact learned term beats plain use"
        );

        // A favourite far down the list does not jump in.
        let far = baseline[60];
        let mut history = LaunchHistory::default();
        history.record(&history_id(far), "", NOW);
        let mut view = view_with(history);
        let first_rows = view.rows("face");
        assert_eq!(first_rows[0].id, history_id(baseline[0]));
    }

    #[test]
    fn activating_a_row_copies_its_glyph() {
        let mut view = view_with(LaunchHistory::default());
        let rows = view.rows("thumbs up");
        let first = rows.iter().find(|row| row.is_selectable()).unwrap().clone();
        assert_eq!(first.icon, RowIcon::Glyph("👍".into()));
        assert_eq!(
            view.activate(&first, "thumbs up", &Host::default()),
            Outcome::Copy("👍".into())
        );
        let header = Row::header("x");
        assert_eq!(view.activate(&header, "", &Host::default()), Outcome::Stay);
    }

    #[test]
    fn tab_cycles_skin_tones_and_applies_them_to_emoji_that_have_them() {
        let mut view = view_with(LaunchHistory::default());
        let message = view.tab().unwrap();
        assert!(message.contains("light skin tone"), "{message}");
        let rows = view.rows("thumbs up");
        let thumb = rows.iter().find(|row| row.is_selectable()).unwrap().clone();
        assert_eq!(thumb.icon, RowIcon::Glyph("👍🏻".into()));
        assert_eq!(
            view.activate(&thumb, "thumbs up", &Host::default()),
            Outcome::Copy("👍🏻".into())
        );
        // An emoji without tones is unaffected, and its history id stays the base glyph.
        let grin = view.rows("grinning face")[0].clone();
        assert_eq!(grin.icon, RowIcon::Glyph("😀".into()));
        assert_eq!(thumb.id, "emoji:👍");
        // Five more presses come back to the default.
        for _ in 0..4 {
            view.tab();
        }
        assert!(view.tab().unwrap().contains("default"));
        assert_eq!(view.tone, None);
    }
}
