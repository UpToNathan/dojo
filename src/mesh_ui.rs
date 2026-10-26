//! The mesh panel: the machines this one is in a mesh with, at the top right like the monitor panel.
//! It reads one snapshot (`dojo_mesh::panel`) and changes nothing itself: every action is the
//! `dojo-mesh` command that does it, so the terminal and the panel cannot disagree. Words are kept for
//! what only words can say; the rest is dots, glyphs and numbers.

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use dojo_mesh::doctor::Severity;
use dojo_mesh::panel::{Kind, Offer, Panel, Request, Signer};
use gpui::{
    App, AppContext, Bounds, Context, DisplayId, FocusHandle, IntoElement, KeyDownEvent,
    MouseButton, Render, Window, WindowBackgroundAppearance, WindowBounds, WindowHandle,
    WindowKind, WindowOptions, div, layer_shell, point, prelude::*, px,
};

use crate::config::ThemeColors;

mod glyph {
    pub const KEYBOARD: &str = "\u{f030c}";
    pub const SEND: &str = "\u{f048a}";
    pub const RENEW: &str = "\u{f0450}";
    pub const DELETE: &str = "\u{f01b4}";
    pub const PLUS: &str = "\u{f0415}";
    pub const CHECK: &str = "\u{f012c}";
    pub const CLOSE: &str = "\u{f0156}";
    pub const KEY: &str = "\u{f0306}";
    pub const LOCK: &str = "\u{f033e}";
    pub const ALERT: &str = "\u{f0026}";
    pub const HEALTHY: &str = "\u{f0565}";
    pub const INBOX: &str = "\u{f0689}";
    pub const LAPTOP: &str = "\u{f0322}";
    pub const USB: &str = "\u{f0553}";
}

const WIDTH: f32 = 360.;
const PAD: f32 = 16.;
const GAP: f32 = 8.;
const HEADER: f32 = 32.;
const CARD: f32 = 52.;
const LINE: f32 = 26.;
const REQUEST: f32 = 92.;
const KEY_ROW: f32 = 44.;
const OFFER: f32 = 76.;
/// The most machines drawn; the panel never scrolls, so a longer mesh is cut here.
const MAX_CARDS: usize = 9;
const REFRESH: Duration = Duration::from_secs(3);

#[derive(Default)]
struct MeshUi {
    panel: Option<WindowHandle<Mesh>>,
}
impl gpui::Global for MeshUi {}

/// Which part of the mesh the panel is showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum View {
    Machines,
    Findings,
    /// Machines asking to join, waiting for approval.
    Requests,
    /// The signing keys this node trusts, and keys offered to it.
    Keys,
}

/// Something that asks the person to press a second time before it happens.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Confirm {
    Remove(String),
    Approve(PathBuf),
    Trust(String),
    Untrust(String),
}

/// A `dojo-mesh` command that is running for the panel.
struct Job {
    /// Whether it signs, so a hardware key may be waiting for a touch.
    signs: bool,
}

struct Mesh {
    data: Option<Panel>,
    view: View,
    confirm: Option<Confirm>,
    /// Requests the person put away for now (the files are left where they are).
    dismissed: std::collections::HashSet<PathBuf>,
    job: Option<Job>,
    /// The last thing that went wrong, short.
    notice: Option<String>,
    display: DisplayId,
    focus: FocusHandle,
    reveal: dojo::design::MenuReveal,
    _polling: gpui::Task<()>,
}

/// Where the installed `dojo-mesh` is; the panel runs the same binary the terminal does.
fn mesh_binary() -> PathBuf {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")));
    data.map(|data| data.join("dojo/bin/dojo-mesh")).filter(|path| path.exists()).unwrap_or_else(|| PathBuf::from("dojo-mesh"))
}

