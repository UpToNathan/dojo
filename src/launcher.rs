use dojo::application_catalogue as catalogue;
mod input;
mod views;
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
use dojo::launcher_core::commands::{self, CommandId, HiddenCommands};
use dojo::launcher_core::rows::{Row, RowAction, RowIcon, Step, first_selectable, step_selection};
use input::SearchInput;
use views::{Host, Notify, Outcome, View, ViewFactory};
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
    views: ViewFactory,
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
            views: views::default_factory(),
        })
    }

    pub fn toggle(
        &mut self,
        display_id: Option<gpui::DisplayId>,
        cx: &mut App,
    ) -> anyhow::Result<()> {
        self.show(display_id, None, cx)
    }

    /// Opens the launcher straight into a command's view, and keeps it open if it already is.
    pub fn open_view(
        &mut self,
        display_id: Option<gpui::DisplayId>,
        command: CommandId,
        cx: &mut App,
    ) -> anyhow::Result<()> {
        self.show(display_id, Some(command), cx)
    }

    #[cfg(test)]
    fn with_views(mut self, views: ViewFactory) -> Self {
        self.views = views;
        self
    }

    fn show(
        &mut self,
        display_id: Option<gpui::DisplayId>,
        initial: Option<CommandId>,
        cx: &mut App,
    ) -> anyhow::Result<()> {
        let displays = cx.displays();
        let requested_display =
            display_id.and_then(|id| displays.iter().find(|display| display.id() == id).cloned());
        if let Some(handle) = self.window {
            let same_display = display_id.is_none()
                || requested_display.as_ref().map(|display| display.id()) == self.window_display;
            // An error means the compositor closed the window, for example with its output.
            match handle.update(cx, |view, window, cx| {
                view.toggle(same_display, initial, window, cx)
            }) {
                Ok(Toggled::Kept) => return Ok(()),
                Ok(Toggled::Replaced) | Err(_) => self.window = None,
            }
        }
        let worker = self.worker.clone();
        let icon_cache = self.icon_cache.clone();
        let views = self.views.clone();
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
                        rows: Vec::new(),
                        stack: Vec::new(),
                        views,
                        status: None,
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
                    if let Some(command) = initial {
                        view.open_command(command, window, cx);
                    }
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
    /// What the list shows: the root list, or the open view's rows.
    rows: Vec<Row>,
    /// Open views, outermost first. Empty means the root list.
    stack: Vec<OpenView>,
    views: ViewFactory,
    /// A line for the status bar that outlives one render (a hidden suggestion, a refusal).
    status: Option<String>,
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

/// A view the launcher has pushed, and the task that listens for its data changes.
struct OpenView {
    view: Box<dyn View>,
    _listener: gpui::Task<()>,
}

impl LauncherView {
    fn navigate(&mut self, navigation: Navigation, window: &mut Window, cx: &mut Context<Self>) {
        if self.input.marked.is_some() {
            return;
        }
        match navigation {
            Navigation::Up => {
                self.selected = step_selection(&self.rows, self.selected, Step::Up);
                self.load_icons(window, cx);
                cx.notify();
            }
            Navigation::Down => {
                self.selected = step_selection(&self.rows, self.selected, Step::Down);
                self.load_icons(window, cx);
                cx.notify();
            }
            Navigation::Activate => self.activate_selected(window, cx),
            Navigation::Dismiss => self.dismiss(window),
        }
    }

    fn dismiss(&mut self, window: &mut Window) {
        self.reveal.set_open(false, std::time::Instant::now());
        window.refresh();
    }

    fn placeholder(&self) -> String {
        match self.stack.last() {
            Some(open) => open.view.placeholder().to_owned(),
            None if commands::available().is_empty() => "Search applications…".into(),
            None => "Search apps and commands…".into(),
        }
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
        initial: Option<CommandId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Toggled {
        match toggle_action(self.phase(), same_display) {
            // Asking for a view on a visible launcher goes there; it never closes the launcher.
            Action::Dismiss => {
                if let Some(command) = initial {
                    self.open_command(command, window, cx);
                } else {
                    self.dismiss(window);
                }
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
                    if let Some(command) = initial {
                        self.open_command(command, window, cx);
                    }
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
        self.stack.clear();
        self.rows.clear();
        self.selected = 0;
        self.error = None;
        self.status = None;
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
        self.status = None;
        match self.worker.refresh(self.input.text.clone()) {
            Ok(generation) => self.generation = generation,
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }

    fn search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.error = None;
        self.status = None;
        if !self.stack.is_empty() {
            self.selected = 0;
            self.view_rows(window, cx);
            return;
        }
        self.ready_generation = None;
        self.selected = 0;
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
                            this.ready_generation = Some(this.generation);
                            // A view's rows are not the root list's; the root list returns on pop.
                            if this.stack.is_empty() {
                                this.rows = results.rows;
                                this.selected = first_selectable(&this.rows);
                            }
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
            .rows
            .iter()
            .skip(self.selected.saturating_sub(9))
            .take(10)
            .filter_map(|row| match row.icon {
                RowIcon::Application(index) => Some(index),
                _ => None,
            })
            .filter(|index| {
                !self.icons.contains_key(index) && !self.icon_misses.contains_key(index)
            })
            .filter_map(|index| {
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

    /// Enter (or a click) on the selected row.
    fn activate_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.launch.busy {
            return;
        }
        let Some(row) = self.rows.get(self.selected).cloned() else {
            return;
        };
        let query = self.input.text.clone();
        if let Some(open) = self.stack.last_mut() {
            let host = Host {
                clipboard: cx.read_from_clipboard().and_then(|item| item.text()),
            };
            match open.view.activate(&row, &query, &host) {
                Outcome::Stay => self.view_rows(window, cx),
                Outcome::Notice(message) => {
                    self.view_rows(window, cx);
                    self.status = Some(message);
                    cx.notify();
                }
                Outcome::Close => self.dismiss(window),
                Outcome::Copy(text) => {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                    self.remember(&row, &query, cx);
                    self.dismiss(window);
                }
                Outcome::Error(message) => {
                    self.error = Some(message);
                    cx.notify();
                }
            }
            return;
        }
        if self.ready_generation != Some(self.generation) {
            return;
        }
        match row.action.clone() {
            RowAction::Launch(index) => self.launch_application(index, window, cx),
            RowAction::Open(command) => {
                self.remember(&row, &query, cx);
                self.open_command(command, window, cx);
            }
            RowAction::Copy(text) => {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                self.dismiss(window);
            }
            RowAction::OpenUrl(url) => {
                self.remember(&row, &query, cx);
                self.run_launch(window, cx, move || dojo::application_launch::open_url(&url));
            }
            RowAction::AskAgent(message) => {
                self.run_launch(window, cx, move || {
                    dojo::agent_runtime::send_text(&message).map_err(anyhow::Error::msg)
                });
            }
            RowAction::Item(_) | RowAction::Header => {}
        }
    }

    /// Remembers a row the user chose, so it ranks higher next time. Runs off the UI thread.
    fn remember(&self, row: &Row, query: &str, cx: &mut Context<Self>) {
        if row.id.is_empty() {
            return;
        }
        let (id, query) = (row.id.clone(), query.to_owned());
        cx.background_executor()
            .spawn(async move {
                if let Err(error) = dojo::launch_history::record_launch(&id, &query) {
                    eprintln!("Dojo could not record the choice: {error:#}");
                }
            })
            .detach();
    }

    fn launch_application(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(catalogue) = self.catalogue.clone() else {
            return;
        };
        let query = self.input.text.clone();
        self.run_launch(window, cx, move || {
            let application = catalogue
                .applications
                .get(index)
                .ok_or_else(|| anyhow::anyhow!("selected application disappeared"))?;
            catalogue::launch(application)?;
            // The launch succeeded; failing to remember it must not report a failure.
            if let Err(error) = dojo::launch_history::record_launch(&application.id, &query) {
                eprintln!("Dojo could not record the launch: {error:#}");
            }
            anyhow::Ok(())
        });
    }

    /// Runs a launch off the UI thread behind the launch gate: one at a time, closing the
    /// launcher on success and showing the error on failure.
    fn run_launch(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        work: impl FnOnce() -> anyhow::Result<()> + Send + 'static,
    ) {
        if !self.launch.begin() {
            return;
        }
        cx.spawn_in(window, async move |this, cx| {
            let result = cx.background_executor().spawn(async move { work() }).await;
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

    /// Pushes a command's view; says whether it opened. A command with no view reports why.
    fn open_command(
        &mut self,
        command: CommandId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(mut view) = (self.views)(command) else {
            self.error = Some(format!("{} is not available", command.command().title));
            cx.notify();
            return false;
        };
        let (changed, listen) = async_channel::bounded::<()>(1);
        let notify: Notify = Arc::new(move || {
            let _ = changed.try_send(());
        });
        view.open(notify);
        let listener = cx.spawn_in(window, async move |this, cx| {
            while listen.recv().await.is_ok() {
                if this
                    .update_in(cx, |this, window, cx| this.view_rows(window, cx))
                    .is_err()
                {
                    return;
                }
            }
        });
        self.stack.push(OpenView {
            view,
            _listener: listener,
        });
        self.input = SearchInput::default();
        self.input_line = None;
        self.error = None;
        self.status = None;
        self.selected = 0;
        self.view_rows(window, cx);
        true
    }

    /// Asks the open view for the rows matching the search box and shows them.
    fn view_rows(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = self.stack.last_mut() else {
            return;
        };
        self.rows = open.view.rows(&self.input.text);
        self.selected = if self.rows.get(self.selected).is_some_and(Row::is_selectable) {
            self.selected
        } else {
            first_selectable(&self.rows)
        };
        self.ready_generation = Some(self.generation);
        self.load_icons(window, cx);
        cx.notify();
    }

    /// Leaves the open view for the one below it, or for the root list.
    fn pop_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.stack.pop().is_none() {
            return;
        }
        self.input = SearchInput::default();
        self.input_line = None;
        self.error = None;
        self.status = None;
        if self.stack.is_empty() {
            // The root list for an empty search comes from the worker again.
            self.refresh(window, cx);
        } else {
            self.view_rows(window, cx);
        }
    }

    /// Escape: back out of a view, or close the launcher from the root list.
    fn back_or_dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.stack.is_empty() {
            self.dismiss(window);
        } else {
            self.pop_view(window, cx);
        }
    }

    /// Ctrl+H on a command: take it out of (or put it back into) the empty launcher's suggestions.
    fn toggle_suggestion(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(RowAction::Open(command)) =
            self.rows.get(self.selected).map(|row| row.action.clone())
        else {
            return;
        };
        if !self.stack.is_empty() {
            return;
        }
        let result = commands::hidden_path().and_then(|path| {
            let mut hidden = HiddenCommands::load(&path);
            let now_hidden = hidden.toggle(command);
            hidden.save(&path).map(|()| now_hidden)
        });
        self.status = Some(match result {
            Ok(true) => format!("{} hidden from suggestions", command.command().title),
            Ok(false) => format!("{} shown in suggestions", command.command().title),
            Err(error) => format!("Could not save that: {error:#}"),
        });
        // The worker reloads the hidden set whenever the catalogue refreshes.
        match self.worker.refresh(self.input.text.clone()) {
            Ok(generation) => {
                self.generation = generation;
                self.ready_generation = None;
            }
            Err(error) => self.error = Some(error),
        }
        let _ = window;
        cx.notify();
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.input.marked.is_some() {
            return;
        }
        let modifiers = event.keystroke.modifiers;
        match event.keystroke.key.as_str() {
            "escape" => self.back_or_dismiss(window, cx),
            "enter" if !event.is_held => self.navigate(Navigation::Activate, window, cx),
            "down" => self.navigate(Navigation::Down, window, cx),
            "up" => self.navigate(Navigation::Up, window, cx),
            "backspace" if self.input.text.is_empty() && !self.stack.is_empty() => {
                self.pop_view(window, cx);
            }
            "backspace" => {
                self.input.backspace();
                self.search(window, cx);
            }
            "h" if modifiers.control => self.toggle_suggestion(window, cx),
            "tab" => {
                let Some(status) = self.stack.last_mut().and_then(|open| open.view.tab()) else {
                    return;
                };
                self.status = Some(status);
                self.view_rows(window, cx);
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
                    view.placeholder().into()
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
        for (position, row) in self.rows.iter().enumerate().skip(start).take(10) {
            if row.action == RowAction::Header {
                rows.push(
                    div()
                        .id(("launcher-header", position))
                        .h(px(24.))
                        .px(px(8.))
                        .flex()
                        .items_end()
                        .text_size(px(11.))
                        .text_color(colors.foreground_muted.rgb())
                        .child(row.title.clone()),
                );
                continue;
            }
            let muted = colors.foreground_muted.rgb();
            let icon: gpui::AnyElement = match &row.icon {
                RowIcon::Application(index) => {
                    if let Some(Some(source)) = self.icons.get(index) {
                        gpui::img(source.clone()).size(px(22.)).into_any_element()
                    } else {
                        div()
                            .size(px(22.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(muted)
                            .child("◇")
                            .into_any_element()
                    }
                }
                RowIcon::Glyph(text) => div()
                    .size(px(22.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(text.clone())
                    .into_any_element(),
                RowIcon::Swatch([r, g, b]) => div()
                    .size(px(18.))
                    .m(px(2.))
                    .rounded(px(5.))
                    .bg(gpui::rgb(
                        (u32::from(*r) << 16) | (u32::from(*g) << 8) | u32::from(*b),
                    ))
                    .border_1()
                    .border_color(colors.border.rgb())
                    .into_any_element(),
                RowIcon::None => div().size(px(22.)).into_any_element(),
            };
            let label = match row.action {
                RowAction::Launch(_) => format!("Launch {} ({})", row.title, row.id),
                _ => row.title.clone(),
            };
            rows.push(
                div()
                    .id(("launcher-result", position))
                    .aria_label(label)
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
                    .child(div().flex_1().overflow_hidden().child(row.title.clone()))
                    .child(div().text_size(px(11.)).child(row.subtitle.clone()))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            this.selected = position;
                            this.activate_selected(window, cx);
                        }),
                    ),
            );
        }
        let status = if let Some(error) = &self.error {
            error.clone()
        } else if let Some(status) = &self.status {
            status.clone()
        } else if self.launch.busy {
            "Launching…".into()
        } else if self.ready_generation != Some(self.generation) {
            "Searching…".into()
        } else if self.rows.iter().all(|row| !row.is_selectable()) {
            match self.stack.last() {
                Some(open) => open.view.empty_text(&self.input.text),
                None => "No results found".into(),
            }
        } else if self.stack.is_empty() {
            match self.rows.get(self.selected).map(|row| &row.action) {
                Some(RowAction::Open(_)) => {
                    "↑ ↓ select · Enter open · Ctrl+H hide · Esc close".into()
                }
                _ => "↑ ↓ select · Enter launch · Esc close".into(),
            }
        } else {
            "↑ ↓ select · Enter choose · Esc back".into()
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
    use super::{Action, LaunchGate, Phase, toggle_action};

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

    mod fake {
        use std::sync::{Arc, Mutex};

        use dojo::launcher_core::commands::CommandId;
        use dojo::launcher_core::rows::{Row, RowAction, RowIcon};

        use super::super::views::{Host, Notify, Outcome, View, ViewFactory};

        /// What the fake view saw, shared with the test that built it.
        #[derive(Default)]
        pub struct Seen {
            pub queries: Vec<String>,
            pub activated: Vec<String>,
            pub notify: Option<Notify>,
        }

        pub struct FakeView {
            pub seen: Arc<Mutex<Seen>>,
            pub outcome: fn(&Row) -> Outcome,
        }

        impl View for FakeView {
            fn placeholder(&self) -> &str {
                "Search fake…"
            }

            fn open(&mut self, notify: Notify) {
                self.seen.lock().unwrap().notify = Some(notify);
            }

            fn rows(&mut self, query: &str) -> Vec<Row> {
                self.seen.lock().unwrap().queries.push(query.to_owned());
                let mut rows = vec![Row::header("Fake")];
                for title in ["alpha", "beta"] {
                    if title.contains(query) {
                        rows.push(Row {
                            id: format!("fake:{title}"),
                            title: title.to_owned(),
                            subtitle: String::new(),
                            icon: RowIcon::Glyph("*".into()),
                            action: RowAction::Item(rows.len()),
                        });
                    }
                }
                rows
            }

            fn activate(&mut self, row: &Row, _query: &str, _host: &Host) -> Outcome {
                self.seen.lock().unwrap().activated.push(row.title.clone());
                (self.outcome)(row)
            }
        }

        pub fn factory(seen: Arc<Mutex<Seen>>, outcome: fn(&Row) -> Outcome) -> ViewFactory {
            Arc::new(move |command| {
                (command == CommandId::Emoji).then(|| {
                    Box::new(FakeView {
                        seen: seen.clone(),
                        outcome,
                    }) as Box<dyn View>
                })
            })
        }
    }

    fn launcher_with_fake_view(
        cx: &mut gpui::TestAppContext,
        outcome: fn(&dojo::launcher_core::rows::Row) -> super::Outcome,
    ) -> (
        super::Launcher,
        gpui::WindowHandle<super::LauncherView>,
        std::sync::Arc<std::sync::Mutex<fake::Seen>>,
        gpui::DisplayId,
    ) {
        let display = gpui::DisplayId::from(2);
        cx.simulate_display_added(display, None);
        let seen = std::sync::Arc::new(std::sync::Mutex::new(fake::Seen::default()));
        let mut launcher = super::Launcher::new()
            .unwrap()
            .with_views(fake::factory(seen.clone(), outcome));
        cx.update(|cx| {
            cx.set_global(crate::config::ThemeColors::default());
            launcher
                .open_view(Some(display), super::CommandId::Emoji, cx)
                .unwrap();
        });
        let handle = launcher.window.unwrap();
        (launcher, handle, seen, display)
    }

    #[gpui::test]
    fn opening_a_view_shows_its_rows_and_escape_and_backspace_leave_it(
        cx: &mut gpui::TestAppContext,
    ) {
        let (mut launcher, handle, seen, display) =
            launcher_with_fake_view(cx, |_| super::Outcome::Stay);
        cx.update(|cx| {
            handle
                .update(cx, |view, window, cx| {
                    assert_eq!(view.stack.len(), 1);
                    assert_eq!(view.placeholder(), "Search fake…");
                    assert_eq!(view.rows.len(), 3, "a header and two rows");
                    assert_eq!(view.selected, 1, "the header is skipped");
                    view.input.replace(None, "alp");
                    view.search(window, cx);
                    assert_eq!(view.rows.len(), 2);
                    assert_eq!(view.selected, 1);
                    // Escape leaves the view for the root list without closing the launcher.
                    view.back_or_dismiss(window, cx);
                    assert!(view.stack.is_empty() && view.input.text.is_empty());
                    assert!(view.reveal.is_open());
                    // Backspace in an empty search leaves an open view too.
                    assert!(view.open_command(super::CommandId::Emoji, window, cx));
                    view.key_down(&backspace(), window, cx);
                    assert!(view.stack.is_empty());
                    // From the root list Escape closes.
                    view.back_or_dismiss(window, cx);
                    assert!(!view.reveal.is_open());
                })
                .unwrap();
        });
        assert!(seen.lock().unwrap().queries.iter().any(|q| q == "alp"));
        let _ = (&mut launcher, display);
    }

    fn backspace() -> gpui::KeyDownEvent {
        gpui::KeyDownEvent {
            keystroke: gpui::Keystroke::parse("backspace").unwrap(),
            is_held: false,
            prefer_character_input: false,
        }
    }

    #[gpui::test]
    fn activating_a_view_row_carries_out_its_outcome(cx: &mut gpui::TestAppContext) {
        let (_launcher, handle, seen, _display) = launcher_with_fake_view(cx, |row| {
            if row.title == "beta" {
                super::Outcome::Error("beta is refused".into())
            } else {
                super::Outcome::Copy(row.title.clone())
            }
        });
        cx.update(|cx| {
            handle
                .update(cx, |view, window, cx| {
                    // beta is refused: an error shows and the view stays open.
                    view.selected = 2;
                    view.activate_selected(window, cx);
                    assert_eq!(view.error.as_deref(), Some("beta is refused"));
                    assert!(view.reveal.is_open() && view.stack.len() == 1);
                    // alpha is copied to the clipboard and the launcher closes.
                    view.selected = 1;
                    view.activate_selected(window, cx);
                    assert!(!view.reveal.is_open());
                })
                .unwrap();
            assert_eq!(
                cx.read_from_clipboard()
                    .and_then(|item| item.text())
                    .as_deref(),
                Some("alpha")
            );
        });
        assert_eq!(seen.lock().unwrap().activated, ["beta", "alpha"]);
    }

    #[gpui::test]
    fn a_view_data_change_refreshes_its_rows_and_unknown_commands_report_why(
        cx: &mut gpui::TestAppContext,
    ) {
        let (launcher, handle, seen, _display) =
            launcher_with_fake_view(cx, |_| super::Outcome::Stay);
        // Let the search thread finish its one request first: it wakes the window's task, and
        // that must not happen while the test scheduler is parked.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while launcher.worker.results().ok().flatten().is_none() {
            assert!(
                std::time::Instant::now() < deadline,
                "the search worker never answered"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let notify = seen
            .lock()
            .unwrap()
            .notify
            .clone()
            .expect("the view was opened");
        let before = seen.lock().unwrap().queries.len();
        notify();
        cx.run_until_parked();
        assert!(
            seen.lock().unwrap().queries.len() > before,
            "a notification asks for rows again"
        );
        cx.update(|cx| {
            handle
                .update(cx, |view, window, cx| {
                    assert!(!view.open_command(super::CommandId::Services, window, cx));
                    assert_eq!(view.stack.len(), 1, "nothing was pushed");
                    assert_eq!(view.error.as_deref(), Some("Services is not available"));
                })
                .unwrap();
        });
    }

    #[gpui::test]
    fn reopening_the_launcher_returns_to_the_root_list_and_opening_a_view_keeps_it_open(
        cx: &mut gpui::TestAppContext,
    ) {
        let (mut launcher, handle, _seen, display) =
            launcher_with_fake_view(cx, |_| super::Outcome::Stay);
        cx.update(|cx| {
            // Asking for a view on a visible launcher does not close it.
            launcher
                .open_view(Some(display), super::CommandId::Emoji, cx)
                .unwrap();
            handle
                .update(cx, |view, _, _| {
                    assert!(view.reveal.is_open());
                    assert_eq!(view.stack.len(), 2, "pushed again on top");
                })
                .unwrap();
            // A plain toggle closes it, and the next opening starts at the root.
            launcher.toggle(Some(display), cx).unwrap();
            handle
                .update(cx, |view, _, _| assert!(!view.reveal.is_open()))
                .unwrap();
            launcher.toggle(Some(display), cx).unwrap();
            handle
                .update(cx, |view, _, _| {
                    assert!(view.stack.is_empty(), "reopening clears the views");
                    assert!(view.reveal.is_open());
                })
                .unwrap();
        });
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
