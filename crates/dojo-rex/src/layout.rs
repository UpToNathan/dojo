//! A session's view as Rex reports it (`session.view`): windows, shown as tabs, each with a
//! tree of splits whose leaves are blocks, shown as panes.
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SessionView {
    pub session_id: String,
    pub label: String,
    pub revision: u64,
    pub active_window: Option<String>,
    pub windows: Vec<WindowView>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WindowView {
    pub id: String,
    pub label: String,
    pub focused_block: Option<String>,
    pub zoomed_block: Option<String>,
    pub layout: Option<Layout>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Layout {
    Block(String),
    Split {
        id: String,
        /// Side by side; otherwise `before` is above `after`.
        horizontal: bool,
        /// The share of the space `before` gets.
        ratio: f32,
        before: Box<Layout>,
        after: Box<Layout>,
    },
}

/// x, y, width, height.
pub type Rect = [f32; 4];

impl Layout {
    fn parse(node: &Value) -> Option<Self> {
        if let Some(block) = node["block_id"].as_str() {
            return Some(Self::Block(block.to_owned()));
        }
        let split = &node["split"];
        Some(Self::Split {
            id: split["split_id"].as_str().unwrap_or_default().to_owned(),
            horizontal: split["direction"].as_str() != Some("vertical"),
            ratio: (split["ratio"].as_f64().unwrap_or(0.5) as f32).clamp(0.05, 0.95),
            before: Box::new(Self::parse(&split["before"])?),
            after: Box::new(Self::parse(&split["after"])?),
        })
    }

    /// Each block's rectangle inside `area`, leaving `gap` between neighbours.
    pub fn rects(&self, area: Rect, gap: f32) -> Vec<(String, Rect)> {
        let mut out = Vec::new();
        self.place(area, gap, &mut out);
        out
    }

    fn place(&self, [x, y, w, h]: Rect, gap: f32, out: &mut Vec<(String, Rect)>) {
        match self {
            Self::Block(block) => out.push((block.clone(), [x, y, w.max(0.0), h.max(0.0)])),
            Self::Split { horizontal: true, ratio, before, after, .. } => {
                let first = ((w - gap) * ratio).round();
                before.place([x, y, first, h], gap, out);
                after.place([x + first + gap, y, w - first - gap, h], gap, out);
            }
            Self::Split { ratio, before, after, .. } => {
                let first = ((h - gap) * ratio).round();
                before.place([x, y, w, first], gap, out);
                after.place([x, y + first + gap, w, h - first - gap], gap, out);
            }
        }
    }

    pub fn blocks(&self) -> Vec<String> {
        self.rects([0.0, 0.0, 1.0, 1.0], 0.0).into_iter().map(|(block, _)| block).collect()
    }
}

impl SessionView {
    pub fn parse(view: &Value) -> Self {
        let windows = view["windows"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|window| WindowView {
                id: window["window_id"].as_str().unwrap_or_default().to_owned(),
                label: window["label"].as_str().unwrap_or_default().to_owned(),
                focused_block: window["focused_block_id"].as_str().map(str::to_owned),
                zoomed_block: window["zoomed_block_id"].as_str().map(str::to_owned),
                // The first layer is the tiled one; floating layers are not drawn yet.
                layout: window["layers"].get(0).and_then(|layer| Layout::parse(&layer["layout"])),
            })
            .collect();
        Self {
            session_id: view["session_id"].as_str().unwrap_or_default().to_owned(),
            label: view["label"].as_str().unwrap_or_default().to_owned(),
            revision: view["revision"].as_u64().unwrap_or(0),
            active_window: view["active_window_id"].as_str().map(str::to_owned),
            windows,
        }
    }

    pub fn active(&self) -> Option<&WindowView> {
        let active = self.active_window.as_deref();
        self.windows.iter().find(|window| Some(window.id.as_str()) == active).or_else(|| self.windows.first())
    }

    /// The panes of the active window: just the zoomed block when one is zoomed.
    pub fn panes(&self, area: Rect, gap: f32) -> Vec<(String, Rect)> {
        let Some(window) = self.active() else { return Vec::new() };
        if let Some(zoomed) = &window.zoomed_block {
            return vec![(zoomed.clone(), area)];
        }
        window.layout.as_ref().map(|layout| layout.rects(area, gap)).unwrap_or_default()
    }

    pub fn focused_block(&self) -> Option<String> {
        let window = self.active()?;
        window.focused_block.clone().or_else(|| window.layout.as_ref()?.blocks().into_iter().next())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn view() -> SessionView {
        SessionView::parse(&json!({
            "session_id": "s", "label": "work", "revision": 4, "active_window_id": "w2",
            "windows": [
                {"window_id": "w1", "label": "one", "layers": [{"layout": {"block_id": "a"}}]},
                {"window_id": "w2", "label": "two", "focused_block_id": "c", "layers": [{"layout": {"split": {
                    "split_id": "x", "direction": "horizontal", "ratio": 0.5,
                    "before": {"block_id": "b"},
                    "after": {"split": {"split_id": "y", "direction": "vertical", "ratio": 0.25,
                        "before": {"block_id": "c"}, "after": {"block_id": "d"}}}}}}]},
            ]
        }))
    }

    #[test]
    fn the_active_window_is_split_into_panes() {
        let view = view();
        assert_eq!(view.windows.len(), 2);
        assert_eq!(view.focused_block().as_deref(), Some("c"));
        let panes = view.panes([0.0, 0.0, 101.0, 81.0], 1.0);
        assert_eq!(
            panes,
            vec![
                ("b".into(), [0.0, 0.0, 50.0, 81.0]),
                ("c".into(), [51.0, 0.0, 50.0, 20.0]),
                ("d".into(), [51.0, 21.0, 50.0, 60.0]),
            ]
        );
    }

    #[test]
    fn a_zoomed_block_fills_the_window() {
        let mut view = view();
        view.windows[1].zoomed_block = Some("d".into());
        assert_eq!(view.panes([0.0, 0.0, 10.0, 10.0], 1.0), vec![("d".into(), [0.0, 0.0, 10.0, 10.0])]);
    }

    #[test]
    fn without_an_active_window_the_first_is_shown() {
        let mut view = view();
        view.active_window = None;
        assert_eq!(view.active().unwrap().id, "w1");
        assert_eq!(view.focused_block().as_deref(), Some("a"));
    }
}
