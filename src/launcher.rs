use dojo::application_catalogue as catalogue;
mod input;
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

pub use dojo::control::LauncherNavigation as Navigation;

#[derive(Clone, Debug, Default, PartialEq, gpui::Action)]
#[action(namespace = launcher, no_json)]
pub struct ToggleLauncher {
    pub display_id: Option<gpui::DisplayId>,
}

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
    /// The one launcher window. Dismissing it hides the layer surface instead of destroying
    /// it, so the next opening skips surface, swapchain and renderer creation.
    window: Option<WindowHandle<LauncherView>>,
    /// The output `window` is bound to. A layer surface cannot change output.
    window_display: Option<gpui::DisplayId>,
    icon_cache: Arc<dojo_icon_cache::LinuxIcons>,
}

/// What a toggle did to the existing launcher window.
#[derive(Debug, PartialEq, Eq)]
enum Toggled {
    /// The window began closing, or opened again; the host keeps it.
    Kept,
    /// The window is on the wrong output and is going away; open a new one.
    Replaced,
}

/// What a toggle should do to a launcher window in `phase`, given whether the request names the
/// output the window is bound to.
fn toggle_action(phase: Phase, same_display: bool) -> Action {
    match (phase, same_display) {
        (Phase::Open, true) => Action::Dismiss,
        (Phase::Open, false) => Action::DismissAndRetire,
        (Phase::Closing, true) | (Phase::Hidden, true) => Action::Reopen,
        (Phase::Closing, false) => Action::Retire,
        (Phase::Hidden, false) => Action::Remove,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// Visible and revealed (or revealing).
    Open,
    /// Visible but animating out.
    Closing,
    /// Unmapped and waiting to be shown.
    Hidden,
}

#[derive(Debug, PartialEq, Eq)]
enum Action {
    Dismiss,
    DismissAndRetire,
    Reopen,
    /// Let the closing animation finish, then destroy the window instead of hiding it.
    Retire,
    Remove,
}

impl Launcher {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            worker: SearchWorker::start()?,
            icon_cache: Arc::new(launcher_icon_cache()?),
            window: None,
            window_display: None,
        })
    }

    pub fn toggle(
        &mut self,
        display_id: Option<gpui::DisplayId>,
        cx: &mut App,
    ) -> anyhow::Result<()> {
        let displays = cx.displays();
        let requested_display =
            display_id.and_then(|id| displays.iter().find(|display| display.id() == id).cloned());
        if let Some(handle) = self.window {
            let same_display = display_id.is_none()
                || requested_display.as_ref().map(|display| display.id()) == self.window_display;
            // An error means the compositor closed the window, for example with its output.
            match handle.update(cx, |view, window, cx| view.toggle(same_display, window, cx)) {
                Ok(Toggled::Kept) => return Ok(()),
                Ok(Toggled::Replaced) | Err(_) => self.window = None,
            }
        }
        let worker = self.worker.clone();
        let icon_cache = self.icon_cache.clone();
        let display = requested_display
            .or_else(|| cx.primary_display())
            .or_else(|| displays.into_iter().next())
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
                        hidden: false,
                        retire: false,
                        input_bounds: None,
                        input_line: None,
                        panel_bounds: None,
                        reveal: crate::design::MenuReveal::opening_for(
                            std::time::Instant::now(),
                            std::time::Duration::from_millis(150),
                        ),
                        result_task: None,
                        icon_task: None,
                        icons: HashMap::new(),
                        icon_cache,
                        icon_misses: HashMap::new(),
                        icon_scale: None,
                    };
                    view.listen_for_results(window, cx);
                    view.refresh(window, cx);
                    view
                })
            },
        )?;
        self.window = Some(handle);
        self.window_display = Some(display.id());
        Ok(())
    }

    pub fn navigate(&mut self, navigation: Navigation, cx: &mut App) -> anyhow::Result<()> {
        let Some(handle) = self.window else {
            return Ok(());
        };
        if let Err(error) = handle.update(cx, |view, window, cx| {
            if !view.hidden {
                view.navigate(navigation, window, cx);
            }
        }) {
            // A dismissed layer-shell window can finish closing before its handle is cleared.
            self.window = None;
            eprintln!("Dojo launcher navigation ended with its window: {error}");
        }
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
    /// The layer surface is unmapped after a dismissal and waits for the next opening.
    hidden: bool,
    /// Destroy the window once it has closed rather than hiding it for reuse.
    retire: bool,
    input_bounds: Option<Bounds<Pixels>>,
    input_line: Option<ShapedLine>,
    panel_bounds: Option<Bounds<Pixels>>,
    reveal: crate::design::MenuReveal,
    result_task: Option<gpui::Task<()>>,
    icon_task: Option<gpui::Task<()>>,
    icons: HashMap<usize, Option<gpui::ImageSource>>,
    icon_cache: Arc<dojo_icon_cache::LinuxIcons>,
    icon_misses: HashMap<usize, std::time::Instant>,
    icon_scale: Option<u16>,
}

