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
    let memory = memory_path().map(|path| Memory::load(&path)).unwrap_or_default();
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
                };
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
}

/// The machines a window can go to: online members of the mesh other than this one.
fn online_peers() -> Vec<String> {
    let var = |name: &str| std::env::var_os(name).map(PathBuf::from);
    let config = dojo_mesh::config::default_path(var("XDG_CONFIG_HOME"), var("HOME"))
        .and_then(|path| dojo_mesh::config::MeshConfig::load(&path).ok())
        .unwrap_or_default();
    let hostname = std::fs::read_to_string("/proc/sys/kernel/hostname").map(|name| name.trim().to_owned()).unwrap_or_default();
    let roster = dojo_mesh::tailscale::roster(dojo_mesh::tailscale::read_status(), &config, &hostname);
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
                        // Runs until the operator lets go (Super+F11) or the connection ends.
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
    pub const GAP: f32 = 8.;
    pub const LINE: f32 = 20.;
    pub const ROW: f32 = 36.;
    pub const TITLE: f32 = 24.;
    pub const NOTE: f32 = 16.;
    pub const HINT: f32 = 16.;
    pub const WIDTH: f32 = 720.;
    pub const ROWS: usize = 6;
}

impl Panel {
    fn status_lines(&self) -> Vec<String> {
        match &self.model.phase {
            Phase::Loading => vec!["Reading windows and machines…".into()],
            Phase::Sending => vec!["Sending…".into()],
            Phase::Failed(message) => vec![format!("Not sent: {message}")],
            Phase::Missing(files) => {
                let mut lines = vec![format!("{} does not have {} file{}. Enter copies {} across and sends; Esc goes back.", self.model.peers.get(self.model.peer).cloned().unwrap_or_default(), files.len(), if files.len() == 1 { "" } else { "s" }, if files.len() == 1 { "it" } else { "them" })];
                lines.extend(files.iter().take(3).cloned());
                lines
            }
            Phase::Sent { peer, sent, notes, source, take_input } => {
                let mut lines = vec![format!("Sent to {peer}: {sent}.")];
                lines.extend(notes.iter().cloned());
                if *take_input {
                    lines.push(format!("Your cursor and keyboard go to {peer} next. Super+F11 brings them back."));
                }
                lines.push(match source {
                    Source::Pending { .. } => "Closing the original in a moment · Enter closes it now · Esc keeps it".into(),
                    Source::Kept => "The original is kept.".into(),
                });
                lines
            }
            Phase::Choosing => {
                if let Some(why) = self.model.blocked() {
                    return vec![why.to_owned()];
                }
                if self.model.input_only {
                    let peer = self.model.peers.get(self.model.peer).cloned().unwrap_or_default();
                    return vec![format!("No window is sent. Enter takes your cursor and keyboard to {peer}; Super+F11 brings them back.")];
                }
                match self.model.preview() {
                    Some(Ok(preview)) => {
                        let mut lines = vec![format!("Sends {}.", preview.sent)];
                        lines.extend(preview.notes.iter().take(2).cloned());
                        if preview.can_send_window {
                            let all = preview.tab_count.map(|count| format!("all {count} tabs")).unwrap_or_else(|| "the whole window".into());
                            lines.push(if self.model.all_tabs { "W sends only the tab in front instead.".into() } else { format!("W sends {all} instead.") });
                        }
                        lines
                    }
                    Some(Err(message)) if message.starts_with("Reading") => vec![message.clone()],
                    Some(Err(message)) => vec![format!("This window cannot be sent: {message}")],
                    None => Vec::new(),
                }
            }
        }
    }
}

