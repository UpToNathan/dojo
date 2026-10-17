use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

use anyhow::Context as _;
use gpui::{
    App, AppContext, Bounds, Context, Render, Window, WindowBackgroundAppearance, WindowBounds,
    WindowHandle, WindowKind, WindowOptions, layer_shell, point, prelude::*, px,
};
use serde::Deserialize;

use crate::config::ThemeColors;

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
    fn kitty_args(self, output: &str) -> Vec<String> {
        let mut args = vec![
            "quick-access-terminal".into(),
            "--instance-group".into(),
            format!("dojo-shell-inset-{output}"),
        ];
        for value in [
            "edge=none".into(),
            format!("output_name={output}"),
            "app_id=dojo-quick-terminal".into(),
            format!("columns={}px", self.width - 48.),
            format!("lines={}px", self.height - 48.),
            format!(
                "margin_left={}",
                self.left + 24. - crate::geometry::LEFT_PANEL_WIDTH as f32
            ),
            format!(
                "margin_top={}",
                self.top + 24. - crate::geometry::TOP_BAR_HEIGHT as f32
            ),
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
        let display = cx
            .displays()
            .into_iter()
            .find(|d| {
                let bounds = d.bounds();
                (f32::from(bounds.origin.x) - output.x as f32).abs() < 2.
                    && (f32::from(bounds.origin.y) - output.y as f32).abs() < 2.
            })
            .context("focused output has no Dojo display")?;
        let placement = Placement::new(output.width as f32, output.height as f32);
        let args = placement.kitty_args(&output_name);
        let visible = Arc::new(AtomicBool::new(false));
        let release_args = args.clone();
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
                            if let Err(error) = launch(&release_args) {
                                eprintln!("Could not hide quick terminal: {error}");
                            }
                        }
                    })
                    .detach();
                    TerminalSurface {
                        placement,
                        args,
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
    args: Vec<String>,
    visible: Arc<AtomicBool>,
    reveal: crate::design::MenuReveal,
    _observer: gpui::Subscription,
}
impl TerminalSurface {
    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.visible.swap(false, Ordering::Relaxed) {
            if let Err(error) = launch(&self.args) {
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
            let mut args = self.args.clone();
            args.push(format!(
                "{}/scripts/dojo-terminal-session",
                env!("CARGO_MANIFEST_DIR")
            ));
            match launch(&args) {
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
