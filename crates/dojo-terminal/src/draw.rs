//! A screen as Rune elements. Every run is placed at its own column, so text never drifts
//! from the grid whatever the font's advance is.
use dojo_rex::{CursorStyle, Rgb, Screen};
use std::sync::OnceLock;

use rune::{Color, El, TextStyle, Transition, el, text};

use crate::grid::{self, Run};

/// The configured font (`settings`), set once at start.
pub static FONT: OnceLock<TextStyle> = OnceLock::new();
/// The gap around the grid in window mode.
pub const WINDOW_INSET: f32 = 8.0;

/// A terminal surface never fades: elements appear and go in the frame that changes them.
fn cell(element: El) -> El {
    element.transition(Transition::identity())
}

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

/// `ns` keeps the elements of one pane from sharing Rune keys with another's.
/// The colours of the window's own parts (tab bar, picker, separators), taken from the theme
/// the focused pane shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub bg: Color,
    pub fg: Color,
    pub dim: Color,
    /// Secondary text.
    pub muted: Color,
    pub accent: Color,
    pub warning: Color,
    pub bar: Color,
    pub panel: Color,
    pub selected: Color,
}

impl Palette {
    pub fn new(background: Rgb, foreground: Rgb, cursor: Option<Rgb>) -> Self {
        let (bg, fg) = (color(background), color(foreground));
        Self {
            bg,
            fg,
            dim: bg.mix(fg, 0.3),
            muted: bg.mix(fg, 0.65),
            accent: color(cursor.unwrap_or(foreground)),
            warning: Color { r: 1.0, g: 0.45, b: 0.1, a: 1.0 },
            bar: bg.mix(fg, 0.07),
            panel: bg.mix(fg, 0.1),
            selected: bg.mix(fg, 0.25),
        }
    }

    pub fn of(screen: Option<&Screen>) -> Self {
        match screen {
            Some(screen) => Self::new(screen.background, screen.foreground, screen.cursor.and_then(|c| c.color)),
            None => Self::new(Rgb { r: 0x14, g: 0x14, b: 0x18 }, Rgb { r: 0xdd, g: 0xdd, b: 0xdd }, None),
        }
    }
}

pub fn screen(screen: &Screen, metrics: Metrics, ns: &str, show_cursor: bool) -> El {
    let background = color(screen.background);
    let mut root = cell(el().w_full().h_full().fill(background)).instant_changes();
    for (row, cells) in screen.cells.iter().enumerate() {
        let runs = grid::runs(cells);
        if runs.is_empty() {
            continue;
        }
        let y = metrics.inset + row as f32 * metrics.cell_height;
        for run in &runs {
            root = run_elements(root, run, ns, row, y, metrics, background);
        }
    }
    if let Some(cursor) = screen.cursor.filter(|_| show_cursor) {
        let under = screen.cells.get(cursor.y as usize).and_then(|row| row.get(cursor.x as usize));
        root = root.children(cursor_elements(cursor, under.map(|c| c.text.as_str()), screen, metrics, ns));
    }
    root
}

/// Every element is keyed by its place and style: Rune reuses unkeyed children by position and
/// would keep an old run's colour on new text.
fn run_elements(mut line: El, run: &Run, ns: &str, row: usize, y: f32, m: Metrics, background: Color) -> El {
    let style_key = format!(
        "{ns}:{row}:{}:{:?}:{:?}:{}{}{}{}{}:{}",
        run.col, run.fg, run.bg, run.bold as u8, run.italic as u8, run.faint as u8, run.underline as u8, run.strikethrough as u8, run.cols
    );
    let x = m.inset + run.col as f32 * m.cell_width;
    let width = run.cols as f32 * m.cell_width;
    if let Some(bg) = run.bg {
        line = line.child(cell(el()).key(format!("bg:{style_key}")).absolute(x, y).size(width, m.cell_height).fill(color(bg)));
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
        line = line.child(cell(text(run.text.clone())).key(format!("text:{style_key}")).font(style).color(fg).absolute(x, y));
    }
    if run.underline {
        line = line.child(cell(el()).key(format!("under:{style_key}")).absolute(x, y + m.cell_height - 2.0).size(width, 1.0).fill(fg));
    }
    if run.strikethrough {
        line = line.child(cell(el()).key(format!("strike:{style_key}")).absolute(x, y + (m.cell_height / 2.0).floor()).size(width, 1.0).fill(fg));
    }
    line
}

fn cursor_elements(cursor: dojo_rex::Cursor, under: Option<&str>, screen: &Screen, m: Metrics, ns: &str) -> Vec<El> {
    let x = m.inset + cursor.x as f32 * m.cell_width;
    let y = m.inset + cursor.y as f32 * m.cell_height;
    let paint = color(cursor.color.unwrap_or(screen.foreground));
    let mut parts = Vec::new();
    match cursor.style {
        CursorStyle::Block => {
            parts.push(cell(el()).key(format!("{ns}:cursor")).absolute(x, y).size(m.cell_width, m.cell_height).fill(paint));
            if let Some(glyph) = under.filter(|t| !t.trim().is_empty()) {
                parts.push(cell(text(glyph.to_owned())).key(format!("{ns}:cursor-glyph")).font(font()).color(color(screen.background)).absolute(x, y));
            }
        }
        CursorStyle::HollowBlock => {
            parts.push(cell(el()).key(format!("{ns}:cursor")).absolute(x, y).size(m.cell_width, m.cell_height).border(1.0, paint));
        }
        CursorStyle::Bar => parts.push(cell(el()).key(format!("{ns}:cursor")).absolute(x, y).size(2.0, m.cell_height).fill(paint)),
        CursorStyle::Underline => parts.push(cell(el()).key(format!("{ns}:cursor")).absolute(x, y + m.cell_height - 2.0).size(m.cell_width, 2.0).fill(paint)),
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