impl Render for Panel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = *cx.global::<ThemeColors>();
        let viewport = window.viewport_size();
        let (progress, moving) = self.reveal.sample_with_motion(Instant::now(), cx.reduce_motion());
        if moving {
            window.request_animation_frame();
        }
        let status = self.status_lines();
        let rows = self.model.windows.len().max(self.model.peers.len()).clamp(1, size::ROWS);
        let content_height = size::PADDING * 2.
            + size::TITLE
            + size::ROW * rows as f32
            + size::LINE
            + size::NOTE * status.len() as f32
            + size::HINT
            + size::GAP * 4.;
        let width = size::WIDTH.min((f32::from(viewport.width) - 120.).max(200.));
        let height = content_height.min((f32::from(viewport.height) - 80.).max(80.));
        let bottom = crate::geometry::BOTTOM_INSET as f32;
        let left = (f32::from(viewport.width) - width) / 2.;
        let bounds = Bounds::new(
            point(px(left), viewport.height - px(bottom + height * progress)),
            gpui::size(px(width), px(height * progress)),
        );

        let row = |id: String, text: String, sub: String, selected: bool, active: bool, list: List, index: usize, cx: &mut Context<Self>| {
            let strong = if selected && active { colors.selection_foreground.rgb() } else { colors.foreground.rgb() };
            div()
                .id(id)
                .h(px(size::ROW))
                .px(px(12.))
                .flex()
                .items_center()
                .gap(px(10.))
                .rounded(px(10.))
                .bg(if selected { colors.background_highlight.rgb() } else { colors.background.rgb() })
                .text_color(strong)
                .cursor_pointer()
                .child(div().flex_none().child(text))
                .child(div().flex_1().min_w_0().truncate().text_color(if selected && active { strong } else { colors.foreground_muted.rgb() }).child(sub))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |panel, _, _, cx| {
                        panel.model.select(list, index);
                        panel.fetch_preview(cx);
                        cx.notify();
                        cx.stop_propagation();
                    }),
                )
        };

        let windows_active = self.model.list == List::Windows;
        let window_start = self.model.window.saturating_sub(size::ROWS - 1);
        let mut windows_column = div().flex().flex_col().flex_1().min_w_0();
        for (index, choice) in self.model.windows.iter().enumerate().skip(window_start).take(size::ROWS) {
            windows_column = windows_column.child(row(
                format!("send-window-{index}"),
                choice.app_id.clone(),
                choice.title.clone(),
                index == self.model.window,
                windows_active,
                List::Windows,
                index,
                cx,
            ));
        }
        let mut peers_column = div().flex().flex_col().w(px(200.)).flex_none();
        for (index, peer) in self.model.peers.iter().enumerate().take(size::ROWS) {
            peers_column = peers_column.child(row(
                format!("send-peer-{index}"),
                peer.clone(),
                String::new(),
                index == self.model.peer,
                !windows_active,
                List::Peers,
                index,
                cx,
            ));
        }

        let mut content = div()
            .flex()
            .flex_col()
            .gap(px(size::GAP))
            .p(px(size::PADDING))
            .text_size(px(13.))
            .line_height(px(size::LINE))
            .child(div().text_size(px(16.)).line_height(px(size::TITLE)).truncate().child("Send a window to another machine"))
            .child(div().flex().gap(px(12.)).child(windows_column).child(div().text_color(colors.foreground_muted.rgb()).child("→")).child(peers_column))
            .child(
                div()
                    .id("send-take-input")
                    .flex()
                    .gap(px(8.))
                    .cursor_pointer()
                    .child(if self.model.take_input { "[x]" } else { "[ ]" })
                    .child("Take my cursor and keyboard")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|panel, _, _, cx| {
                            panel.model.take_input = !panel.model.take_input;
                            cx.notify();
                            cx.stop_propagation();
                        }),
                    ),
            );
        for line in status {
            content = content.child(div().text_size(px(11.)).line_height(px(size::NOTE)).truncate().child(line));
        }
        content = content.child(
            div()
                .text_size(px(11.))
                .line_height(px(size::HINT))
                .text_color(colors.foreground_muted.rgb())
                .truncate()
                .child("Tab windows/machines · W all tabs · I only cursor and keyboard · Space take them along · Enter go · Esc close"),
        );

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
                let request = panel.model.key(&event.keystroke.key);
                panel.fetch_preview(cx);
                panel.handle(request, window, cx);
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
