//! The send panel: pick a window and a machine, and send the window there. It sits at the bottom
//! centre of the focused output. The work is `dojo-mesh send`, run off the UI thread, so the panel
//! and the command line behave the same and every send is audited the same way.

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use dojo::send_panel::{List, Memory, Model, Phase, Preview, Request, Source, WindowChoice};
use gpui::{
    App, AppContext, Bounds, Context, FocusHandle, IntoElement, KeyDownEvent, MouseButton, Render,
    Window, WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowKind, WindowOptions, div,
    layer_shell, point, prelude::*, px,
};

use crate::config::ThemeColors;

/// How long the panel shows what was sent before the original closes, so it can be read and kept.
const SENT_LINGER: Duration = Duration::from_secs(4);

#[derive(Default)]
struct SendWindow {
    panel: Option<WindowHandle<Panel>>,
}
impl gpui::Global for SendWindow {}

fn memory_path() -> Option<PathBuf> {
    Memory::path(std::env::var_os("XDG_STATE_HOME").map(PathBuf::from), std::env::var_os("HOME").map(PathBuf::from))
}

/// Where the installed `dojo-mesh` is; the panel needs the same binary peers run.
fn mesh_binary() -> PathBuf {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")));
    data.map(|data| data.join("dojo/bin/dojo-mesh")).filter(|path| path.exists()).unwrap_or_else(|| PathBuf::from("dojo-mesh"))
}

pub fn toggle(display_id: gpui::DisplayId, cx: &mut App) -> anyhow::Result<()> {
    toggle_for(display_id, None, false, cx)
}

