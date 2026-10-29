//! The session picker: which sessions show for a query, and how the panel looks.
use dojo_rex::{SessionSummary, Status};
use rune::{Color, El, el, text};

use crate::draw::{self, Metrics, Palette};

const MAX_ROWS: usize = 12;
const PAD: f32 = 12.0;

#[derive(Default)]
pub struct Picker {
    pub query: String,
    /// None until the listing arrives.
    pub entries: Option<Vec<SessionSummary>>,
    pub selected: usize,
}

#[derive(Debug, PartialEq)]
pub enum Accept {
    Switch(String),
    Create(String),
    Nothing,
}

/// Most recent first (by `recent`), then the server's order.
pub fn order(mut entries: Vec<SessionSummary>, recent: &[String]) -> Vec<SessionSummary> {
    entries.sort_by_key(|entry| recent.iter().position(|label| *label == entry.label).unwrap_or(usize::MAX));
    entries
}

/// Whether the characters of `needle` appear in `haystack` in order, ignoring case.
pub fn subsequence(needle: &str, haystack: &str) -> bool {
    let mut hay = haystack.chars().flat_map(char::to_lowercase);
    needle.chars().flat_map(char::to_lowercase).all(|c| hay.any(|h| h == c))
}

/// Entries whose label matches, then ones whose directory does; all of them for an empty query.
pub fn filter<'a>(entries: &'a [SessionSummary], query: &str) -> Vec<&'a SessionSummary> {
    let query = query.trim();
    if query.is_empty() {
        return entries.iter().collect();
    }
    let by_label = entries.iter().filter(|entry| subsequence(query, &entry.label));
    let by_directory = entries
        .iter()
        .filter(|entry| !subsequence(query, &entry.label) && entry.directory.as_deref().is_some_and(|dir| subsequence(query, dir)));
    by_label.chain(by_directory).collect()
}

pub fn home_abbreviated(directory: &str, home: Option<&str>) -> String {
    match home.filter(|home| !home.is_empty()) {
        Some(home) if directory == home => "~".into(),
        Some(home) if directory.strip_prefix(home).is_some_and(|rest| rest.starts_with('/')) => format!("~{}", &directory[home.len()..]),
        _ => directory.to_owned(),
    }
}

/// `text` cut to `width` characters, ending in an ellipsis when it was longer.
pub fn fit(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(width.saturating_sub(1)).collect();
    cut.push('\u{2026}');
    cut
}

impl Picker {
    pub fn visible(&self) -> Vec<&SessionSummary> {
        self.entries.as_deref().map(|entries| filter(entries, &self.query)).unwrap_or_default()
    }

    pub fn type_text(&mut self, typed: &str) {
        self.query.push_str(typed);
        self.selected = 0;
    }

    pub fn backspace(&mut self) {
        self.query.pop();
        self.selected = 0;
    }

    pub fn step(&mut self, down: bool) {
        let count = self.visible().len();
        if count == 0 {
            return;
        }
        self.selected = if down { (self.selected + 1) % count } else { (self.selected + count - 1) % count };
    }

    pub fn accept(&self) -> Accept {
        if let Some(entry) = self.visible().get(self.selected) {
            return Accept::Switch(entry.label.clone());
        }
        match self.query.trim() {
            "" => Accept::Nothing,
            name => Accept::Create(name.to_owned()),
        }
    }
}

fn mark(status: Option<Status>, palette: &Palette) -> (&'static str, Color) {
    match status {
        Some(Status::Blocked | Status::Error) => ("!", palette.warning),
        Some(Status::Done) => ("\u{2713}", palette.accent),
        Some(Status::Working) => ("\u{25cf}", palette.muted),
        None => (" ", palette.dim),
    }
}

fn label(content: String, x: f32, y: f32, color: Color, key: String) -> El {
    text(content).key(key).font(draw::font()).color(color).absolute(x, y)
}

