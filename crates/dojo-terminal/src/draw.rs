//! A screen as Rune elements. Every run is placed at its own column, so text never drifts
//! from the grid whatever the font's advance is.
use std::collections::HashMap;

use dojo_rex::{CursorStyle, ImageLayer, Rgb, Screen};
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

/// A screen's image placements in `layers`, at their cells, sized from device pixels.
fn images(screen: &Screen, m: Metrics, ns: &str, pixels: &HashMap<String, rune::Pixels>, layers: impl Fn(ImageLayer) -> bool) -> Vec<El> {
    // Image pixels map through the cell: the terminal's cells are whole device pixels
    // (`grid::cell_px`), so this keeps pieces that tile a picture edge to edge on the grid.
    let per_x = m.cell_width / grid::cell_px(m.cell_width, m.scale) as f32;
    let per_y = m.cell_height / grid::cell_px(m.cell_height, m.scale) as f32;
    screen
        .images
        .iter()
        .enumerate()
        .filter(|(_, image)| layers(image.layer))
        .filter_map(|(index, image)| {
            let pixels = pixels.get(&image.key)?;
            let x = m.inset + image.col as f32 * m.cell_width + image.offset_x as f32 * per_x;
            let y = m.inset + image.row as f32 * m.cell_height + image.offset_y as f32 * per_y;
            Some(
                cell(el())
                    .key(format!("{ns}:image:{index}:{}", image.key))
                    .absolute(x, y)
                    .size(image.draw_width as f32 * per_x, image.draw_height as f32 * per_y)
                    .pixels(pixels)
                    .inert(),
            )
        })
        .collect()
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
    /// Device pixels per logical pixel; image sizes and offsets come in device pixels.
    pub scale: f32,
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

    /// The focused pane's colours, or the configured theme's while no pane is showing (the
    /// picker before a session opens), so the window is themed either way.
    pub fn of(screen: Option<&Screen>) -> Self {
        let theme = THEME_COLORS.get().copied().flatten();
        let mut palette = match (screen, theme) {
            (Some(screen), _) => Self::new(screen.background, screen.foreground, screen.cursor.and_then(|c| c.color)),
            (None, Some(theme)) => Self::new(theme.background, theme.foreground, theme.cursor),
            (None, None) => Self::new(Rgb { r: 0x14, g: 0x14, b: 0x18 }, Rgb { r: 0xdd, g: 0xdd, b: 0xdd }, None),
        };
        if let Some(warning) = theme.and_then(|theme| theme.warning) {
            palette.warning = color(warning);
        }
        palette
    }
}

/// Colours read from the theme file at start (`settings::theme_colors`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThemeColors {
    pub background: Rgb,
    pub foreground: Rgb,
    pub cursor: Option<Rgb>,
    /// The palette's yellow, for status marks.
    pub warning: Option<Rgb>,
}

pub static THEME_COLORS: OnceLock<Option<ThemeColors>> = OnceLock::new();

impl ThemeColors {
    /// From the `set_theme` payload `settings::theme_payload` builds.
    pub fn from_payload(theme: &serde_json::Value) -> Option<Self> {
        let rgb = |value: &serde_json::Value| value.as_str().and_then(Rgb::parse);
        let palette = |index: &str| {
            theme["palette"].as_array()?.iter().filter_map(|entry| entry.as_str()?.split_once('=')).find(|(i, _)| *i == index).and_then(|(_, c)| Rgb::parse(c))
        };
        Some(Self {
            background: rgb(&theme["background"])?,
            foreground: rgb(&theme["foreground"])?,
            cursor: rgb(&theme["cursor"]),
            warning: palette("3"),
        })
    }
}

pub fn screen(screen: &Screen, metrics: Metrics, ns: &str, show_cursor: bool, pixels: &HashMap<String, rune::Pixels>) -> El {
    let background = color(screen.background);
    let mut root = cell(el().w_full().h_full().fill(background)).instant_changes();
    // Ghostty's layers: images below the backgrounds, cell backgrounds, images below the text,
    // the text, then images above it.
    root = root.children(images(screen, metrics, ns, pixels, |layer| layer == ImageLayer::BelowBackground));
    let rows: Vec<(usize, f32, Vec<Run>)> = screen
        .cells
        .iter()
        .enumerate()
        .map(|(row, cells)| (row, metrics.inset + row as f32 * metrics.cell_height, grid::runs(cells)))
        .filter(|(_, _, runs)| !runs.is_empty())
        .collect();
    for (row, y, runs) in &rows {
        for run in runs {
            root = run_background(root, run, ns, *row, *y, metrics);
        }
    }
    root = root.children(images(screen, metrics, ns, pixels, |layer| layer == ImageLayer::BelowText));
    for (row, y, runs) in &rows {
        for run in runs {
            root = run_elements(root, run, ns, *row, *y, metrics, background);
        }
    }
    root = root.children(images(screen, metrics, ns, pixels, |layer| layer == ImageLayer::AboveText));
    if let Some(cursor) = screen.cursor.filter(|_| show_cursor) {
        let under = screen.cells.get(cursor.y as usize).and_then(|row| row.get(cursor.x as usize));
        root = root.children(cursor_elements(cursor, under.map(|c| c.text.as_str()), screen, metrics, ns));
    }
    root
}

/// Every element is keyed by its place and style: Rune reuses unkeyed children by position and
/// would keep an old run's colour on new text.
fn style_key(run: &Run, ns: &str, row: usize) -> String {
    format!(
        "{ns}:{row}:{}:{:?}:{:?}:{}{}{}{}{}:{}",
        run.col, run.fg, run.bg, run.bold as u8, run.italic as u8, run.faint as u8, run.underline as u8, run.strikethrough as u8, run.cols
    )
}

fn run_background(line: El, run: &Run, ns: &str, row: usize, y: f32, m: Metrics) -> El {
    let Some(bg) = run.bg else { return line };
    let x = m.inset + run.col as f32 * m.cell_width;
    let width = run.cols as f32 * m.cell_width;
    line.child(cell(el()).key(format!("bg:{}", style_key(run, ns, row))).absolute(x, y).size(width, m.cell_height).fill(color(bg)))
}

/// A run's text and its underline and strikethrough; its background is `run_background`.
fn run_elements(mut line: El, run: &Run, ns: &str, row: usize, y: f32, m: Metrics, background: Color) -> El {
    let style_key = style_key(run, ns, row);
    let x = m.inset + run.col as f32 * m.cell_width;
    let width = run.cols as f32 * m.cell_width;
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
    #[test]
    fn theme_colours_come_from_the_payload() {
        let theme = crate::settings::theme_payload("background = #24283b\nforeground = #c0caf5\ncursor-color = #7aa2f7\npalette = 3=#e0af68\n");
        let colours = ThemeColors::from_payload(&theme).unwrap();
        assert_eq!(colours.background, Rgb { r: 0x24, g: 0x28, b: 0x3b });
        assert_eq!(colours.cursor, Some(Rgb { r: 0x7a, g: 0xa2, b: 0xf7 }));
        assert_eq!(colours.warning, Some(Rgb { r: 0xe0, g: 0xaf, b: 0x68 }));
        assert!(ThemeColors::from_payload(&serde_json::json!({})).is_none());
    }

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
