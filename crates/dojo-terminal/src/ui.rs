//! The window's element tree: panes, separators, the tab bar, the picker.
use rune::{Cx, El, el, text};

use crate::draw::{self, Palette};
use crate::term::{self, Term};
use crate::{picker, tabs};

/// Stand-in length of the probe text, so rounding its width to device pixels barely moves the
/// per-cell advance.
const PROBE_LEN: usize = 100;

pub fn view(s: &Term, cx: &mut Cx<Term>) -> El {
    let scale = cx.env().scale;
    let probe = text("M".repeat(PROBE_LEN))
        .font(draw::font())
        .key("probe")
        .absolute(-20000.0, 0.0)
        .on_layout(move |s: &mut Term, [_, _, w, h]| {
            s.cell = Some((w / PROBE_LEN as f32, h));
            s.scale = scale;
            s.sync();
        });
    let focused = s.focused();
    let palette = Palette::of(focused.as_ref().and_then(|block| s.panes.get(block)).and_then(|pane| pane.screen.as_ref()));
    let metrics = s.metrics();
    let layout = s.layout();
    let multiple = layout.len() > 1;

    let mut root = el()
        .key("terminal")
        .instant_changes()
        .w_full()
        .h_full()
        .fill(palette.bg)
        .on_layout(move |s: &mut Term, [_, _, w, h]| {
            s.area = Some((w, h));
            s.scale = scale;
            s.sync();
        })
        .on_key_up(term::on_key_up);
    let [ax, ay, aw, ah] = s.pane_area();
    if multiple {
        root = root.child(el().key("separator").absolute(ax, ay).size(aw, ah).fill(palette.bg.mix(palette.fg, 0.2)).inert());
    }
    for (block, [x, y, w, h]) in &layout {
        let is_focused = focused.as_ref() == Some(block);
        let body = match s.panes.get(block).and_then(|pane| pane.screen.as_ref().map(|screen| (screen, &pane.pixels))) {
            Some((screen, pixels)) => draw::screen(screen, metrics, block, is_focused, pixels),
            None => el().key(format!("{block}:empty")).w_full().h_full().fill(palette.bg),
        };
        let target = block.clone();
        root = root.child(
            el().key(format!("pane:{block}"))
                .absolute(*x, *y)
                .size(*w, *h)
                // Images reach past their pane's edge; the pane cuts them off.
                .clip_rounded(0.0)
                .focusable(false)
                .on_press(move |s: &mut Term| s.focus_pane(&target))
                .child(body),
        );
        if multiple && is_focused {
            root = root.child(el().key("focus-ring").absolute(*x, *y).size(*w, *h).border(1.0, palette.accent).inert());
        }
    }
    if tabs::visible(&s.view) {
        root = root.child(tabs::bar(&s.view, metrics, s.size().0, &palette));
    }
    root = root.child(probe);
    if let Some(open) = &s.picker {
        let home = std::env::var("HOME").ok();
        root = root.child(picker::view(open, metrics, s.size(), &palette, home.as_deref()));
    }
    root
}