/// Opens the panel with `peer` already chosen, and in control-only mode when `control_only` is set
/// (the mesh panel's machine cards open it this way); closes it when it is open.
pub fn toggle_for(display_id: gpui::DisplayId, peer: Option<String>, control_only: bool, cx: &mut App) -> anyhow::Result<()> {
    if !cx.has_global::<SendWindow>() {
        cx.set_global(SendWindow::default());
    }
    if let Some(panel) = cx.update_global::<SendWindow, _>(|state, _| state.panel.take()) {
        if panel.update(cx, |_, window, _| window.remove_window()).is_ok() {
            return Ok(());
        }
    }
    // Read the windows before the panel takes the keyboard, while Niri still says which one has focus.
    let windows = dojo_mesh::niri::socket()
        .ok_or_else(|| "Niri is not running".to_string())
        .and_then(|socket| dojo_mesh::niri::windows(&socket));
    let display = cx
        .displays()
        .into_iter()
        .find(|display| display.id() == display_id)
        .ok_or_else(|| anyhow::anyhow!("the send panel's output disappeared"))?;
    let mut memory = memory_path().map(|path| Memory::load(&path)).unwrap_or_default();
    if peer.is_some() {
        memory.peer = peer;
    }
    if control_only {
        memory.input_only = true;
    }
    let panel = cx.open_window(
        WindowOptions {
            display_id: Some(display_id),
            window_bounds: Some(WindowBounds::Windowed(display.bounds())),
            titlebar: None,
            app_id: Some("dojo".into()),
            window_background: WindowBackgroundAppearance::Transparent,
            kind: WindowKind::LayerShell(layer_shell::LayerShellOptions {
                namespace: "dojo-send-window".into(),
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
        move |window, cx| {
            cx.new(|cx| {
                let focus = cx.focus_handle();
                window.focus(&focus, cx);
                let mut panel = Panel {
                    model: Model::loading(&memory),
                    memory,
                    focus,
                    reveal: dojo::design::MenuReveal::opening(Instant::now()),
                    task: None,
                    preview_task: None,
                    mode_t: 0.,
                    last_frame: Instant::now(),
                    show_keys: false,
                };
                panel.mode_t = if panel.model.input_only { 1. } else { 0. };
                panel.load(windows, cx);
                panel
            })
        },
    )?;
    cx.update_global::<SendWindow, _>(|state, _| state.panel = Some(panel));
    Ok(())
}

struct Panel {
    model: Model,
    memory: Memory,
    focus: FocusHandle,
    reveal: dojo::design::MenuReveal,
    task: Option<gpui::Task<()>>,
    preview_task: Option<gpui::Task<()>>,
    /// 0 while a window is being sent, 1 in control-only mode; it moves between them so the panel changes
    /// shape instead of jumping.
    mode_t: f32,
    last_frame: Instant,
    /// The key list is shown on request (`?`), not all the time.
    show_keys: bool,
}

/// The machines a window can go to: online members of the mesh other than this one.
fn online_peers() -> Vec<String> {
    let roster = dojo_mesh::cli::roster_now();
    roster.peers().filter(|member| member.device.online).map(|member| member.device.name.clone()).collect()
}

/// What `dojo-mesh send` said.
enum Outcome {
    Sent { sent: String, notes: Vec<String>, closes: String },
    Missing(Vec<String>),
    Failed(String),
}

/// What `dojo-mesh send --dry-run` says a window would send.
fn run_preview(window: u64, all_tabs: bool) -> Result<Preview, String> {
    let output = Command::new(mesh_binary())
        .args(["send", "--window", &window.to_string(), "--scope", if all_tabs { "window" } else { "tab" }, "--dry-run", "--json"])
        .output()
        .map_err(|error| format!("dojo-mesh could not run: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let message = stderr.lines().rev().find(|line| !line.trim().is_empty()).unwrap_or("it cannot be sent");
        return Err(message.trim_start_matches("dojo-mesh: ").to_owned());
    }
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|error| error.to_string())?;
    Ok(Preview {
        sent: value["sent"].as_str().unwrap_or_default().to_owned(),
        notes: value["notes"].as_array().map(|notes| notes.iter().filter_map(|note| note.as_str().map(String::from)).collect()).unwrap_or_default(),
        can_send_window: value["scopes"].as_array().is_some_and(|scopes| scopes.iter().any(|scope| scope == "window")),
        tab_count: value["tab_count"].as_u64().map(|count| count as usize),
    })
}

/// Closes the original after the peer opened the handoff, as `closes` (from `send --json`) says: the
/// window, only the tab sent, a browser window still holding the tabs sent, or nothing.
fn run_close_source(window: u64, closes: &str) -> Result<(), String> {
    let mut command = Command::new(mesh_binary());
    command.args(["close-source", "--window", &window.to_string(), "--closes", closes]);
    let output = command.output().map_err(|error| format!("dojo-mesh could not run: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(stderr.lines().rev().find(|line| !line.trim().is_empty()).unwrap_or("it could not be closed").trim_start_matches("dojo-mesh: ").to_owned())
}

fn run_send(window: u64, peer: &str, copy_missing: bool, all_tabs: bool) -> Outcome {
    let mut command = Command::new(mesh_binary());
    // The original stays until the panel has shown what was sent and let go of the keyboard.
    command.args(["send", "--window", &window.to_string(), "--to", peer, "--scope", if all_tabs { "window" } else { "tab" }, "--keep-source", "--json"]);
    if copy_missing {
        command.arg("--copy-missing");
    }
    let output = match command.output() {
        Ok(output) => output,
        Err(error) => return Outcome::Failed(format!("dojo-mesh could not run: {error}")),
    };
    let reply: Option<serde_json::Value> = serde_json::from_slice(&output.stdout).ok();
    let strings = |value: Option<&serde_json::Value>| -> Vec<String> {
        value.and_then(|v| v.as_array()).map(|items| items.iter().filter_map(|item| item.as_str().map(String::from)).collect()).unwrap_or_default()
    };
    match reply.as_ref().and_then(|reply| reply.get("status")).and_then(|status| status.as_str()) {
        Some("sent") => {
            let value = reply.as_ref();
            Outcome::Sent {
                sent: value.and_then(|r| r["sent"].as_str()).unwrap_or_default().to_owned(),
                notes: strings(value.and_then(|r| r.get("notes"))),
                closes: value.map(|r| r["closes"].to_string()).unwrap_or_else(|| "{\"scope\":\"window\"}".into()),
            }
        }
        Some("missing") => Outcome::Missing(strings(reply.as_ref().and_then(|r| r.get("files")))),
        _ => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let message = stderr.lines().rev().find(|line| !line.trim().is_empty()).unwrap_or("the send failed");
            Outcome::Failed(message.trim_start_matches("dojo-mesh: ").to_owned())
        }
    }
}

impl Panel {
    fn load(&mut self, windows: Result<Vec<dojo_mesh::niri::Window>, String>, cx: &mut Context<Self>) {
        let windows = match windows {
            Ok(windows) => windows,
            Err(message) => {
                self.model.failed(message);
                return;
            }
        };
        let focused = windows.iter().find(|window| window.is_focused).map(|window| window.id);
        let choices: Vec<WindowChoice> = windows
            .into_iter()
            .map(|window| WindowChoice {
                id: window.id,
                app_id: window.app_id.unwrap_or_default(),
                title: window.title.unwrap_or_default(),
            })
            .collect();
        let memory = self.memory.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let peers = cx.background_executor().spawn(async move { online_peers() }).await;
            let _ = this.update(cx, |panel: &mut Panel, cx| {
                panel.model.loaded(choices, focused, peers, &memory);
                panel.fetch_preview(cx);
                cx.notify();
            });
        }));
    }

    /// Asks what the selected window would send, once per window.
    fn fetch_preview(&mut self, cx: &mut Context<Self>) {
        let Some((window, all_tabs)) = self.model.needs_preview() else { return };
        // Marked as asked at once, so a second selection of the same window does not ask again.
        self.model.set_preview(window, all_tabs, Err("Reading what this window would send…".into()));
        self.preview_task = Some(cx.spawn(async move |this, cx| {
            let preview = cx.background_executor().spawn(async move { run_preview(window, all_tabs) }).await;
            let _ = this.update(cx, |panel: &mut Panel, cx| {
                panel.model.set_preview(window, all_tabs, preview);
                cx.notify();
            });
        }));
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.remove_window();
        cx.defer(|cx| {
            if cx.has_global::<SendWindow>() {
                cx.update_global::<SendWindow, _>(|state, _| state.panel = None);
            }
        });
    }

    fn handle(&mut self, request: Request, window: &mut Window, cx: &mut Context<Self>) {
        match request {
            Request::Nothing => {}
            Request::Close => self.close(window, cx),
            Request::Send { window: id, peer, take_input, copy_missing, all_tabs } => {
                self.memory = self.model.memory();
                if let Some(path) = memory_path() {
                    let _ = self.memory.save(&path);
                }
                self.task = Some(cx.spawn_in(window, async move |this, cx| {
                    let target = peer.clone();
                    let outcome = cx.background_executor().spawn(async move { run_send(id, &target, copy_missing, all_tabs) }).await;
                    let sent = matches!(outcome, Outcome::Sent { .. });
                    let _ = this.update(cx, |panel: &mut Panel, cx| {
                        match outcome {
                            Outcome::Sent { sent, notes, closes } => panel.model.sent(peer, sent, notes, id, closes, take_input),
                            Outcome::Missing(files) => panel.model.missing(files),
                            Outcome::Failed(message) => panel.model.failed(message),
                        }
                        cx.notify();
                    });
                    if sent {
                        // Time to read what was sent, and to press Esc to keep the original.
                        cx.background_executor().timer(SENT_LINGER).await;
                        let _ = this.update_in(cx, |panel: &mut Panel, window, cx| {
                            if let Phase::Sent { source: Source::Pending { .. }, .. } = panel.model.phase.clone() {
                                let request = panel.model.finish(false);
                                panel.handle(request, window, cx);
                            }
                        });
                    }
                }));
            }
            Request::RemoteOnly { peer } => {
                self.memory = self.model.memory();
                if let Some(path) = memory_path() {
                    let _ = self.memory.save(&path);
                }
                self.handle(Request::Finish { close: None, remote_to: Some(peer) }, window, cx);
                return;
            }
            Request::Finish { close, remote_to } => {
                // The panel holds the keyboard, so it goes first: the browser needs it to close a
                // tab, and the capture helper needs it to take the keyboard.
                self.close(window, cx);
                std::thread::spawn(move || {
                    // Let go of Enter first, or Niri holds the browser's input back as still being typed.
                    std::thread::sleep(Duration::from_millis(300));
                    if let Some((original, closes)) = close {
                        if let Err(message) = run_close_source(original, &closes) {
                            eprintln!("Dojo send window: the original was not closed: {message}");
                        }
                    }
                    if let Some(peer) = remote_to {
                        // Runs until the operator lets go (Super+Alt+F11) or the connection ends.
                        match Command::new(mesh_binary()).args(["remote", &peer]).output() {
                            Ok(output) if !output.status.success() => {
                                eprintln!("Dojo send window: remote input ended: {}", String::from_utf8_lossy(&output.stderr).trim());
                            }
                            Err(error) => eprintln!("Dojo send window: could not start remote input: {error}"),
                            Ok(_) => {}
                        }
                    }
                });
            }
        }
        cx.notify();
    }
}

mod size {
    pub const PADDING: f32 = 18.;
    pub const GAP: f32 = 10.;
    pub const HEADER: f32 = 36.;
    pub const TILE_HEIGHT: f32 = 84.;
    pub const TILE_WIDTH: f32 = 112.;
    pub const TILE_GAP: f32 = 8.;
    pub const ROW: f32 = 34.;
    pub const STATUS: f32 = 18.;
    pub const FOOTER: f32 = 40.;
    pub const BIG: f32 = 44.;
    pub const WIDTH_WINDOW: f32 = 640.;
    pub const WIDTH_CONTROL: f32 = 420.;
    pub const ROWS: usize = 5;
}

mod glyph {
    pub const WINDOW: &str = "\u{f05b2}";
    pub const KEYBOARD: &str = "\u{f030c}";
    pub const MOUSE: &str = "\u{f037d}";
    pub const MONITOR: &str = "\u{f0379}";
    pub const SEND: &str = "\u{f048a}";
}

/// The colour between `a` and `b`, `t` of the way.
fn mix(a: gpui::Rgba, b: gpui::Rgba, t: f32) -> gpui::Rgba {
    gpui::Rgba { r: a.r + (b.r - a.r) * t, g: a.g + (b.g - a.g) * t, b: a.b + (b.b - a.b) * t, a: a.a + (b.a - a.a) * t }
}

/// The accent when only the cursor and keyboard go: warmer, so the mode shows from across the room.
const CONTROL_ACCENT: u32 = 0xe5a550;

impl Panel {
    /// The one line under the choices, when there is something to say. Sentences are for what a
    /// glyph cannot say: a failure, a missing file, what is about to be sent.
    fn status_line(&self) -> Option<String> {
        match &self.model.phase {
            Phase::Loading => Some("Reading windows and machines…".into()),
            Phase::Sending => Some("Sending…".into()),
            Phase::Failed(message) => Some(format!("Not sent: {message}")),
            Phase::Missing(files) => {
                let peer = self.model.peers.get(self.model.peer).cloned().unwrap_or_default();
                Some(format!("{peer} lacks {} file{}: Enter copies them and sends", files.len(), if files.len() == 1 { "" } else { "s" }))
            }
            Phase::Sent { peer, sent, source, take_input, .. } => Some(match (source, take_input) {
                (Source::Pending { .. }, true) => format!("Sent to {peer}: {sent}. Taking control next · Esc keeps the original"),
                (Source::Pending { .. }, false) => format!("Sent to {peer}: {sent} · Esc keeps the original"),
                (Source::Kept, _) => format!("Sent to {peer}. The original is kept"),
            }),
            Phase::Choosing => {
                if let Some(why) = self.model.blocked() {
                    return Some(why.to_owned());
                }
                if self.model.input_only {
                    return None;
                }
                match self.model.preview() {
                    Some(Ok(preview)) => Some(format!("Sends {}", preview.sent)),
                    Some(Err(message)) if message.starts_with("Reading") => Some(message.clone()),
                    Some(Err(message)) => Some(format!("This window cannot be sent: {message}")),
                    None => None,
                }
            }
        }
    }

    fn glyph_button(&self, id: &'static str, glyph: &'static str, label: &str, lit: bool, accent: gpui::Rgba, cx: &mut Context<Self>, action: impl Fn(&mut Panel, &mut Window, &mut Context<Panel>) + 'static) -> gpui::AnyElement {
        let colors = *cx.global::<ThemeColors>();
        div()
            .id(id)
            .size(px(32.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(dojo::design::PILL_RADIUS))
            .text_size(px(16.))
            .when(lit, |el| el.bg(accent).text_color(colors.primary_foreground.rgb()))
            .when(!lit, |el| el.text_color(colors.foreground_muted.rgb()).hover(|el| el.bg(colors.background_highlight.rgb())))
            .cursor_pointer()
            .tooltip(dojo::design::tip(label.to_string()))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |panel, _, window, cx| {
                    action(panel, window, cx);
                    cx.stop_propagation();
                }),
            )
            .child(glyph)
            .into_any_element()
    }
}

