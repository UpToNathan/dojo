//! A screen as Rune elements. Every run is placed at its own column, so text never drifts
//! from the grid whatever the font's advance is.
use dojo_rex::{CursorStyle, Rgb, Screen};
use std::sync::OnceLock;

use rune::{Color, El, TextStyle, el, text};

use crate::grid::{self, Run};

/// The configured font (`settings`), set once at start.
pub static FONT: OnceLock<TextStyle> = OnceLock::new();
/// The gap around the grid in window mode.
pub const WINDOW_INSET: f32 = 8.0;

pub fn font() -> TextStyle {
    *FONT.get_or_init(|| crate::settings::Font::default().style())
}

/// Where the grid sits and how big its cells are, in logical px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    pub cell_width: f32,
    pub cell_height: f32,
    pub inset: f32,
}

/// sRGB bytes as Rune's linear colour.
pub fn color(rgb: Rgb) -> Color {
    let linear = |v: u8| {
        let v = v as f32 / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    Color { r: linear(rgb.r), g: linear(rgb.g), b: linear(rgb.b), a: 1.0 }
}

pub fn screen(screen: &Screen, metrics: Metrics) -> El {
    let background = color(screen.background);
    let mut root = el().w_full().h_full().fill(background);
    for (row, cells) in screen.cells.iter().enumerate() {
        let runs = grid::runs(cells);
        if runs.is_empty() {
            continue;
        }
        let y = metrics.inset + row as f32 * metrics.cell_height;
        for run in &runs {
            root = run_elements(root, run, row, y, metrics, background);
        }
    }
    if let Some(cursor) = screen.cursor {
        let under = screen.cells.get(cursor.y as usize).and_then(|row| row.get(cursor.x as usize));
        root = root.children(cursor_elements(cursor, under.map(|c| c.text.as_str()), screen, metrics));
    }
    root
}

/// Every element is keyed by its place and style: Rune reuses unkeyed children by position and
/// would keep an old run's colour on new text.
fn run_elements(mut line: El, run: &Run, row: usize, y: f32, m: Metrics, background: Color) -> El {
    let style_key = format!(
        "{row}:{}:{:?}:{:?}:{}{}{}{}{}:{}",
        run.col, run.fg, run.bg, run.bold as u8, run.italic as u8, run.faint as u8, run.underline as u8, run.strikethrough as u8, run.cols
    );
    let x = m.inset + run.col as f32 * m.cell_width;
    let width = run.cols as f32 * m.cell_width;
    if let Some(bg) = run.bg {
        line = line.child(el().key(format!("bg:{style_key}")).absolute(x, y).size(width, m.cell_height).fill(color(bg)));
    }
    let mut fg = color(run.fg);
    if run.faint {
        fg = fg.mix(background, 0.5);
    }
    if run.text.chars().any(|c| c != ' ') {
        let mut style = font();
        if run.bold {
            style.weight = 700.0;
        }
        style.italic = run.italic;
        line = line.child(text(run.text.clone()).key(format!("text:{style_key}")).font(style).color(fg).absolute(x, y));
    }
    if run.underline {
        line = line.child(el().key(format!("under:{style_key}")).absolute(x, y + m.cell_height - 2.0).size(width, 1.0).fill(fg));
    }
    if run.strikethrough {
        line = line.child(el().key(format!("strike:{style_key}")).absolute(x, y + (m.cell_height / 2.0).floor()).size(width, 1.0).fill(fg));
    }
    line
}

fn cursor_elements(cursor: dojo_rex::Cursor, under: Option<&str>, screen: &Screen, m: Metrics) -> Vec<El> {
    let x = m.inset + cursor.x as f32 * m.cell_width;
    let y = m.inset + cursor.y as f32 * m.cell_height;
    let paint = color(cursor.color.unwrap_or(screen.foreground));
    let mut parts = Vec::new();
    match cursor.style {
        CursorStyle::Block => {
            parts.push(el().absolute(x, y).size(m.cell_width, m.cell_height).fill(paint));
            if let Some(glyph) = under.filter(|t| !t.trim().is_empty()) {
                parts.push(text(glyph.to_owned()).font(font()).color(color(screen.background)).absolute(x, y));
            }
        }
        CursorStyle::HollowBlock => {
            parts.push(el().absolute(x, y).size(m.cell_width, m.cell_height).border(1.0, paint));
        }
        CursorStyle::Bar => parts.push(el().absolute(x, y).size(2.0, m.cell_height).fill(paint)),
        CursorStyle::Underline => parts.push(el().absolute(x, y + m.cell_height - 2.0).size(m.cell_width, 2.0).fill(paint)),
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_decode_from_srgb() {
        let white = color(Rgb { r: 255, g: 255, b: 255 });
        assert!((white.r - 1.0).abs() < 1e-5);
        let grey = color(Rgb { r: 128, g: 128, b: 128 });
        assert!((grey.g - 0.2158).abs() < 1e-3);
        assert_eq!(color(Rgb::default()).a, 1.0);
    }
}