impl LauncherView {
    fn navigate(&mut self, navigation: Navigation, window: &mut Window, cx: &mut Context<Self>) {
        if self.input.marked.is_some() {
            return;
        }
        match navigation {
            Navigation::Up => {
                self.selected = selection_after(self.selected, self.matches.len(), Navigation::Up);
                self.load_icons(window, cx);
                cx.notify();
            }
            Navigation::Down => {
                self.selected =
                    selection_after(self.selected, self.matches.len(), Navigation::Down);
                self.load_icons(window, cx);
                cx.notify();
            }
            Navigation::Activate => self.launch_selected(window, cx),
            Navigation::Dismiss => self.dismiss(window),
        }
    }

    fn dismiss(&mut self, window: &mut Window) {
        self.reveal.set_open(false, std::time::Instant::now());
        window.refresh();
    }

    fn phase(&self) -> Phase {
        if self.hidden {
            Phase::Hidden
        } else if self.reveal.is_open() {
            Phase::Open
        } else {
            Phase::Closing
        }
    }

    fn toggle(
        &mut self,
        same_display: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Toggled {
        match toggle_action(self.phase(), same_display) {
            Action::Dismiss => {
                self.dismiss(window);
                Toggled::Kept
            }
            Action::DismissAndRetire | Action::Retire => {
                self.retire = true;
                self.dismiss(window);
                Toggled::Replaced
            }
            // If the platform cannot map it again it is gone and the host opens a new one.
            Action::Reopen => {
                if self.reopen(window, cx) {
                    Toggled::Kept
                } else {
                    Toggled::Replaced
                }
            }
            Action::Remove => {
                window.remove_window();
                Toggled::Replaced
            }
        }
    }

    /// Starts a fresh opening in a window that is hidden, or still closing: the search starts
    /// empty and the catalogue is refreshed, exactly as in a window created for the purpose.
    fn reopen(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let now = std::time::Instant::now();
        self.input = SearchInput::default();
        self.matches.clear();
        self.selected = 0;
        self.error = None;
        self.launch = LaunchGate::default();
        self.input_bounds = None;
        self.input_line = None;
        if self.hidden {
            self.panel_bounds = None;
            self.reveal =
                crate::design::MenuReveal::opening_for(now, std::time::Duration::from_millis(150));
            self.hidden = false;
            if !window.show() {
                window.remove_window();
                return false;
            }
        } else {
            self.reveal.set_open(true, now);
        }
        window.focus(&self.focus, cx);
        self.refresh(window, cx);
        true
    }

    /// Ends a dismissal: unmap the window for reuse, or destroy it if it cannot be or must not be.
    fn finish_dismissal(&mut self, window: &mut Window) {
        if self.retire || !window.hide() {
            window.remove_window();
        } else {
            self.hidden = true;
        }
    }

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
                    .update_in(cx, |this, window, cx| match this.worker.results() {
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
        let scale = window.scale_factor().ceil().clamp(1., 4.) as u16;
        if self.icon_scale != Some(scale) {
            self.icons.clear();
            self.icon_misses.clear();
            self.icon_scale = Some(scale);
        }
        self.icon_misses
            .retain(|_, at| at.elapsed() < std::time::Duration::from_secs(300));
        let Some(catalogue) = &self.catalogue else {
            return;
        };
        let requested = self
            .matches
            .iter()
            .skip(self.selected.saturating_sub(9))
            .take(10)
            .filter(|index| {
                !self.icons.contains_key(index) && !self.icon_misses.contains_key(index)
            })
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
        let cache = self.icon_cache.clone();
        let icon_catalogue = catalogue.clone();
        self.icon_task = Some(cx.spawn_in(window, async move |this, cx| {
            let resolved = cx
                .background_executor()
                .spawn(async move {
                    let mut resolved = Vec::with_capacity(requested.len());
                    for (index, name) in requested {
                        let request = dojo_icon_cache::IconRequest {
                            app_id: icon_catalogue.applications[index].id.clone(),
                            icon_name: name,
                            scale,
                        };
                        let asset = cache.get(&request).await.ok().flatten();
                        resolved.push((index, asset));
                    }
                    resolved
                })
                .await;
            if let Err(error) = this.update_in(cx, |this, _, cx| {
                if this
                    .catalogue
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(current, &catalogue))
                {
                    for (index, asset) in resolved {
                        let asset = asset.map(|asset| match asset {
                            dojo_icon_cache::IconAsset::Png(icon) => {
                                gpui::ImageSource::from(Arc::new(gpui::Image::from_bytes(
                                    gpui::ImageFormat::Png,
                                    icon.for_size(24 * u32::from(scale)).to_vec(),
                                )))
                            }
                            dojo_icon_cache::IconAsset::OriginalPath(path) => {
                                gpui::ImageSource::from(path)
                            }
                        });
                        if asset.is_some() {
                            this.icons.insert(index, asset);
                        } else {
                            this.icon_misses.insert(index, std::time::Instant::now());
                        }
                    }
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
                    Ok(()) => this.dismiss(window),
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
            "escape" => self.navigate(Navigation::Dismiss, window, cx),
            "enter" if !event.is_held => self.navigate(Navigation::Activate, window, cx),
            "down" => self.navigate(Navigation::Down, window, cx),
            "up" => self.navigate(Navigation::Up, window, cx),
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

fn selection_after(selected: usize, matches_len: usize, navigation: Navigation) -> usize {
    match navigation {
        Navigation::Up => selected.saturating_sub(1),
        Navigation::Down => selected
            .saturating_add(1)
            .min(matches_len.saturating_sub(1)),
        Navigation::Activate | Navigation::Dismiss => selected,
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
        let (progress, moving) = self
            .reveal
            .sample_with_motion(std::time::Instant::now(), cx.reduce_motion());
        if moving {
            window.request_animation_frame();
        } else if !self.reveal.is_open() && !self.hidden {
            self.finish_dismissal(window);
        }
        let reveal_height = self
            .panel_bounds
            .map_or(100., |bounds| f32::from(bounds.size.height));
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
                let selection = view.input.selection.clone();
                let mut runs = Vec::new();
                if !view.input.text.is_empty() && !selection.is_empty() {
                    for (start, end, selected) in [
                        (0, selection.start, false),
                        (selection.start, selection.end, true),
                        (selection.end, text.len(), false),
                    ] {
                        if end > start {
                            let mut segment = run.clone();
                            segment.len = end - start;
                            if selected {
                                segment.color = colors.selection_foreground.rgb().into();
                            }
                            runs.push(segment);
                        }
                    }
                } else {
                    runs.push(run);
                }
                let line = window.text_system().shape_line(text, px(14.), &runs, None);
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
                let icon: gpui::AnyElement = if let Some(Some(source)) = self.icons.get(&index) {
                    gpui::img(source.clone()).size(px(22.)).into_any_element()
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
                        .text_color(if position == self.selected {
                            colors.selection_foreground.rgb()
                        } else {
                            colors.foreground.rgb()
                        })
                        .hover(|style| {
                            style
                                .bg(colors.background_highlight.rgb())
                                .text_color(colors.selection_foreground.rgb())
                        })
                        .child(icon)
                        .child(div().flex_1().overflow_hidden().child(app.name.clone()))
                        .child(div().text_size(px(11.)).child(app.description.clone()))
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
            .absolute()
            .top(px(0.))
            .left(px(0.))
            .w(px(width))
            .p(px(12.))
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
            .pt(px(crate::geometry::TOP_BAR_HEIGHT as f32))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, window, cx| {
                    if this
                        .panel_bounds
                        .is_some_and(|bounds| !bounds.contains(&event.position))
                    {
                        this.dismiss(window);
                    } else {
                        window.focus(&this.focus, cx);
                    }
                }),
            )
            .child(crate::design::shell_extension(
                gpui::Bounds::new(
                    gpui::point(
                        (window.viewport_size().width - px(width)) / 2.,
                        px(crate::geometry::TOP_BAR_HEIGHT as f32),
                    ),
                    gpui::size(px(width), px(reveal_height * progress)),
                ),
                colors.background.rgb(),
                colors.border.rgb(),
                false,
            ))
            .child(
                div()
                    .relative()
                    .w(px(width))
                    .h(px(reveal_height * progress))
                    .overflow_hidden()
                    .child(panel),
            )
    }
}

#[cfg(test)]
mod launch_tests {
    use super::{Action, LaunchGate, Navigation, Phase, selection_after, toggle_action};

    #[test]
    fn toggling_dismisses_an_open_launcher_and_reopens_a_hidden_one() {
        assert_eq!(toggle_action(Phase::Open, true), Action::Dismiss);
        assert_eq!(toggle_action(Phase::Closing, true), Action::Reopen);
        assert_eq!(toggle_action(Phase::Hidden, true), Action::Reopen);
    }

    #[test]
    fn another_output_retires_the_window_because_a_layer_surface_cannot_move() {
        assert_eq!(toggle_action(Phase::Open, false), Action::DismissAndRetire);
        assert_eq!(toggle_action(Phase::Closing, false), Action::Retire);
        assert_eq!(toggle_action(Phase::Hidden, false), Action::Remove);
    }

    #[test]
    fn launcher_navigation_stays_within_results_and_handles_empty_search() {
        assert_eq!(selection_after(0, 4, Navigation::Up), 0);
        assert_eq!(selection_after(1, 4, Navigation::Up), 0);
        assert_eq!(selection_after(2, 4, Navigation::Down), 3);
        assert_eq!(selection_after(3, 4, Navigation::Down), 3);
        assert_eq!(selection_after(0, 0, Navigation::Down), 0);
        assert_eq!(selection_after(2, 4, Navigation::Activate), 2);
        assert_eq!(selection_after(2, 4, Navigation::Dismiss), 2);
    }

    #[gpui::test]
    fn launcher_opens_on_the_requested_monitor(cx: &mut gpui::TestAppContext) {
        let external = gpui::DisplayId::from(2);
        let second_external = gpui::DisplayId::from(3);
        cx.simulate_display_added(external, None);
        cx.simulate_display_added(second_external, None);
        let mut launcher = super::Launcher::new().unwrap();
        cx.update(|cx| {
            cx.set_global(crate::config::ThemeColors::default());
            launcher.toggle(Some(external), cx).unwrap();
            launcher
                .window
                .unwrap()
                .update(cx, |_, window, cx| {
                    assert_eq!(window.display(cx).unwrap().id(), external);
                })
                .unwrap();
            launcher.toggle(Some(second_external), cx).unwrap();
            launcher
                .window
                .unwrap()
                .update(cx, |_, window, cx| {
                    assert_eq!(window.display(cx).unwrap().id(), second_external);
                })
                .unwrap();
            launcher.toggle(Some(second_external), cx).unwrap();
        });
    }

    #[gpui::test]
    fn dismissed_launcher_is_hidden_and_reopens_with_a_clean_search(cx: &mut gpui::TestAppContext) {
        let display = gpui::DisplayId::from(2);
        cx.simulate_display_added(display, None);
        let mut launcher = super::Launcher::new().unwrap();
        cx.update(|cx| {
            cx.set_global(crate::config::ThemeColors::default());
            launcher.toggle(Some(display), cx).unwrap();
        });
        let handle = launcher.window.unwrap();
        let any_handle: gpui::AnyWindowHandle = handle.into();
        cx.update(|cx| {
            handle
                .update(cx, |view, _, _| {
                    view.input.replace(None, "fire");
                    view.launch.begin();
                })
                .unwrap();
            launcher.toggle(Some(display), cx).unwrap();
        });
        // The dismissal animates for 150 ms and the window hides on the first frame after it.
        std::thread::sleep(std::time::Duration::from_millis(200));
        cx.update(|cx| {
            any_handle
                .update(cx, |_, window, cx| window.draw(cx).clear(cx))
                .unwrap();
        });
        assert!(cx.window_is_hidden(any_handle));
        cx.update(|cx| {
            launcher.toggle(Some(display), cx).unwrap();
            assert_eq!(launcher.window, Some(handle), "the window is reused");
            handle
                .update(cx, |view, _, _| {
                    assert!(!view.hidden);
                    assert!(view.input.text.is_empty());
                    assert!(view.launch.begin(), "a new opening can launch again");
                })
                .unwrap();
        });
        assert!(!cx.window_is_hidden(any_handle));
    }

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

fn launcher_icon_cache() -> anyhow::Result<dojo_icon_cache::LinuxIcons> {
    let mut roots = vec![
        PathBuf::from("/usr/share"),
        PathBuf::from("/nix/store"),
        PathBuf::from("/opt"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        roots.push(PathBuf::from(home));
    }
    if let Some(dirs) = std::env::var_os("XDG_DATA_DIRS") {
        roots.extend(std::env::split_paths(&dirs));
    }
    roots.retain(|path| path.is_dir());
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .map(|root| root.join("dojo/icons"));
    Ok(dojo_icon_cache::LinuxIcons::new(
        cache,
        dojo_icon_cache::Config::default(),
        roots,
    )?)
}