pub fn toggle(display_id: DisplayId, tab: crate::control::MeshTab, cx: &mut App) -> anyhow::Result<()> {
    if !cx.has_global::<MeshUi>() {
        cx.set_global(MeshUi::default());
    }
    if let Some(panel) = cx.update_global::<MeshUi, _>(|ui, _| ui.panel.take()) {
        if panel.update(cx, |_, window, _| window.remove_window()).is_ok() {
            return Ok(());
        }
    }
    let display = cx.displays().into_iter().find(|d| d.id() == display_id).ok_or_else(|| anyhow::anyhow!("Monitor disappeared"))?;
    let panel = cx.open_window(
        WindowOptions {
            display_id: Some(display_id),
            window_bounds: Some(WindowBounds::Windowed(display.bounds())),
            titlebar: None,
            app_id: Some("dojo".into()),
            window_background: WindowBackgroundAppearance::Transparent,
            kind: WindowKind::LayerShell(layer_shell::LayerShellOptions {
                namespace: "dojo-mesh-dock".into(),
                layer: layer_shell::Layer::Overlay,
                anchor: layer_shell::Anchor::TOP | layer_shell::Anchor::BOTTOM | layer_shell::Anchor::LEFT | layer_shell::Anchor::RIGHT,
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
                let polling = cx.spawn(async move |this, cx| {
                    loop {
                        // The snapshot probes peers and runs ssh-keygen, so it is read off the UI thread.
                        let data = cx.background_executor().spawn(async { dojo_mesh::panel::read() }).await;
                        let alive = this
                            .update(cx, |mesh: &mut Mesh, cx| {
                                mesh.data = Some(data);
                                cx.notify();
                                mesh.reveal.is_open()
                            })
                            .unwrap_or(false);
                        if !alive {
                            break;
                        }
                        cx.background_executor().timer(REFRESH).await;
                    }
                });
                Mesh {
                    data: None,
                    view: match tab {
                        crate::control::MeshTab::Machines => View::Machines,
                        crate::control::MeshTab::Requests => View::Requests,
                        crate::control::MeshTab::Keys => View::Keys,
                        crate::control::MeshTab::Health => View::Findings,
                    },
                    confirm: None,
                    dismissed: Default::default(),
                    job: None,
                    notice: None,
                    display: display_id,
                    focus,
                    reveal: dojo::design::MenuReveal::opening_for(Instant::now(), Duration::from_millis(150)),
                    _polling: polling,
                }
            })
        },
    )?;
    cx.update_global::<MeshUi, _>(|ui, _| ui.panel = Some(panel));
    Ok(())
}

/// What a dot says about a machine.
fn dot_color(colors: &ThemeColors, online: bool, expiring: bool) -> gpui::Rgba {
    if !online {
        colors.border.rgb()
    } else if expiring {
        gpui::rgb(dojo::design::ATTENTION)
    } else {
        colors.primary.rgb()
    }
}

/// The panel's rows, with the height they add up to, so the panel is sized for them and never scrolls.
struct Body {
    el: gpui::Div,
    height: f32,
    count: usize,
}

impl Body {
    fn new() -> Self {
        Self { el: div().flex().flex_col().gap(px(GAP)), height: 0., count: 0 }
    }

    fn push(&mut self, child: impl IntoElement, height: f32) {
        self.el = std::mem::replace(&mut self.el, div()).child(child);
        self.height += height + if self.count > 0 { GAP } else { 0. };
        self.count += 1;
    }

    fn finish(self) -> (gpui::Div, f32) {
        (self.el, self.height)
    }
}

/// A fingerprint split in two lines for comparing by eye: `SHA256:` dropped, the 43 characters halved.
fn fingerprint_lines(fingerprint: &str) -> (String, String) {
    let body = fingerprint.strip_prefix("SHA256:").unwrap_or(fingerprint);
    let half = body.len().div_ceil(2);
    (body[..half].to_string(), body[half..].to_string())
}

/// The start and end of a fingerprint, enough to tell keys apart in a list.
fn short_fingerprint(fingerprint: &str) -> String {
    let body = fingerprint.strip_prefix("SHA256:").unwrap_or(fingerprint);
    if body.len() <= 20 {
        return body.to_string();
    }
    format!("{}…{}", &body[..12], &body[body.len() - 6..])
}

