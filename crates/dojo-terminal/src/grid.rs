//! Pure grid arithmetic: how many cells fit, and how a row of cells groups into runs.
use dojo_rex::{Cell, Rgb};

/// Columns and rows that fit in `width` x `height` logical px, at least 1 each.
pub fn cols_rows(width: f32, height: f32, cell_width: f32, cell_height: f32) -> (u16, u16) {
    let fit = |size: f32, cell: f32| {
        if cell > 0.0 && size.is_finite() { ((size / cell).floor() as i64).clamp(1, u16::MAX as i64) as u16 } else { 1 }
    };
    (fit(width, cell_width), fit(height, cell_height))
}

/// A cell's size in device px, for the terminal's pixel size reports.
pub fn cell_px(cell: f32, scale: f32) -> u32 {
    (cell * scale).round().max(1.0) as u32
}

/// Consecutive cells with the same style, drawn as one piece of text at `col`.
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub col: u16,
    /// Grid columns covered; wide characters cover two.
    pub cols: u16,
    pub text: String,
    pub fg: Rgb,
    pub bg: Option<Rgb>,
    pub bold: bool,
    pub italic: bool,
    pub faint: bool,
    pub underline: bool,
    pub strikethrough: bool,
}

impl Run {
    fn of(cell: &Cell, col: u16) -> Self {
        Self {
            col,
            cols: 0,
            text: String::new(),
            fg: cell.fg,
            bg: cell.bg,
            bold: cell.bold,
            italic: cell.italic,
            faint: cell.faint,
            underline: cell.underline,
            strikethrough: cell.strikethrough,
        }
    }

    fn same_style(&self, cell: &Cell) -> bool {
        self.fg == cell.fg
            && self.bg == cell.bg
            && self.bold == cell.bold
            && self.italic == cell.italic
            && self.faint == cell.faint
            && self.underline == cell.underline
            && self.strikethrough == cell.strikethrough
    }

    /// Nothing to see: spaces with no background or line.
    pub fn is_blank(&self) -> bool {
        self.bg.is_none() && !self.underline && !self.strikethrough && self.text.chars().all(|c| c == ' ')
    }
}

/// Characters that take two columns. The cell after one is empty.
pub fn is_wide(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x115F | 0x2E80..=0x303E | 0x3041..=0x33FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF
        | 0xA000..=0xA4CF | 0xAC00..=0xD7A3 | 0xF900..=0xFAFF | 0xFE30..=0xFE6F | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6 | 0x1F300..=0x1F64F | 0x1F900..=0x1F9FF | 0x20000..=0x3FFFD)
}

/// Groups a row into runs. A wide character is a run of its own so that the text after it is
/// placed on the grid again instead of trusting the font's advance. Blank trailing runs are
/// dropped.
pub fn runs(row: &[Cell]) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    let mut col = 0;
    while col < row.len() {
        let cell = &row[col];
        let wide = cell.text.chars().next().is_some_and(is_wide);
        let width = if wide { 2 } else { 1 };
        let text = if cell.text.is_empty() { " " } else { cell.text.as_str() };
        let extend = !wide && runs.last().is_some_and(|run| run.same_style(cell) && !run.text.chars().next().is_some_and(is_wide));
        if extend {
            let run = runs.last_mut().unwrap();
            run.text.push_str(text);
            run.cols += 1;
        } else {
            let mut run = Run::of(cell, col as u16);
            run.text.push_str(text);
            run.cols = width;
            runs.push(run);
        }
        col += width as usize;
    }
    while runs.last().is_some_and(Run::is_blank) {
        runs.pop();
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(text: &str) -> Cell {
        Cell { text: text.into(), ..Cell::default() }
    }

    #[test]
    fn the_grid_is_the_floor_of_the_size_over_the_cell() {
        assert_eq!(cols_rows(800.0, 400.0, 8.5, 17.0), (94, 23));
        assert_eq!(cols_rows(3.0, 3.0, 8.5, 17.0), (1, 1));
        assert_eq!(cols_rows(100.0, 100.0, 0.0, 0.0), (1, 1));
        assert_eq!(cols_rows(f32::NAN, 100.0, 8.0, 10.0), (1, 10));
    }

    #[test]
    fn cell_pixels_scale_and_round() {
        assert_eq!(cell_px(8.57, 1.75), 15);
        assert_eq!(cell_px(17.14, 2.0), 34);
        assert_eq!(cell_px(0.0, 1.0), 1);
    }

    #[test]
    fn same_style_cells_share_a_run() {
        let red = Rgb { r: 255, g: 0, b: 0 };
        let mut row = vec![cell("h"), cell("i"), cell(""), cell("x"), cell("y")];
        row[3].fg = red;
        row[4].fg = red;
        let runs = runs(&row);
        assert_eq!(runs.len(), 2);
        assert_eq!((runs[0].col, runs[0].cols, runs[0].text.as_str()), (0, 3, "hi "));
        assert_eq!((runs[1].col, runs[1].cols, runs[1].text.as_str()), (3, 2, "xy"));
    }

    #[test]
    fn wide_characters_stand_alone_and_skip_their_tail() {
        let row = vec![cell("a"), cell("漢"), cell(""), cell("b")];
        let runs = runs(&row);
        let shape: Vec<_> = runs.iter().map(|r| (r.col, r.cols, r.text.as_str())).collect();
        assert_eq!(shape, [(0, 1, "a"), (1, 2, "漢"), (3, 1, "b")]);
    }

    #[test]
    fn blank_tails_go_but_backgrounds_and_lines_stay() {
        let mut row = vec![cell("a"), cell(""), cell(""), cell("")];
        assert_eq!(runs(&row).len(), 1);
        row[3].bg = Some(Rgb { r: 1, g: 2, b: 3 });
        let kept = runs(&row);
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[1].col, 1 + 2);
        row[3].bg = None;
        row[2].underline = true;
        assert_eq!(runs(&row).len(), 2);
    }

    #[test]
    fn split_panes_get_a_grid_each_from_their_rectangle() {
        let layout = dojo_rex::Layout::Split {
            id: "s".into(),
            horizontal: true,
            ratio: 0.5,
            before: Box::new(dojo_rex::Layout::Block("a".into())),
            after: Box::new(dojo_rex::Layout::Block("b".into())),
        };
        let rects = layout.rects([0.0, 0.0, 801.0, 340.0], 1.0);
        let grids: Vec<_> = rects.iter().map(|(_, [_, _, w, h])| cols_rows(*w, *h, 8.0, 17.0)).collect();
        assert_eq!(grids, [(50, 20), (50, 20)]);
    }
}
