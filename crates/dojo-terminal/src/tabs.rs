//! The tab bar: shown when the session has more than one window.
use dojo_rex::{SessionView, WindowView};
use rune::{El, el, text};

use crate::draw::{self, Metrics, Palette};
use crate::term::Term;

/// Rex names a window it was not told about "Window xxxx": those show as their number.
pub fn labels(windows: &[WindowView]) -> Vec<String> {
    windows
        .iter()
        .enumerate()
        .map(|(i, window)| if window.label.is_empty() || window.label.starts_with("Window ") { (i + 1).to_string() } else { window.label.clone() })
        .collect()
}

pub fn height(m: Metrics) -> f32 {
    m.cell_height + 8.0
}

pub fn visible(view: &SessionView) -> bool {
    view.windows.len() > 1
}

pub fn bar(view: &SessionView, m: Metrics, width: f32, palette: &Palette) -> El {
    let active = view.active().map(|window| window.id.as_str());
    let mut bar = el().key("tabs").absolute(0.0, 0.0).size(width, height(m)).fill(palette.bar);
    let mut x = 0.0;
    for (window, label) in view.windows.iter().zip(labels(&view.windows)) {
        let w = (label.chars().count() as f32 + 3.0) * m.cell_width;
        let is_active = Some(window.id.as_str()) == active;
        let id = window.id.clone();
        let tab = el()
            .key(format!("tab:{}", window.id))
            .absolute(x, 0.0)
            .size(w, height(m))
            .fill(if is_active { palette.selected } else { palette.bar })
            .focusable(false)
            .on_press(move |s: &mut Term| s.focus_tab(&id))
            .child(
                text(label)
                    .key(format!("tab-label:{}", window.id))
                    .font(draw::font())
                    .color(if is_active { palette.fg } else { palette.dim.mix(palette.fg, 0.4) })
                    .absolute(1.5 * m.cell_width, 4.0),
            );
        bar = bar.child(tab);
        x += w;
    }
    bar
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(label: &str) -> WindowView {
        WindowView { id: label.into(), label: label.into(), focused_block: None, zoomed_block: None, layout: None }
    }

    #[test]
    fn automatic_labels_become_numbers() {
        let windows = [window("Window 3fa9"), window("logs"), window("Window 71c2")];
        assert_eq!(labels(&windows), ["1", "logs", "3"]);
    }
}
