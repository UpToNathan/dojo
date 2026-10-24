//! The rows a launcher list shows, independent of the window that draws them.

use super::commands::CommandId;

/// What a row shows to the left of its title.
#[derive(Clone, Debug, PartialEq)]
pub enum RowIcon {
    None,
    /// Characters drawn as text: an emoji, a symbol.
    Glyph(String),
    /// A filled colour square, sRGB.
    Swatch([u8; 3]),
    /// The icon of the application at this index in the catalogue.
    Application(usize),
}

/// What Enter does on a row.
#[derive(Clone, Debug, PartialEq)]
pub enum RowAction {
    /// Launch the application at this catalogue index.
    Launch(usize),
    /// Push the command's view.
    Open(CommandId),
    /// A row of the open view, by position in that view's own list.
    Item(usize),
    /// Copy the text to the clipboard and close.
    Copy(String),
    /// Open the address in the browser and close.
    OpenUrl(String),
    /// Send the text to the Dojo agent and show its panel.
    AskAgent(String),
    /// A section title. It cannot be selected.
    Header,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    /// Key under which launches of this row are remembered; empty for rows that are not.
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub icon: RowIcon,
    pub action: RowAction,
}

impl Row {
    pub fn header(title: &str) -> Self {
        Self {
            id: String::new(),
            title: title.to_owned(),
            subtitle: String::new(),
            icon: RowIcon::None,
            action: RowAction::Header,
        }
    }

    pub fn is_selectable(&self) -> bool {
        self.action != RowAction::Header
    }
}

/// Which way a selection moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Up,
    Down,
}

/// The first row that can be selected.
pub fn first_selectable(rows: &[Row]) -> usize {
    rows.iter().position(Row::is_selectable).unwrap_or(0)
}

/// The selection after one step, skipping section titles and stopping at the ends.
pub fn step_selection(rows: &[Row], selected: usize, step: Step) -> usize {
    let candidates: Box<dyn Iterator<Item = usize>> = match step {
        Step::Up => Box::new((0..selected.min(rows.len())).rev()),
        Step::Down => Box::new(selected.saturating_add(1)..rows.len()),
    };
    for index in candidates {
        if rows[index].is_selectable() {
            return index;
        }
    }
    selected.min(rows.len().saturating_sub(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(title: &str) -> Row {
        Row {
            id: title.to_owned(),
            title: title.to_owned(),
            subtitle: String::new(),
            icon: RowIcon::None,
            action: RowAction::Item(0),
        }
    }

    #[test]
    fn selection_skips_headers_and_stops_at_the_ends() {
        let rows = vec![
            Row::header("Suggestions"),
            item("a"),
            item("b"),
            Row::header("Applications"),
            item("c"),
        ];
        assert_eq!(first_selectable(&rows), 1);
        assert_eq!(step_selection(&rows, 1, Step::Down), 2);
        assert_eq!(step_selection(&rows, 2, Step::Down), 4);
        assert_eq!(step_selection(&rows, 4, Step::Down), 4);
        assert_eq!(step_selection(&rows, 4, Step::Up), 2);
        assert_eq!(step_selection(&rows, 1, Step::Up), 1);
    }

    #[test]
    fn empty_and_all_header_lists_do_not_panic() {
        assert_eq!(first_selectable(&[]), 0);
        assert_eq!(step_selection(&[], 0, Step::Down), 0);
        assert_eq!(step_selection(&[], 3, Step::Up), 0);
        let headers = vec![Row::header("a"), Row::header("b")];
        assert_eq!(step_selection(&headers, 0, Step::Down), 0);
    }
}
