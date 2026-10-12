mod catalogue;
mod icons;
mod input;
use dojo::application_launch as launch;
mod worker;

use std::collections::HashMap;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

use gpui::{
    App, Bounds, Context, ElementInputHandler, EntityInputHandler, FocusHandle, KeyDownEvent,
    MouseButton, Pixels, Point, ShapedLine, SharedString, UTF16Selection, Window,
    WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowKind, WindowOptions, div, fill,
    layer_shell, point, prelude::*, px, size,
};

use crate::config::ThemeColors;
use catalogue::Catalogue;
use input::SearchInput;
use worker::SearchWorker;

gpui::actions!(launcher, [ToggleLauncher]);

#[derive(Default)]
struct LaunchGate {
    busy: bool,
}

impl LaunchGate {
    fn begin(&mut self) -> bool {
        if self.busy {
            false
        } else {
            self.busy = true;
            true
        }
    }
    fn complete(&mut self, succeeded: bool) {
        // Successful launch remains latched until this launcher window is gone.
        // Releasing it before removal lets queued input launch the same app again.
        if !succeeded {
            self.busy = false;
        }
    }
}

pub fn benchmark() -> anyhow::Result<()> {
    let started = std::time::Instant::now();
    let catalogue = Catalogue::discover()?;
    let discovery_us = started.elapsed().as_micros();
    let mut matcher = nucleo_matcher::Matcher::new(nucleo_matcher::Config::DEFAULT);
    let mut timings = Vec::new();
    for query in [
        "",
        "a",
        "fire",
        "ffx",
        "term",
        "code",
        "settings",
        "not-present",
    ] {
        for _ in 0..20 {
            catalogue.search(query, &mut matcher);
        }
        let mut samples = Vec::with_capacity(1000);
        for _ in 0..1000 {
            let start = std::time::Instant::now();
            std::hint::black_box(catalogue.search(std::hint::black_box(query), &mut matcher));
            samples.push(start.elapsed().as_nanos());
        }
        samples.sort_unstable();
        timings.push(serde_json::json!({
            "query": query,
            "p50_ns": samples[500], "p95_ns": samples[950], "p99_ns": samples[990],
            "matches": catalogue.search(query, &mut matcher).iter().take(5).map(|&index| &catalogue.applications[index].name).collect::<Vec<_>>(),
        }));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "application_count": catalogue.applications.len(),
            "discovery_us": discovery_us,
            "matcher": "nucleo-matcher 0.3; reusable worker scratch; no UI/IPC timing",
            "samples_per_query": 1000,
            "queries": timings,
        }))?
    );
    Ok(())
}

pub struct Launcher {
    worker: SearchWorker,
    window: Option<WindowHandle<LauncherView>>,
}

impl Launcher {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            worker: SearchWorker::start()?,
            window: None,
        })
    }

    pub fn toggle(&mut self, cx: &mut App) -> anyhow::Result<()> {
        if let Some(handle) = self.window.take() {
            if handle
                .update(cx, |_, window, _| window.remove_window())
                .is_ok()
            {
                return Ok(());
            }
        }
        let worker = self.worker.clone();
        let display = cx
            .primary_display()
            .or_else(|| cx.displays().into_iter().next())
            .ok_or_else(|| anyhow::anyhow!("no output available for launcher"))?;
        let handle = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(display.bounds())),
                display_id: Some(display.id()),
                titlebar: None,
                app_id: Some("dojo".into()),
                window_background: WindowBackgroundAppearance::Transparent,
                kind: WindowKind::LayerShell(layer_shell::LayerShellOptions {
                    namespace: "dojo-launcher".into(),
                    layer: layer_shell::Layer::Overlay,
                    anchor: layer_shell::Anchor::TOP
                        | layer_shell::Anchor::BOTTOM
                        | layer_shell::Anchor::LEFT
                        | layer_shell::Anchor::RIGHT,
                    exclusive_zone: Some(px(-1.)),
                    keyboard_interactivity: layer_shell::KeyboardInteractivity::Exclusive,
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| {
                cx.new(|cx| {
                    let focus = cx.focus_handle();
                    window.focus(&focus, cx);
                    let mut view = LauncherView {
                        focus,
                        input: SearchInput::default(),
                        worker,
                        catalogue: None,
                        matches: Vec::new(),
                        selected: 0,
                        generation: 0,
                        ready_generation: None,
                        error: None,
                        launch: LaunchGate::default(),
                        input_bounds: None,
                        input_line: None,
                        panel_bounds: None,
                        result_task: None,
                        icon_task: None,
                        icons: HashMap::new(),
                    };
                    view.listen_for_results(window, cx);
                    view.refresh(window, cx);
                    view
                })
            },
        )?;
        self.window = Some(handle);
        Ok(())
    }
}

