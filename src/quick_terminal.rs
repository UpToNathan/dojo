use std::collections::HashMap;
use std::process::{Child, Command, Stdio};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

use anyhow::Context as _;
use gpui::{
    App, AppContext, Bounds, Context, Render, Window, WindowBackgroundAppearance, WindowBounds,
    WindowHandle, WindowKind, WindowOptions, layer_shell, point, prelude::*, px,
};
use serde::Deserialize;

use crate::config::{QuickTerminalClient, ThemeColors};

#[derive(Default)]
pub struct QuickTerminal {
    panel: Option<WindowHandle<TerminalSurface>>,
}

#[derive(Deserialize)]
struct Workspace {
    is_focused: bool,
    output: Option<String>,
}
#[derive(Deserialize)]
struct Output {
    logical: Option<LogicalOutput>,
}
#[derive(Deserialize)]
struct LogicalOutput {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

fn focused_output() -> anyhow::Result<(String, LogicalOutput)> {
    let workspaces = Command::new("niri")
        .args(["msg", "--json", "workspaces"])
        .output()?;
    anyhow::ensure!(workspaces.status.success(), "Niri workspace query failed");
    let workspaces: Vec<Workspace> = serde_json::from_slice(&workspaces.stdout)?;
    let name = workspaces
        .into_iter()
        .find(|w| w.is_focused)
        .and_then(|w| w.output)
        .context("no focused output")?;
    let outputs = Command::new("niri")
        .args(["msg", "--json", "outputs"])
        .output()?;
    anyhow::ensure!(outputs.status.success(), "Niri output query failed");
    let mut outputs: HashMap<String, Output> = serde_json::from_slice(&outputs.stdout)?;
    let logical = outputs
        .remove(&name)
        .and_then(|o| o.logical)
        .context("focused output is inactive")?;
    anyhow::ensure!(
        logical.width >= 200 && logical.height >= 200,
        "output is too small for quick terminal"
    );
    Ok((name, logical))
}

#[derive(Clone, Copy)]
struct Placement {
    left: f32,
    top: f32,
    width: f32,
    height: f32,
}
impl Placement {
    fn new(width: f32, height: f32) -> Self {
        let panel_width = (width - crate::geometry::LEFT_PANEL_WIDTH as f32 - 48.)
            .min(940.)
            .floor();
        Self {
            left: ((width - panel_width) / 2.)
                .floor()
                .max(crate::geometry::LEFT_PANEL_WIDTH as f32 - 24.),
            top: crate::geometry::TOP_BAR_HEIGHT as f32,
            width: panel_width,
            height: (height - 100.).min(520.).floor(),
        }
    }
    fn bounds(self, progress: f32) -> Bounds<gpui::Pixels> {
        Bounds::new(
            point(px(self.left), px(self.top)),
            gpui::size(px(self.width), px(self.height * progress)),
        )
    }
    /// The terminal's own rectangle inside the outline: x and y from the corner Niri leaves
    /// after its reserved edges, as layer surfaces anchored there are placed.
    fn content(self) -> (f32, f32, f32, f32) {
        (
            self.left + 24. - crate::geometry::LEFT_PANEL_WIDTH as f32,
            self.top + 24. - crate::geometry::TOP_BAR_HEIGHT as f32,
            self.width - 48.,
            self.height - 48.,
        )
    }
    fn kitty_args(self, output: &str) -> Vec<String> {
        let (x, y, width, height) = self.content();
        let mut args = vec![
            "quick-access-terminal".into(),
            "--instance-group".into(),
            format!("dojo-shell-inset-{output}"),
        ];
        for value in [
            "edge=none".into(),
            format!("output_name={output}"),
            "app_id=dojo-quick-terminal".into(),
            format!("columns={width}px"),
            format!("lines={height}px"),
            format!("margin_left={x}"),
            format!("margin_top={y}"),
            "layer=overlay".into(),
            "background_opacity=1".into(),
            "kitty_override=window_padding_width=0".into(),
            "kitty_override=window_margin_width=0".into(),
            "hide_on_focus_loss=no".into(),
        ] {
            args.push("--override".into());
            args.push(value);
        }
        args
    }