/// The panel, centred in an area of `area` logical px, over a backdrop that keeps clicks away
/// from the panes.
pub fn view(picker: &Picker, m: Metrics, area: (f32, f32), palette: &Palette, home: Option<&str>) -> El {
    let cols = (((area.0 - 48.0 - 2.0 * PAD) / m.cell_width).floor() as usize).clamp(30, 110);
    let visible = picker.visible();
    let first = (picker.selected + 1).saturating_sub(MAX_ROWS);
    let shown: Vec<_> = visible.iter().skip(first).take(MAX_ROWS).collect();
    let body_rows = shown.len().max(1);
    let width = cols as f32 * m.cell_width + 2.0 * PAD;
    let height = (body_rows + 2) as f32 * m.cell_height + 2.0 * PAD;
    let (x, y) = (((area.0 - width) / 2.0).max(0.0), ((area.1 - height) / 3.0).max(0.0));
    let (label_w, dir_w) = (20usize, (cols / 3).clamp(12, 40));
    let dir_at = label_w + 3;
    let title_at = dir_at + dir_w + 1;
    let title_w = cols.saturating_sub(title_at);

    let mut panel = el()
        .key("picker:panel")
        .absolute(x, y)
        .size(width, height)
        .radius(6.0)
        .fill(palette.panel)
        .border(1.0, palette.dim);
    let top = PAD;
    panel = panel.child(label(format!("> {}\u{258f}", picker.query), PAD, top, palette.fg, "picker:query".into()));
    let rows_top = top + 2.0 * m.cell_height;
    if picker.entries.is_none() {
        panel = panel.child(label("\u{2026}".into(), PAD, rows_top, palette.muted, "picker:loading".into()));
    } else if shown.is_empty() {
        let hint = match picker.query.trim() {
            "" => "no sessions".to_owned(),
            name => format!("Enter creates \u{201c}{}\u{201d}", fit(name, cols.saturating_sub(20))),
        };
        panel = panel.child(label(hint, PAD, rows_top, palette.muted, "picker:empty".into()));
    }
    for (i, entry) in shown.iter().enumerate() {
        let row_y = rows_top + i as f32 * m.cell_height;
        let key = |part: &str| format!("picker:{part}:{}", entry.id);
        if first + i == picker.selected {
            panel = panel.child(el().key(key("sel")).absolute(PAD / 2.0, row_y).size(width - PAD, m.cell_height).radius(3.0).fill(palette.selected));
        }
        let (symbol, symbol_color) = mark(entry.status, palette);
        let at = |col: usize| PAD + col as f32 * m.cell_width;
        panel = panel
            .child(label(fit(&entry.label, label_w), at(0), row_y, palette.fg, key("label")))
            .child(label(symbol.into(), at(label_w + 1), row_y, symbol_color, key("mark")))
            .child(label(fit(&home_abbreviated(entry.directory.as_deref().unwrap_or(""), home), dir_w), at(dir_at), row_y, palette.muted, key("dir")))
            .child(label(fit(entry.title.as_deref().unwrap_or(""), title_w), at(title_at), row_y, palette.muted, key("title")));
    }
    el().key("picker")
        .absolute(0.0, 0.0)
        .size(area.0, area.1)
        .fill(Color { r: 0.0, g: 0.0, b: 0.0, a: 0.45 })
        .focusable(false)
        .on_press(|_: &mut crate::term::Term| {})
        .child(panel)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(label: &str, directory: Option<&str>) -> SessionSummary {
        SessionSummary { id: label.into(), label: label.into(), tabs: 1, directory: directory.map(Into::into), title: None, status: None }
    }

    #[test]
    fn recent_sessions_come_first_then_server_order() {
        let entries = vec![entry("a", None), entry("b", None), entry("c", None), entry("d", None)];
        let ordered = order(entries, &["c".to_owned(), "a".to_owned()]);
        let labels: Vec<_> = ordered.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(labels, ["c", "a", "b", "d"]);
    }

    #[test]
    fn typing_matches_a_case_insensitive_subsequence() {
        assert!(subsequence("dj", "Dojo-Quick"));
        assert!(subsequence("DQ", "dojo-quick"));
        assert!(!subsequence("qd", "dojo-quick"));
        assert!(subsequence("", "x"));
    }

    #[test]
    fn labels_match_before_directories() {
        let entries = vec![entry("notes", Some("/home/u/work")), entry("work", None), entry("misc", Some("/srv/Work"))];
        let found: Vec<_> = filter(&entries, "wrk").iter().map(|e| e.label.as_str()).collect();
        assert_eq!(found, ["work", "notes", "misc"]);
        assert_eq!(filter(&entries, "").len(), 3);
        assert!(filter(&entries, "zzz").is_empty());
    }

    #[test]
    fn enter_switches_creates_or_does_nothing() {
        let mut picker = Picker { entries: Some(vec![entry("alpha", None), entry("beta", None)]), ..Picker::default() };
        assert_eq!(picker.accept(), Accept::Switch("alpha".into()));
        picker.step(true);
        assert_eq!(picker.accept(), Accept::Switch("beta".into()));
        picker.step(true);
        assert_eq!(picker.selected, 0);
        picker.type_text("newname");
        assert_eq!(picker.accept(), Accept::Create("newname".into()));
        picker.backspace();
        picker.query.clear();
        picker.entries = Some(Vec::new());
        assert_eq!(picker.accept(), Accept::Nothing);
    }

    #[test]
    fn home_is_abbreviated_and_long_text_is_cut() {
        assert_eq!(home_abbreviated("/home/u/Dojo", Some("/home/u")), "~/Dojo");
        assert_eq!(home_abbreviated("/home/u", Some("/home/u")), "~");
        assert_eq!(home_abbreviated("/home/user2/x", Some("/home/user")), "/home/user2/x");
        assert_eq!(fit("abcdef", 4), "abc\u{2026}");
        assert_eq!(fit("abc", 4), "abc");
    }
}