impl Drop for Launcher {
    fn drop(&mut self) {
        self.worker.stop();
    }
}

struct LauncherView {
    focus: FocusHandle,
    input: SearchInput,
    worker: SearchWorker,
    catalogue: Option<Arc<Catalogue>>,
    matches: Vec<usize>,
    selected: usize,
    generation: u64,
    ready_generation: Option<u64>,
    error: Option<String>,
    launch: LaunchGate,
    input_bounds: Option<Bounds<Pixels>>,
    input_line: Option<ShapedLine>,
    panel_bounds: Option<Bounds<Pixels>>,
    result_task: Option<gpui::Task<()>>,
    icon_task: Option<gpui::Task<()>>,
    icons: HashMap<usize, Option<PathBuf>>,
}

impl LauncherView {
    fn refresh(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.ready_generation = None;
        self.selected = 0;
        self.error = None;
        match self.worker.refresh(self.input.text.clone()) {
            Ok(generation) => self.generation = generation,
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }

    fn search(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.ready_generation = None;
        self.selected = 0;
        self.error = None;
        match self.worker.query(self.input.text.clone()) {
            Ok(generation) => self.generation = generation,
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }

    fn listen_for_results(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let worker = self.worker.clone();
        self.result_task = Some(cx.spawn_in(window, async move |this, cx| {
            while worker.wait().await {
                if this
                    .update_in(cx, |this, window, cx| match this.worker.take_results() {
                        Ok(Some(results)) if results.generation == this.generation => {
                            if this
                                .catalogue
                                .as_ref()
                                .is_none_or(|old| !Arc::ptr_eq(old, &results.catalogue))
                            {
                                this.icon_task = None;
                                this.icons.clear();
                            }
                            this.catalogue = Some(results.catalogue);
                            this.matches = results.matches;
                            this.ready_generation = Some(this.generation);
                            this.load_icons(window, cx);
                            cx.notify();
                        }
                        Err(error) => {
                            this.error = Some(error);
                            cx.notify();
                        }
                        _ => {}
                    })
                    .is_err()
                {
                    return;
                }
            }
        }));
    }

    fn load_icons(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(catalogue) = &self.catalogue else {
            return;
        };
        let requested = self
            .matches
            .iter()
            .skip(self.selected.saturating_sub(9))
            .take(10)
            .filter(|index| !self.icons.contains_key(index))
            .filter_map(|&index| {
                catalogue
                    .applications
                    .get(index)
                    .and_then(|app| app.icon.clone().map(|name| (index, name)))
            })
            .collect::<Vec<_>>();
        if requested.is_empty() {
            return;
        }
        let catalogue = catalogue.clone();
        let scale = window.scale_factor().ceil().clamp(1., 4.) as u16;
        self.icon_task = Some(cx.spawn_in(window, async move |this, cx| {
            let resolved = cx
                .background_executor()
                .spawn(async move {
                    requested
                        .into_iter()
                        .map(|(index, name)| (index, icons::resolve(&name, scale)))
                        .collect::<Vec<_>>()
                })
                .await;
            if let Err(error) = this.update_in(cx, |this, _, cx| {
                if this
                    .catalogue
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(current, &catalogue))
                {
                    this.icons.extend(resolved);
                }
                cx.notify();
            }) {
                eprintln!("Dojo launcher closed during icon lookup: {error}");
            }
        }));
    }

    fn launch_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.launch.busy || self.ready_generation != Some(self.generation) {
            return;
        }
        let Some(index) = self.matches.get(self.selected).copied() else {
            return;
        };
        let Some(catalogue) = self.catalogue.clone() else {
            return;
        };
        if !self.launch.begin() {
            return;
        }
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let application = catalogue
                        .applications
                        .get(index)
                        .ok_or_else(|| anyhow::anyhow!("selected application disappeared"))?;
                    catalogue::launch(application)
                })
                .await;
            if let Err(error) = this.update_in(cx, |this, window, cx| {
                this.launch.complete(result.is_ok());
                match result {
                    Ok(()) => window.remove_window(),
                    Err(error) => {
                        this.error = Some(format!("{error:#}"));
                        cx.notify();
                    }
                }
            }) {
                eprintln!("Dojo launcher closed during launch: {error}");
            }
        })
        .detach();
        cx.notify();
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.input.marked.is_some() {
            return;
        }
        let modifiers = event.keystroke.modifiers;
        match event.keystroke.key.as_str() {
            "escape" => window.remove_window(),
            "enter" if !event.is_held => self.launch_selected(window, cx),
            "down" => {
                self.selected = (self.selected + 1).min(self.matches.len().saturating_sub(1));
                self.load_icons(window, cx);
                cx.notify();
            }
            "up" => {
                self.selected = self.selected.saturating_sub(1);
                self.load_icons(window, cx);
                cx.notify();
            }
            "backspace" => {
                self.input.backspace();
                self.search(window, cx);
            }
            "delete" => {
                self.input.delete();
                self.search(window, cx);
            }
            "left" => {
                self.input.move_cursor(false);
                cx.notify();
            }
            "right" => {
                self.input.move_cursor(true);
                cx.notify();
            }
            "home" => {
                self.input.selection = 0..0;
                cx.notify();
            }
            "end" => {
                let end = self.input.text.len();
                self.input.selection = end..end;
                cx.notify();
            }
            "a" if modifiers.control => {
                self.input.selection = 0..self.input.text.len();
                cx.notify();
            }
            "v" if modifiers.control => {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    self.replace_text_in_range(None, &text.replace(['\n', '\r'], " "), window, cx);
                }
            }
            "c" | "x" if modifiers.control => {
                if !self.input.selection.is_empty() {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                        self.input.text[self.input.selection.clone()].to_string(),
                    ));
                    if event.keystroke.key == "x" {
                        self.input.replace(None, "");
                        self.search(window, cx);
                    }
                }
            }
            _ => return,
        }
        window.prevent_default();
        cx.stop_propagation();
    }
}