    /// `dojo-terminal --panel` on the `dojo-quick` Rex session, in the same rectangle as kitty.
    fn rex_args(self, output: &str) -> Vec<String> {
        let (x, y, width, height) = self.content();
        let mut args: Vec<String> = vec!["--panel".into(), "--output".into(), output.into()];
        for (name, value) in [("--x", x), ("--y", y), ("--width", width), ("--height", height)] {
            args.push(name.into());
            args.push(value.to_string());
        }
        args.extend(["--session".into(), QUICK_SESSION.into(), "--".into()]);
        args.extend(session_command(|name| std::env::var(name).ok()));
        args
    }
}

const QUICK_SESSION: &str = "dojo-quick";

/// The Rex server is a systemd user service that may predate this desktop session, so a new
/// session gets the desktop's own variables through `env`.
const SESSION_VARIABLES: [&str; 7] = [
    "WAYLAND_DISPLAY",
    "NIRI_SOCKET",
    "DISPLAY",
    "DBUS_SESSION_BUS_ADDRESS",
    "SSH_AUTH_SOCK",
    "XDG_CURRENT_DESKTOP",
    "XDG_SESSION_TYPE",
];

fn session_command(var: impl Fn(&str) -> Option<String>) -> Vec<String> {
    let mut command = vec!["env".to_owned()];
    command.extend(
        SESSION_VARIABLES
            .iter()
            .filter_map(|name| var(name).map(|value| format!("{name}={value}"))),
    );
    command.push(terminal_session_script());
    command
}

fn terminal_session_script() -> String {
    format!("{}/scripts/dojo-terminal-session", env!("CARGO_MANIFEST_DIR"))
}

/// `dojo-terminal` is installed beside the shell.
fn rex_client() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .map(|exe| exe.with_file_name("dojo-terminal"))
        .filter(|path| path.is_file())
        .unwrap_or_else(|| "dojo-terminal".into())
}

/// How the terminal appears and goes: kitty toggles its own hidden instance; the Rex client is
/// started on show and ended on hide, leaving its session running.
enum Launcher {
    Kitty(Vec<String>),
    Rex { args: Vec<String>, child: Mutex<Option<Child>> },
}

impl Launcher {
    fn new(client: QuickTerminalClient, placement: Placement, output: &str) -> Self {
        match client {
            QuickTerminalClient::Kitty => Self::Kitty(placement.kitty_args(output)),
            QuickTerminalClient::Rex => Self::Rex { args: placement.rex_args(output), child: Mutex::new(None) },
        }
    }
    fn show(&self) -> anyhow::Result<()> {
        match self {
            Self::Kitty(args) => {
                let mut args = args.clone();
                args.push(terminal_session_script());
                launch(&args)
            }
            Self::Rex { args, child } => {
                let spawned = Command::new(rex_client())
                    .args(args)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .spawn()
                    .context("starting dojo-terminal")?;
                *child.lock().unwrap() = Some(spawned);
                Ok(())
            }
        }
    }
    fn hide(&self) -> anyhow::Result<()> {
        match self {
            Self::Kitty(args) => launch(args),
            Self::Rex { child, .. } => {
                if let Some(mut child) = child.lock().unwrap().take() {
                    let _ = child.kill();
                    std::thread::Builder::new()
                        .name("dojo-terminal-reaper".into())
                        .spawn(move || drop(child.wait()))?;
                }
                Ok(())
            }
        }
    }
}

fn launch(args: &[String]) -> anyhow::Result<()> {
    let mut child = Command::new("kitten")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .spawn()
        .context("starting quick-access Kitty")?;
    std::thread::Builder::new()
        .name("dojo-terminal-reaper".into())
        .spawn(move || match child.wait() {
            Ok(status) if !status.success() => eprintln!("Quick-access Kitty exited with {status}"),
            Err(error) => eprintln!("Quick-access Kitty wait failed: {error}"),
            _ => {}
        })?;
    Ok(())
}

impl QuickTerminal {
    pub fn toggle(&mut self, cx: &mut App) -> anyhow::Result<()> {
        if let Some(panel) = self.panel {
            if panel
                .update(cx, |view, window, cx| view.dismiss(window, cx))
                .is_ok()
            {
                return Ok(());
            }
            self.panel = None;
        }
        let (output_name, output) = focused_output()?;
        let display = crate::shell::focused_display_id(cx)
            .and_then(|id| cx.displays().into_iter().find(|d| d.id() == id))
            .context("focused output has no Dojo display")?;
        let placement = Placement::new(output.width as f32, output.height as f32);
        let client = crate::config::ConfigStore::user_default()
            .and_then(|store| store.quick_terminal_settings())
            .map(|settings| settings.client)
            .unwrap_or_else(|error| {
                eprintln!("Quick terminal settings: {error:#}");
                QuickTerminalClient::default()
            });
        let launcher = Arc::new(Launcher::new(client, placement, &output_name));
        let visible = Arc::new(AtomicBool::new(false));
        // The Rex client starts while the frame opens: connecting and its first frame take
        // longer than the reveal, and nothing of it shows until it has drawn.
        if client == QuickTerminalClient::Rex {
            match launcher.show() {
                Ok(()) => visible.store(true, Ordering::Relaxed),
                Err(error) => eprintln!("Quick terminal failed: {error:#}"),
            }
        }
        let release_launcher = launcher.clone();
        let release_visible = visible.clone();
        self.panel = Some(cx.open_window(
            WindowOptions {
                display_id: Some(display.id()),
                window_bounds: Some(WindowBounds::Windowed(display.bounds())),
                titlebar: None,
                app_id: Some("dojo".into()),
                window_background: WindowBackgroundAppearance::Transparent,
                kind: WindowKind::LayerShell(layer_shell::LayerShellOptions {
                    namespace: "dojo-terminal-shell".into(),
                    layer: layer_shell::Layer::Top,
                    anchor: layer_shell::Anchor::TOP
                        | layer_shell::Anchor::BOTTOM
                        | layer_shell::Anchor::LEFT
                        | layer_shell::Anchor::RIGHT,
                    exclusive_zone: Some(px(-1.)),
                    keyboard_interactivity: layer_shell::KeyboardInteractivity::None,
                    ..Default::default()
                }),
                ..Default::default()
            },
            move |window, cx| {
                window.set_input_region(Some(&[]));
                cx.new(move |cx| {
                    let observer = cx.observe_global::<ThemeColors>(|_, cx| cx.notify());
                    cx.on_release(move |_, _| {
                        if release_visible.swap(false, Ordering::Relaxed) {
                            if let Err(error) = release_launcher.hide() {
                                eprintln!("Could not hide quick terminal: {error}");
                            }
                        }
                    })
                    .detach();
                    TerminalSurface {
                        placement,
                        launcher,
                        visible,
                        reveal: crate::design::MenuReveal::opening_for(
                            Instant::now(),
                            Duration::from_millis(150),
                        ),
                        _observer: observer,
                    }
                })
            },
        )?);
        Ok(())
    }
    pub fn session_ended(&mut self, cx: &mut App) {
        if let Some(panel) = self.panel {
            let _ = panel.update(cx, |view, window, cx| {
                view.visible.store(false, Ordering::Relaxed);
                view.dismiss(window, cx);
            });
        }
    }
    pub fn close(&mut self, cx: &mut App) {
        if let Some(panel) = self.panel.take() {
            let _ = panel.update(cx, |_, window, _| window.remove_window());
        }
    }
}