impl Mesh {
    fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.reveal.set_open(false, Instant::now());
        cx.notify();
    }

    /// Runs `dojo-mesh ARGS` off the UI thread; the panel shows the key glyph while it signs.
    fn run(&mut self, args: Vec<String>, signs: bool, cx: &mut Context<Self>) {
        if self.job.is_some() {
            return;
        }
        self.job = Some(Job { signs });
        self.notice = None;
        self.confirm = None;
        let binary = mesh_binary();
        cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_executor()
                .spawn(async move { Command::new(binary).args(&args).stdin(std::process::Stdio::null()).output() })
                .await;
            let notice = match outcome {
                Ok(output) if output.status.success() => None,
                Ok(output) => Some(String::from_utf8_lossy(&output.stderr).lines().rev().find(|line| !line.trim().is_empty()).unwrap_or("it did not finish").trim().trim_start_matches("dojo-mesh: ").to_string()),
                Err(error) => Some(format!("dojo-mesh could not run: {error}")),
            };
            let _ = this.update(cx, |mesh: &mut Mesh, cx| {
                mesh.job = None;
                mesh.notice = notice;
                cx.notify();
            });
            // Show the change at once instead of at the next poll.
            let data = cx.background_executor().spawn(async { dojo_mesh::panel::read() }).await;
            let _ = this.update(cx, |mesh: &mut Mesh, cx| {
                mesh.data = Some(data);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Closes the panel first (it holds the keyboard exclusively) and hands the keyboard and cursor to `peer`.
    fn take_control(&mut self, peer: String, window: &mut Window, cx: &mut Context<Self>) {
        window.remove_window();
        std::thread::spawn(move || {
            // Let go of the click first.
            std::thread::sleep(Duration::from_millis(300));
            match Command::new(mesh_binary()).args(["remote", &peer]).output() {
                Ok(output) if !output.status.success() => eprintln!("Dojo mesh: remote input ended: {}", String::from_utf8_lossy(&output.stderr).trim()),
                Err(error) => eprintln!("Dojo mesh: could not start remote input: {error}"),
                Ok(_) => {}
            }
        });
        let _ = cx;
    }

    fn icon_button(
        &self,
        id: String,
        glyph: &'static str,
        label: &str,
        enabled: bool,
        tone: Option<gpui::Rgba>,
        cx: &mut Context<Self>,
        action: impl Fn(&mut Mesh, &mut Window, &mut Context<Mesh>) + 'static,
    ) -> gpui::AnyElement {
        let colors = *cx.global::<ThemeColors>();
        div()
            .id(id)
            .size(px(28.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(dojo::design::PILL_RADIUS))
            .text_size(px(15.))
            .text_color(if !enabled {
                colors.border.rgb()
            } else {
                tone.unwrap_or(colors.foreground.rgb())
            })
            .tooltip(dojo::design::tip(label.to_string()))
            .when(enabled, |el| {
                el.cursor_pointer().hover(|el| el.bg(colors.background_highlight.rgb())).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |mesh, _, window, cx| {
                        action(mesh, window, cx);
                        cx.stop_propagation();
                    }),
                )
            })
            .child(glyph)
            .into_any_element()
    }

    /// A header glyph with a dot when something is waiting behind it, lit while its view is open.
    fn badge_button(
        &self,
        id: &'static str,
        glyph: &'static str,
        label: &str,
        badge: bool,
        active: bool,
        cx: &mut Context<Self>,
        action: impl Fn(&mut Mesh, &mut Window, &mut Context<Mesh>) + 'static,
    ) -> gpui::AnyElement {
        let colors = *cx.global::<ThemeColors>();
        div()
            .id(id)
            .relative()
            .size(px(28.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(dojo::design::PILL_RADIUS))
            .text_size(px(15.))
            .when(active, |el| el.bg(colors.background_highlight.rgb()))
            .cursor_pointer()
            .tooltip(dojo::design::tip(label.to_string()))
            .hover(|el| el.bg(colors.background_highlight.rgb()))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |mesh, _, window, cx| {
                    action(mesh, window, cx);
                    cx.stop_propagation();
                }),
            )
            .child(glyph)
            .when(badge, |el| el.child(div().absolute().top(px(3.)).right(px(3.)).size(px(7.)).rounded(px(dojo::design::PILL_RADIUS)).bg(gpui::rgb(dojo::design::ATTENTION))))
            .into_any_element()
    }

    /// A machine asking to join: its name and where it is, the fingerprint to compare, and the one
    /// press (then a second) that approves it.
    fn request_card(&self, request: &Request, cx: &mut Context<Self>) -> gpui::AnyElement {
        let colors = *cx.global::<ThemeColors>();
        let idle = self.job.is_none();
        let confirming = self.confirm == Some(Confirm::Approve(request.file.clone()));
        let (first, second) = fingerprint_lines(&request.fingerprint);
        let file = request.file.clone();
        let args = {
            let mut args = vec!["approve".to_string(), request.file.display().to_string(), "--fingerprint".into(), request.fingerprint.clone(), "--days".into(), "90".into()];
            if request.replaces {
                args.push("--replace".into());
            }
            args
        };
        let mut sub = div().text_size(px(11.)).truncate();
        sub = if request.replaces {
            sub.text_color(gpui::rgb(dojo::design::ATTENTION)).child(format!("{} {}", glyph::ALERT, format!("replaces the {} already here", request.name)))
        } else {
            sub.text_color(colors.foreground_muted.rgb()).child(request.routes.join(", "))
        };
        let mut actions = div().flex().items_center().gap(px(2.)).flex_none();
        if confirming {
            actions = actions
                .child(self.icon_button(format!("mesh-approve-{}", file.display()), glyph::CHECK, "Approve", idle, Some(colors.primary.rgb()), cx, move |mesh, _, cx| {
                    mesh.run(args.clone(), true, cx);
                }))
                .child(self.icon_button(format!("mesh-approve-cancel-{}", request.file.display()), glyph::CLOSE, "Cancel", true, None, cx, |mesh, _, cx| {
                    mesh.confirm = None;
                    cx.notify();
                }));
        } else {
            let ask = request.file.clone();
            let away = request.file.clone();
            actions = actions
                .child(self.icon_button(format!("mesh-ask-{}", file.display()), glyph::CHECK, "Approve…", idle, None, cx, move |mesh, _, cx| {
                    mesh.confirm = Some(Confirm::Approve(ask.clone()));
                    cx.notify();
                }))
                .child(self.icon_button(format!("mesh-away-{}", request.file.display()), glyph::CLOSE, "Dismiss", true, None, cx, move |mesh, _, cx| {
                    mesh.dismissed.insert(away.clone());
                    cx.notify();
                }));
        }
        div()
            .id(format!("mesh-request-{}", request.file.display()))
            .h(px(REQUEST))
            .px(px(12.))
            .py(px(10.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .rounded(px(14.))
            .bg(colors.surface_raised.rgb())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(div().flex_1().min_w_0().flex().flex_col().child(div().text_size(px(14.)).truncate().child(if confirming { format!("Approve {}?", request.name) } else { request.name.clone() })).child(sub))
                    .child(actions),
            )
            .child(div().text_size(px(11.)).text_color(colors.foreground.rgb()).child(format!("{first}\n{second}")))
            .into_any_element()
    }

    fn signer_row(&self, signer: &Signer, cx: &mut Context<Self>) -> gpui::AnyElement {
        let colors = *cx.global::<ThemeColors>();
        let idle = self.job.is_none();
        let confirming = self.confirm == Some(Confirm::Untrust(signer.fingerprint.clone()));
        let fingerprint = signer.fingerprint.clone();
        let mut row = div()
            .id(format!("mesh-signer-{}", signer.fingerprint))
            .h(px(KEY_ROW))
            .px(px(12.))
            .flex()
            .items_center()
            .gap(px(10.))
            .rounded(px(14.))
            .bg(colors.surface_raised.rgb())
            .child(div().text_size(px(15.)).text_color(colors.primary.rgb()).child(if signer.hardware { glyph::USB } else { glyph::KEY }))
            .child(div().flex_1().min_w_0().text_size(px(12.)).truncate().child(if confirming { "Stop trusting this key?".to_string() } else { short_fingerprint(&signer.fingerprint) }));
        if confirming {
            let target = fingerprint.clone();
            row = row
                .child(self.icon_button(format!("mesh-untrust-{fingerprint}"), glyph::CHECK, "Stop trusting", idle, Some(gpui::rgb(dojo::design::PROBLEM)), cx, move |mesh, _, cx| {
                    mesh.run(vec!["trust".into(), "remove".into(), target.clone()], false, cx);
                }))
                .child(self.icon_button(format!("mesh-untrust-cancel-{fingerprint}"), glyph::CLOSE, "Cancel", true, None, cx, |mesh, _, cx| {
                    mesh.confirm = None;
                    cx.notify();
                }));
        } else {
            let target = fingerprint.clone();
            row = row.child(self.icon_button(format!("mesh-untrust-ask-{fingerprint}"), glyph::DELETE, "Stop trusting…", idle, None, cx, move |mesh, _, cx| {
                mesh.confirm = Some(Confirm::Untrust(target.clone()));
                cx.notify();
            }));
        }
        row.into_any_element()
    }

    /// A public key somebody left in the inbox: shown in full to compare with the machine that holds it.
    fn offer_row(&self, offer: &Offer, cx: &mut Context<Self>) -> gpui::AnyElement {
        let colors = *cx.global::<ThemeColors>();
        let idle = self.job.is_none();
        let confirming = self.confirm == Some(Confirm::Trust(offer.fingerprint.clone()));
        let (first, second) = fingerprint_lines(&offer.fingerprint);
        let (file, fingerprint) = (offer.file.display().to_string(), offer.fingerprint.clone());
        let mut actions = div().flex().items_center().gap(px(2.)).flex_none();
        if confirming {
            actions = actions
                .child(self.icon_button(format!("mesh-trust-{fingerprint}"), glyph::CHECK, "Trust this key", idle, Some(colors.primary.rgb()), cx, {
                    let (file, fingerprint) = (file.clone(), fingerprint.clone());
                    move |mesh, _, cx| mesh.run(vec!["trust".into(), "add".into(), file.clone(), "--fingerprint".into(), fingerprint.clone()], false, cx)
                }))
                .child(self.icon_button(format!("mesh-trust-cancel-{fingerprint}"), glyph::CLOSE, "Cancel", true, None, cx, |mesh, _, cx| {
                    mesh.confirm = None;
                    cx.notify();
                }));
        } else {
            let target = fingerprint.clone();
            actions = actions.child(self.icon_button(format!("mesh-trust-ask-{fingerprint}"), glyph::PLUS, "Trust this key…", idle, None, cx, move |mesh, _, cx| {
                mesh.confirm = Some(Confirm::Trust(target.clone()));
                cx.notify();
            }));
        }
        div()
            .id(format!("mesh-offer-{}", offer.fingerprint))
            .h(px(OFFER))
            .px(px(12.))
            .py(px(8.))
            .flex()
            .items_center()
            .gap(px(10.))
            .rounded(px(14.))
            .bg(colors.surface_raised.rgb())
            .child(div().text_size(px(15.)).text_color(colors.foreground_muted.rgb()).child(if offer.hardware { glyph::USB } else { glyph::KEY }))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().text_size(px(11.)).text_color(colors.foreground_muted.rgb()).child(if confirming { "Trust this key?" } else { "offered" }))
                    .child(div().text_size(px(11.)).child(format!("{first}\n{second}"))),
            )
            .child(actions)
            .into_any_element()
    }

    fn card(&self, data: &Panel, card: &dojo_mesh::panel::Card, cx: &mut Context<Self>) -> gpui::AnyElement {
        let colors = *cx.global::<ThemeColors>();
        let roster = data.kind == Kind::Roster;
        let idle = self.job.is_none();
        let reachable = card.online && data.blocked.is_none() && !card.this;
        let name = card.name.clone();
        let confirming = self.confirm == Some(Confirm::Remove(card.name.clone()));
        let sub = if card.this {
            "this machine".to_string()
        } else if !card.online {
            "offline".to_string()
        } else {
            match card.days_left {
                Some(days) if days < 0 => "expired".to_string(),
                Some(days) => format!("{days} days"),
                None => "online".to_string(),
            }
        };
        let sub_color = if card.expiring && roster { gpui::rgb(dojo::design::ATTENTION) } else { colors.foreground_muted.rgb() };
        let mut row = div()
            .id(format!("mesh-card-{}", card.name))
            .h(px(CARD))
            .px(px(12.))
            .flex()
            .items_center()
            .gap(px(10.))
            .rounded(px(14.))
            .bg(colors.surface_raised.rgb())
            .child(div().size(px(9.)).flex_none().rounded(px(dojo::design::PILL_RADIUS)).bg(dot_color(&colors, card.online, card.expiring && roster)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().text_size(px(14.)).truncate().child(if confirming { format!("Remove {}?", card.name) } else { card.name.clone() }))
                    .child(div().text_size(px(11.)).text_color(sub_color).truncate().child(sub)),
            );
        if card.this {
            return row.child(div().text_size(px(15.)).text_color(colors.foreground_muted.rgb()).child(glyph::LAPTOP)).into_any_element();
        }
        if confirming {
            let target = name.clone();
            row = row
                .child(self.icon_button(format!("mesh-confirm-{}", card.name), glyph::CHECK, "Remove", idle, Some(gpui::rgb(dojo::design::PROBLEM)), cx, move |mesh, _, cx| {
                    mesh.run(vec!["rm".into(), target.clone()], true, cx);
                }))
                .child(self.icon_button(format!("mesh-cancel-{}", card.name), glyph::CLOSE, "Cancel", true, None, cx, |mesh, _, cx| {
                    mesh.confirm = None;
                    cx.notify();
                }));
            return row.into_any_element();
        }
        let peer = name.clone();
        row = row.child(self.icon_button(format!("mesh-control-{}", card.name), glyph::KEYBOARD, "Take control", reachable && idle, None, cx, move |mesh, window, cx| {
            mesh.take_control(peer.clone(), window, cx);
        }));
        let peer = name.clone();
        row = row.child(self.icon_button(format!("mesh-send-{}", card.name), glyph::SEND, "Send a window", reachable && idle, None, cx, move |mesh, window, cx| {
            let display = mesh.display;
            window.remove_window();
            // The switch panel opens with this machine already chosen.
            if let Err(error) = crate::send_window_ui::toggle_for(display, Some(peer.clone()), false, cx) {
                eprintln!("Dojo mesh: could not open the send panel: {error:#}");
            }
        }));
        if roster && data.can_sign {
            let peer = name.clone();
            row = row.child(self.icon_button(format!("mesh-renew-{}", card.name), glyph::RENEW, "Renew", idle, if card.expiring { Some(gpui::rgb(dojo::design::ATTENTION)) } else { None }, cx, move |mesh, _, cx| {
                mesh.run(vec!["renew".into(), peer.clone()], true, cx);
            }));
            let peer = name;
            row = row.child(self.icon_button(format!("mesh-remove-{}", card.name), glyph::DELETE, "Remove", idle, None, cx, move |mesh, _, cx| {
                mesh.confirm = Some(Confirm::Remove(peer.clone()));
                cx.notify();
            }));
        }
        row.into_any_element()
    }
}