impl Render for Panel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = *cx.global::<ThemeColors>();
        let viewport = window.viewport_size();
        let (progress, moving) = self.reveal.sample_with_motion(Instant::now(), cx.reduce_motion());

        // The mode moves towards where the model says it is.
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32().min(0.05);
        self.last_frame = now;
        let target = if self.model.input_only { 1. } else { 0. };
        if cx.reduce_motion() {
            self.mode_t = target;
        } else if self.mode_t < target {
            self.mode_t = (self.mode_t + dt / 0.18).min(target);
        } else if self.mode_t > target {
            self.mode_t = (self.mode_t - dt / 0.18).max(target);
        }
        let t = self.mode_t;
        if moving || (t - target).abs() > f32::EPSILON {
            window.request_animation_frame();
        }
        let accent = mix(colors.primary.rgb(), gpui::rgb(CONTROL_ACCENT), t);
        let blocked = self.model.blocked().is_some();

        let status = if self.show_keys {
            Some("Tab list · ←→ machine · ↑↓ window · I control only · W all tabs · Space bring control · Enter go · Esc close".to_string())
        } else {
            self.status_line()
        };
        let windows_rows = self.model.windows.len().clamp(1, size::ROWS) as f32;
        let windows_height = (size::GAP + size::ROW * windows_rows) * (1. - t);
        let big_height = (size::GAP + size::BIG) * t;
        let content_height = size::PADDING * 2.
            + size::HEADER
            + size::GAP
            + size::TILE_HEIGHT
            + windows_height
            + big_height
            + status.as_ref().map_or(0., |_| size::GAP + size::STATUS)
            + size::GAP
            + size::FOOTER;
        let full_width = size::WIDTH_WINDOW + (size::WIDTH_CONTROL - size::WIDTH_WINDOW) * t;
        let width = full_width.min((f32::from(viewport.width) - 120.).max(200.));
        let height = content_height.min((f32::from(viewport.height) - 80.).max(80.));
        let bottom = crate::geometry::BOTTOM_INSET as f32;
        let left = (f32::from(viewport.width) - width) / 2.;
        let bounds = Bounds::new(
            point(px(left), viewport.height - px(bottom + height * progress)),
            gpui::size(px(width), px(height * progress)),
        );

        // Header: the mode as a two-way toggle of glyphs, and nothing else.
        let toggle = div()
            .flex()
            .items_center()
            .gap(px(2.))
            .p(px(2.))
            .rounded(px(dojo::design::PILL_RADIUS))
            .bg(colors.surface_raised.rgb())
            .child(self.glyph_button("send-mode-window", glyph::WINDOW, "Send a window", !self.model.input_only, accent, cx, |panel, _, cx| {
                panel.model.set_control_only(false);
                cx.notify();
            }))
            .child(self.glyph_button("send-mode-control", glyph::KEYBOARD, "Only take control", self.model.input_only, accent, cx, |panel, _, cx| {
                panel.model.set_control_only(true);
                cx.notify();
            }));
        let heading = if self.model.input_only { "Take control" } else { "Send a window" };
        let header = div()
            .h(px(size::HEADER))
            .flex()
            .items_center()
            .justify_between()
            .child(div().text_size(px(15.)).text_color(mix(colors.foreground.rgb(), accent, t)).child(heading))
            .child(toggle);

        // Machines as tiles.
        let columns = (((width - size::PADDING * 2.) + size::TILE_GAP) / (size::TILE_WIDTH + size::TILE_GAP)).floor().max(1.) as usize;
        let peer_start = self.model.peer.saturating_sub(columns - 1);
        let machines_focused = self.model.list == List::Peers || self.model.input_only;
        let mut tiles = div().flex().gap(px(size::TILE_GAP)).h(px(size::TILE_HEIGHT));
        for (index, peer) in self.model.peers.iter().enumerate().skip(peer_start).take(columns) {
            let selected = index == self.model.peer;
            tiles = tiles.child(
                div()
                    .id(format!("send-peer-{index}"))
                    .w(px(size::TILE_WIDTH))
                    .h_full()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(6.))
                    .rounded(px(16.))
                    .border_2()
                    .border_color(if selected { accent } else { colors.surface_raised.rgb() })
                    .bg(if selected { colors.background_highlight.rgb() } else { colors.surface_raised.rgb() })
                    .opacity(if selected || machines_focused { 1. } else { 0.8 })
                    .cursor_pointer()
                    .child(div().text_size(px(24.)).text_color(if selected { accent } else { colors.foreground_muted.rgb() }).child(glyph::MONITOR))
                    .child(div().text_size(px(13.)).truncate().child(peer.clone()))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |panel, _, _, cx| {
                            panel.model.select(List::Peers, index);
                            cx.notify();
                            cx.stop_propagation();
                        }),
                    ),
            );
        }
        if self.model.peers.is_empty() {
            tiles = tiles.child(div().flex().items_center().text_size(px(12.)).text_color(colors.foreground_muted.rgb()).child("No other machine is online"));
        }

        // Windows: a short list that folds away in control-only mode.
        let windows_active = self.model.list == List::Windows && !self.model.input_only;
        let window_start = self.model.window.saturating_sub(size::ROWS - 1);
        let mut windows_column = div().flex().flex_col().gap(px(2.));
        for (index, choice) in self.model.windows.iter().enumerate().skip(window_start).take(size::ROWS) {
            let selected = index == self.model.window;
            windows_column = windows_column.child(
                div()
                    .id(format!("send-window-{index}"))
                    .h(px(size::ROW - 2.))
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .rounded(px(10.))
                    .bg(if selected && windows_active { colors.background_highlight.rgb() } else { colors.background.rgb() })
                    .text_size(px(13.))
                    .cursor_pointer()
                    .child(div().flex_none().text_color(if selected && windows_active { accent } else { colors.foreground.rgb() }).child(choice.app_id.clone()))
                    .child(div().flex_1().min_w_0().truncate().text_color(colors.foreground_muted.rgb()).child(choice.title.clone()))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |panel, _, _, cx| {
                            panel.model.select(List::Windows, index);
                            panel.fetch_preview(cx);
                            cx.notify();
                            cx.stop_propagation();
                        }),
                    ),
            );
        }

        // Footer: whether the cursor and keyboard come along (window mode), a way to see the keys, and the go button.
        let go_label = if self.model.input_only { "Take control" } else { "Send" };
        let go = div()
            .id("send-go")
            .h(px(32.))
            .px(px(16.))
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded(px(dojo::design::PILL_RADIUS))
            .text_size(px(13.))
            .when(!blocked, |el| el.bg(accent).text_color(colors.primary_foreground.rgb()).cursor_pointer())
            .when(blocked, |el| el.bg(colors.surface_raised.rgb()).text_color(colors.foreground_muted.rgb()))
            .child(div().text_size(px(15.)).child(if self.model.input_only { glyph::KEYBOARD } else { glyph::SEND }))
            .child(go_label)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|panel, _, window, cx| {
                    let request = panel.model.enter();
                    panel.fetch_preview(cx);
                    panel.handle(request, window, cx);
                    cx.stop_propagation();
                }),
            );
        let bring = self.glyph_button("send-take-input", glyph::KEYBOARD, "Bring cursor and keyboard along", self.model.take_input, accent, cx, |panel, _, cx| {
            panel.model.take_input = !panel.model.take_input;
            cx.notify();
        });
        let footer = div()
            .h(px(size::FOOTER))
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(self.glyph_button("send-keys", "?", "Show the keys", self.show_keys, accent, cx, |panel, _, cx| {
                        panel.show_keys = !panel.show_keys;
                        cx.notify();
                    }))
                    .child(div().opacity(1. - t).child(bring)),
            )
            .child(go);

        let mut content = div()
            .flex()
            .flex_col()
            .gap(px(size::GAP))
            .p(px(size::PADDING))
            .text_size(px(13.))
            .child(header)
            .child(tiles)
            .child(div().h(px(big_height)).overflow_hidden().opacity(t).flex().items_center().justify_center().gap(px(14.)).text_size(px(size::BIG * 0.8)).text_color(accent).child(glyph::KEYBOARD).child(glyph::MOUSE))
            .child(div().h(px(windows_height)).overflow_hidden().opacity(1. - t).child(div().pt(px(size::GAP)).child(windows_column)));
        if let Some(line) = status {
            content = content.child(div().h(px(size::STATUS)).text_size(px(11.)).truncate().text_color(if self.show_keys { colors.foreground_muted.rgb() } else { colors.foreground.rgb() }).child(line));
        }
        content = content.child(footer);

        div()
            .size_full()
            .font_family(crate::power::FONT_FAMILY)
            .font_features(crate::power::font_features())
            .text_color(colors.foreground.rgb())
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|panel, event: &KeyDownEvent, window, cx| {
                cx.stop_propagation();
                if event.is_held {
                    return;
                }
                if event.keystroke.key == "?" || (event.keystroke.key == "/" && event.keystroke.modifiers.shift) {
                    panel.show_keys = !panel.show_keys;
                    cx.notify();
                    return;
                }
                let request = panel.model.key(&event.keystroke.key);
                panel.fetch_preview(cx);
                panel.handle(request, window, cx);
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |panel, event: &gpui::MouseDownEvent, window, cx| {
                    if !bounds.contains(&event.position) && panel.model.phase != Phase::Sending {
                        panel.close(window, cx);
                    }
                }),
            )
            .child(dojo::design::bottom_center_shell_extension(bounds, colors.background.rgb(), colors.border.rgb()))
            .child(
                div()
                    .id("send-window-panel")
                    .absolute()
                    .left(px(left))
                    .bottom(px(bottom))
                    .w(px(width))
                    .h(px(height * progress))
                    .opacity(progress)
                    .overflow_hidden()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(content),
            )
    }
}