struct TerminalSurface {
    placement: Placement,
    launcher: Arc<Launcher>,
    visible: Arc<AtomicBool>,
    reveal: crate::design::MenuReveal,
    _observer: gpui::Subscription,
}
impl TerminalSurface {
    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.visible.swap(false, Ordering::Relaxed) {
            if let Err(error) = self.launcher.hide() {
                eprintln!("Could not hide quick terminal: {error}");
            }
        }
        self.reveal.set_open(false, Instant::now());
        window.refresh();
        cx.notify();
    }
}
impl Render for TerminalSurface {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (progress, moving) = self
            .reveal
            .sample_with_motion(Instant::now(), cx.reduce_motion());
        if moving {
            window.request_animation_frame();
        } else if self.reveal.is_open() && !self.visible.load(Ordering::Relaxed) {
            match self.launcher.show() {
                Ok(()) => self.visible.store(true, Ordering::Relaxed),
                Err(error) => {
                    eprintln!("Quick terminal failed: {error:#}");
                    window.remove_window();
                }
            }
        } else if !self.reveal.is_open() {
            window.remove_window();
        }
        crate::design::shell_extension(
            self.placement.bounds(progress),
            cx.global::<ThemeColors>().background.rgb(),
            cx.global::<ThemeColors>().border.rgb(),
            false,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn kitty_is_aligned_with_the_shell_after_niri_reserved_edges() {
        let args = Placement::new(1645., 1028.).kitty_args("eDP-1");
        let value = |name: &str| {
            args.iter()
                .find_map(|arg| arg.strip_prefix(name))
                .unwrap()
                .parse::<f32>()
                .unwrap()
        };
        let actual = (
            value("margin_left=") + crate::geometry::LEFT_PANEL_WIDTH as f32,
            value("margin_top=") + crate::geometry::TOP_BAR_HEIGHT as f32,
        );
        assert_eq!(actual, (376., 48.));
    }
    #[test]
    fn the_rex_client_takes_kittys_rectangle_and_the_quick_session() {
        let args = Placement::new(1645., 1028.).rex_args("eDP-1");
        let after = |flag: &str| args[args.iter().position(|arg| arg == flag).unwrap() + 1].clone();
        assert_eq!(
            [after("--output"), after("--x"), after("--y"), after("--width"), after("--height")],
            ["eDP-1", "312", "24", "892", "472"]
        );
        assert_eq!(after("--session"), QUICK_SESSION);
        assert_eq!(after("--"), "env");
        assert!(args.last().unwrap().ends_with("scripts/dojo-terminal-session"));
    }
    #[test]
    fn a_new_session_carries_only_the_desktop_variables_that_are_set() {
        let command = session_command(|name| (name == "WAYLAND_DISPLAY").then(|| "wayland-1".into()));
        assert_eq!(command[..2], ["env".to_owned(), "WAYLAND_DISPLAY=wayland-1".to_owned()]);
        assert_eq!(command.len(), 3);
    }
    #[test]
    fn terminal_content_is_inset_inside_the_attached_shell_outline() {
        let p = Placement::new(1645., 1028.);
        assert_eq!((p.left, p.top, p.width, p.height), (352., 24., 940., 520.));
        let args = p.kitty_args("eDP-1");
        for expected in [
            "columns=892px",
            "lines=472px",
            "margin_left=312",
            "margin_top=24",
            "output_name=eDP-1",
        ] {
            assert!(args.iter().any(|arg| arg == expected), "missing {expected}");
        }
    }
}