impl Render for Mesh {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = *cx.global::<ThemeColors>();
        let viewport = window.viewport_size();
        let (entry, animating) = self.reveal.sample_with_motion(Instant::now(), cx.reduce_motion());
        if animating || self.job.as_ref().is_some_and(|job| job.signs) {
            window.request_animation_frame();
        }
        if !self.reveal.is_open() && entry < 0.001 {
            window.remove_window();
        }
        let width = WIDTH.min((f32::from(viewport.width) - 32.).max(240.));
        let right = crate::geometry::RIGHT_INSET as f32;
        let top = crate::geometry::TOP_BAR_HEIGHT as f32;

        let mut body = Body::new();
        match (&self.data, self.view) {
            (None, _) => {
                body.push(div().h(px(CARD)).flex().items_center().text_size(px(12.)).text_color(colors.foreground_muted.rgb()).child("Reading the mesh…"), CARD);
            }
            (Some(data), View::Machines) => {
                for card in data.cards.iter().take(MAX_CARDS) {
                    body.push(self.card(data, card, cx), CARD);
                }
                if data.cards.len() > MAX_CARDS {
                    body.push(div().text_size(px(11.)).text_color(colors.foreground_muted.rgb()).child(format!("+{} more", data.cards.len() - MAX_CARDS)), LINE);
                }
                if data.cards.len() == 1 {
                    body.push(div().h(px(CARD)).flex().items_center().px(px(12.)).text_size(px(12.)).text_color(colors.foreground_muted.rgb()).child("No other machines yet"), CARD);
                }
            }
            (Some(data), View::Findings) => {
                if data.findings.is_empty() {
                    body.push(div().h(px(CARD)).flex().items_center().gap(px(10.)).px(px(12.)).child(div().text_size(px(16.)).text_color(colors.primary.rgb()).child(glyph::HEALTHY)).child(div().text_size(px(13.)).child("Healthy")), CARD);
                }
                for (index, finding) in data.findings.iter().take(8).enumerate() {
                    let tone = if finding.severity == Severity::Problem { gpui::rgb(dojo::design::PROBLEM) } else { gpui::rgb(dojo::design::ATTENTION) };
                    body.push(
                        div()
                            .id(format!("mesh-finding-{index}"))
                            .flex()
                            .gap(px(10.))
                            .px(px(12.))
                            .py(px(6.))
                            .rounded(px(12.))
                            .bg(colors.surface_raised.rgb())
                            .child(div().flex_none().text_size(px(14.)).text_color(tone).child(glyph::ALERT))
                            .child(div().flex_1().min_w_0().text_size(px(12.)).child(match &finding.device {
                                Some(device) => format!("{device}: {}", finding.message),
                                None => finding.message.clone(),
                            })),
                        44.,
                    );
                }
            }
            (Some(data), View::Requests) => {
                let pending: Vec<&Request> = data.requests.iter().filter(|request| !self.dismissed.contains(&request.file)).collect();
                if pending.is_empty() {
                    body.push(
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(px(6.))
                            .py(px(10.))
                            .text_color(colors.foreground_muted.rgb())
                            .child(div().text_size(px(22.)).child(glyph::INBOX))
                            .child(div().text_size(px(12.)).child("No requests"))
                            .child(div().text_size(px(10.)).child("Put a machine's .join.json in the mesh inbox or Downloads")),
                        96.,
                    );
                }
                for request in pending {
                    body.push(self.request_card(request, cx), REQUEST);
                }
                if data.skipped > 0 {
                    body.push(div().px(px(12.)).text_size(px(10.)).text_color(colors.foreground_muted.rgb()).child(format!("{} file(s) in the inbox were not taken: altered, unsigned or unreadable", data.skipped)), LINE);
                }
            }
            (Some(data), View::Keys) => {
                if data.signers.is_empty() {
                    body.push(div().h(px(CARD)).flex().items_center().px(px(12.)).text_size(px(12.)).text_color(colors.foreground_muted.rgb()).child("No signing keys are trusted"), CARD);
                }
                for signer in &data.signers {
                    body.push(self.signer_row(signer, cx), KEY_ROW);
                }
                for offer in &data.offers {
                    body.push(self.offer_row(offer, cx), OFFER);
                }
            }
        }
        if let Some(notice) = &self.notice {
            body.push(div().px(px(12.)).text_size(px(11.)).text_color(gpui::rgb(dojo::design::ATTENTION)).child(notice.clone()), LINE);
        }
        let (body, body_height) = body.finish();