impl EntityInputHandler for LauncherView {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.input.from_utf16(range);
        *actual = Some(self.input.to_utf16(range.clone()));
        Some(self.input.text[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.input.to_utf16(self.input.selection.clone()),
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.input
            .marked
            .clone()
            .map(|range| self.input.to_utf16(range))
    }

    fn unmark_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.marked = None;
        self.search(window, cx);
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.input.replace(range, &text.replace(['\n', '\r'], " "));
        self.search(window, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selection: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let inserted = self.input.replace(range, text);
        if !text.is_empty() {
            self.input.marked = Some(inserted.clone());
        }
        if let Some(selection) = selection {
            let base = self.input.utf16_offset(inserted.start);
            self.input.selection = self
                .input
                .from_utf16(base + selection.start..base + selection.end);
        }
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = self.input.from_utf16(range);
        let line = self.input_line.as_ref()?;
        Some(Bounds::from_corners(
            point(bounds.left() + line.x_for_index(range.start), bounds.top()),
            point(bounds.left() + line.x_for_index(range.end), bounds.bottom()),
        ))
    }

    fn character_index_for_point(
        &mut self,
        position: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let bounds = self.input_bounds?;
        if !bounds.contains(&position) {
            return None;
        }
        Some(
            self.input.utf16_offset(
                self.input_line
                    .as_ref()?
                    .closest_index_for_x(position.x - bounds.left()),
            ),
        )
    }
}

impl Render for LauncherView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = *cx.global::<ThemeColors>();
        let width = (f32::from(window.viewport_size().width) - 48.).clamp(1., 520.);
        let input_entity = cx.entity();
        let panel_entity = cx.entity();
        let input = gpui::canvas(
            |_, _, _| (),
            move |bounds, (), window, cx| {
                let view = input_entity.read(cx);
                let focus = view.focus.clone();
                let text: SharedString = if view.input.text.is_empty() {
                    "Search applications…".into()
                } else {
                    view.input.text.clone().into()
                };
                let run = gpui::TextRun {
                    len: text.len(),
                    font: window.text_style().font(),
                    color: if view.input.text.is_empty() {
                        colors.foreground_muted.rgb().into()
                    } else {
                        colors.foreground.rgb().into()
                    },
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let line = window.text_system().shape_line(text, px(14.), &[run], None);
                let selection = view.input.selection.clone();
                let empty = view.input.text.is_empty();
                window.handle_input(
                    &focus,
                    ElementInputHandler::new(bounds, input_entity.clone()),
                    cx,
                );
                if !empty && !selection.is_empty() {
                    window.paint_quad(fill(
                        Bounds::from_corners(
                            point(
                                bounds.left() + line.x_for_index(selection.start),
                                bounds.top(),
                            ),
                            point(
                                bounds.left() + line.x_for_index(selection.end),
                                bounds.bottom(),
                            ),
                        ),
                        colors.background_highlight.rgb(),
                    ));
                }
                if let Err(error) = line.paint(
                    bounds.origin,
                    px(24.),
                    gpui::TextAlign::Left,
                    None,
                    window,
                    cx,
                ) {
                    eprintln!("Dojo could not paint launcher search: {error}");
                }
                let cursor = if empty {
                    px(0.)
                } else {
                    line.x_for_index(selection.end)
                };
                if focus.is_focused(window) && selection.is_empty() {
                    window.paint_quad(fill(
                        Bounds::new(
                            point(bounds.left() + cursor, bounds.top() + px(3.)),
                            size(px(2.), px(16.)),
                        ),
                        colors.foreground.rgb(),
                    ));
                }
                input_entity.update(cx, |view, _| {
                    view.input_line = Some(line);
                    view.input_bounds = Some(bounds);
                });
            },
        )
        .w_full()
        .h(px(24.));

        let start = self.selected.saturating_sub(9);
        let mut rows = Vec::new();
        if let Some(catalogue) = &self.catalogue {
            for (position, &index) in self.matches.iter().enumerate().skip(start).take(10) {
                let Some(app) = catalogue.applications.get(index) else {
                    continue;
                };
                let icon: gpui::AnyElement = if let Some(Some(path)) = self.icons.get(&index) {
                    gpui::img(path.clone()).size(px(22.)).into_any_element()
                } else {
                    div()
                        .size(px(22.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(colors.foreground_muted.rgb())
                        .child("◇")
                        .into_any_element()
                };
                rows.push(
                    div()
                        .id(("launcher-result", position))
                        .aria_label(format!("Launch {} ({})", app.name, app.id))
                        .h(px(34.))
                        .px(px(8.))
                        .rounded(px(17.))
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .bg(if position == self.selected {
                            colors.background_highlight.rgb()
                        } else {
                            colors.background.rgb()
                        })
                        .hover(|style| style.bg(colors.background_highlight.rgb()))
                        .child(icon)
                        .child(div().flex_1().overflow_hidden().child(app.name.clone()))
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(colors.foreground_muted.rgb())
                                .child(app.description.clone()),
                        )
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, window, cx| {
                                this.selected = position;
                                this.launch_selected(window, cx);
                            }),
                        ),
                );
            }
        }
        let status = if let Some(error) = &self.error {
            error.clone()
        } else if self.launch.busy {
            "Launching…".into()
        } else if self.ready_generation != Some(self.generation) {
            "Searching…".into()
        } else if self.matches.is_empty() {
            "No applications found".into()
        } else {
            "↑ ↓ select · Enter launch · Esc close".into()
        };
        let panel = div()
            .id("launcher-panel")
            .relative()
            .w(px(width))
            .p(px(12.))
            .rounded(px(12.))
            .bg(colors.background.rgb())
            .border_1()
            .border_color(colors.border.rgb().opacity(crate::design::BORDER_OPACITY))
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(div().pb(px(4.)).child(input))
            .child(div().flex().flex_col().children(rows))
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(colors.foreground_muted.rgb())
                    .child(status),
            )
            .child(
                gpui::canvas(
                    move |bounds, _, cx| {
                        panel_entity.update(cx, |this, _| {
                            this.panel_bounds = Some(bounds);
                        });
                    },
                    |_, (), _, _| (),
                )
                .absolute()
                .top(px(0.))
                .left(px(0.))
                .size_full(),
            );
        div()
            .size_full()
            .bg({
                let mut scrim = colors.background.rgb();
                scrim.a = 48. / 255.;
                scrim
            })
            .font_family(crate::power::FONT_FAMILY)
            .font_features(crate::power::font_features())
            .text_size(px(13.))
            .text_color(colors.foreground.rgb())
            .flex()
            .items_start()
            .justify_center()
            .pt(px(70.))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, window, cx| {
                    if this
                        .panel_bounds
                        .is_some_and(|bounds| !bounds.contains(&event.position))
                    {
                        window.remove_window();
                    } else {
                        window.focus(&this.focus, cx);
                    }
                }),
            )
            .child(panel)
    }
}

#[cfg(test)]
mod launch_tests {
    use super::LaunchGate;

    #[test]
    fn queued_input_cannot_launch_twice_after_success() {
        let mut gate = LaunchGate::default();
        assert!(gate.begin());
        assert!(!gate.begin());
        gate.complete(true);
        assert!(!gate.begin());
    }

    #[test]
    fn failed_launch_allows_an_explicit_retry() {
        let mut gate = LaunchGate::default();
        assert!(gate.begin());
        gate.complete(false);
        assert!(gate.begin());
    }
}