        // Header: the name of the panel, then the glyphs that switch what it shows.
        let problems = self.data.as_ref().map_or(0, |d| d.findings.len());
        let worst = self.data.as_ref().and_then(|d| d.findings.iter().map(|f| f.severity).max_by_key(|s| matches!(s, Severity::Problem)));
        let health_tone = match worst {
            None => colors.primary.rgb(),
            Some(Severity::Problem) => gpui::rgb(dojo::design::PROBLEM),
            Some(Severity::Warning) => gpui::rgb(dojo::design::ATTENTION),
        };
        let locked = self.data.as_ref().is_some_and(|d| d.blocked.is_some());
        let signing = self.job.as_ref().is_some_and(|job| job.signs);
        let pulse = if signing { 0.55 + 0.45 * (self.reveal_clock() * 5.0).sin().abs() } else { 1. };
        let mut header = div()
            .h(px(HEADER))
            .flex()
            .items_center()
            .justify_between()
            .child(div().text_size(px(16.)).child("Mesh"));
        let mut tools = div().flex().items_center().gap(px(2.));
        if signing {
            tools = tools.child(div().size(px(28.)).flex().items_center().justify_center().text_size(px(15.)).text_color(colors.primary.rgb()).opacity(pulse).child(glyph::KEY));
        }
        if locked {
            tools = tools.child(div().size(px(28.)).flex().items_center().justify_center().text_size(px(15.)).text_color(gpui::rgb(dojo::design::ATTENTION)).child(glyph::LOCK));
        }
        if let Some(data) = &self.data {
            if data.kind == Kind::Roster {
                let waiting = data.requests.iter().filter(|r| !self.dismissed.contains(&r.file)).count();
                let in_requests = self.view == View::Requests;
                if data.can_sign || waiting > 0 {
                    tools = tools.child(self.badge_button("mesh-requests", glyph::INBOX, "Join requests", waiting > 0, in_requests, cx, move |mesh, _, cx| {
                        mesh.view = if in_requests { View::Machines } else { View::Requests };
                        mesh.confirm = None;
                        cx.notify();
                    }));
                }
                let in_keys = self.view == View::Keys;
                tools = tools.child(self.badge_button("mesh-keys", glyph::KEY, "Trusted keys", !data.offers.is_empty(), in_keys, cx, move |mesh, _, cx| {
                    mesh.view = if in_keys { View::Machines } else { View::Keys };
                    mesh.confirm = None;
                    cx.notify();
                }));
            }
        }
        let findings_open = self.view == View::Findings;
        tools = tools.child(self.icon_button(
            "mesh-health".into(),
            if problems == 0 { glyph::HEALTHY } else { glyph::ALERT },
            if findings_open { "Back to machines" } else { "Health" },
            true,
            Some(health_tone),
            cx,
            move |mesh, _, cx| {
                mesh.view = if findings_open { View::Machines } else { View::Findings };
                mesh.confirm = None;
                cx.notify();
            },
        ));
        header = header.child(tools);

        let content_height = PAD * 2. + HEADER + GAP + body_height;
        let height = content_height.min((f32::from(viewport.height) - top - 20.).max(100.));
        let bounds = Bounds::new(point(viewport.width - px(width + right), px(top)), gpui::size(px(width), px(height * entry)));
        let content = div()
            .p(px(PAD))
            .flex()
            .flex_col()
            .gap(px(GAP))
            .text_color(colors.foreground.rgb())
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(header)
            .child(body);

        div()
            .size_full()
            .font_family(crate::power::FONT_FAMILY)
            .font_features(crate::power::font_features())
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|mesh, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    if mesh.confirm.take().is_some() || mesh.view != View::Machines {
                        mesh.view = View::Machines;
                        cx.notify();
                    } else {
                        mesh.dismiss(cx);
                    }
                    cx.stop_propagation();
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |mesh, event: &gpui::MouseDownEvent, _, cx| {
                    if !bounds.contains(&event.position) {
                        mesh.dismiss(cx);
                    }
                }),
            )
            .child(dojo::design::shell_extension(bounds, colors.background.rgb(), colors.border.rgb(), true))
            .child(
                div()
                    .id("mesh-panel")
                    .absolute()
                    .top(px(top))
                    .right(px(right))
                    .w(px(width))
                    .h(px(height * entry))
                    .overflow_hidden()
                    .child(content.opacity(((entry - 0.25) / 0.65).clamp(0., 1.))),
            )
    }
}

impl Mesh {
    /// Seconds, for a pulse. Any steady clock will do.
    fn reveal_clock(&self) -> f32 {
        static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        START.get_or_init(Instant::now).elapsed().as_secs_f32()
    }
}
