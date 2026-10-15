use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::config::ThemeColors;
use crate::wifi::{Action, Availability, Controller, Network, NetworkId, Security, Snapshot};
use gpui::{
    App, AppContext, Bounds, Context, DisplayId, FocusHandle, IntoElement, KeyDownEvent,
    MouseButton, Render, Window, WindowBackgroundAppearance, WindowBounds, WindowHandle,
    WindowKind, WindowOptions, div, layer_shell, point, prelude::*, px,
};

const PANEL_WIDTH: f32 = 340.;
const PANEL_MAX_HEIGHT: f32 = 560.;
const NETWORK_REFRESH_INTERVAL: Duration = Duration::from_secs(15);
const MAX_PASSWORD_BYTES: usize = 128;

pub struct WifiUi {
    pub snapshot: Arc<Snapshot>,
    controller: Controller,
    panel: Option<(DisplayId, WindowHandle<Dock>)>,
    _updates: gpui::Task<()>,
}
impl gpui::Global for WifiUi {}

pub fn initialize(cx: &mut App) -> anyhow::Result<()> {
    let (controller, changes) = Controller::start()?;
    let updates_controller = controller.clone();
    let task = cx.spawn(async move |cx| {
        while changes.recv().await.is_ok() {
            let snapshot = updates_controller.snapshot();
            let _ = cx.update(|cx| cx.update_global::<WifiUi, _>(|ui, _| ui.snapshot = snapshot));
        }
    });
    cx.set_global(WifiUi {
        snapshot: controller.snapshot(),
        controller,
        panel: None,
        _updates: task,
    });
    Ok(())
}

pub fn close(cx: &mut App) {
    if cx.try_global::<WifiUi>().is_none() {
        return;
    }
    let (panel, controller) =
        cx.update_global::<WifiUi, _>(|ui, _| (ui.panel.take(), ui.controller.clone()));
    if let Some((_, panel)) = panel {
        let _ = panel.update(cx, |_, window, _| window.remove_window());
    }
    let _ = controller.request(Action::CancelPassword);
}

pub fn toggle(display_id: DisplayId, cx: &mut App) -> anyhow::Result<()> {
    if !cx.has_global::<WifiUi>() {
        initialize(cx)?;
    }
    let (previous, controller) =
        cx.update_global::<WifiUi, _>(|ui, _| (ui.panel.take(), ui.controller.clone()));
    let same_display = previous
        .as_ref()
        .is_some_and(|(display, _)| *display == display_id);
    let previous_removed = if let Some((_, previous)) = previous {
        let removed = previous
            .update(cx, |_, window, _| window.remove_window())
            .is_ok();
        let _ = controller.request(Action::CancelPassword);
        removed
    } else {
        false
    };
    if close_toggle_panel(same_display, previous_removed) {
        return Ok(());
    }

    let display = cx
        .displays()
        .into_iter()
        .find(|display| display.id() == display_id)
        .ok_or_else(|| anyhow::anyhow!("Wi-Fi output disappeared"))?;
    let panel_controller = controller.clone();
    let panel = cx.open_window(
        WindowOptions {
            display_id: Some(display_id),
            window_bounds: Some(WindowBounds::Windowed(display.bounds())),
            titlebar: None,
            app_id: Some("dojo".into()),
            window_background: WindowBackgroundAppearance::Transparent,
            kind: WindowKind::LayerShell(layer_shell::LayerShellOptions {
                namespace: "dojo-wifi-panel".into(),
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
                let observer = cx.observe_global::<WifiUi>(|_, cx| cx.notify());
                let refresh_controller = panel_controller.clone();
                let refresh_task = cx.spawn(async move |_, cx| {
                    loop {
                        cx.background_executor()
                            .timer(NETWORK_REFRESH_INTERVAL)
                            .await;
                        if refresh_controller.refresh().is_err() {
                            // The controller publishes the durable error; polling must stay nonfatal.
                        }
                    }
                });
                Dock {
                    controller: panel_controller.clone(),
                    password_network: None,
                    submitted_password_for: None,
                    password: String::new(),
                    select_all: false,
                    local_error: None,
                    focus,
                    reveal: crate::design::MenuReveal::opening_for(
                        Instant::now(),
                        Duration::from_millis(150),
                    ),
                    _observer: observer,
                    _refresh_task: refresh_task,
                }
            })
        },
    )?;
    cx.update_global::<WifiUi, _>(|ui, _| ui.panel = Some((display_id, panel)));
    let _ = controller.refresh();
    Ok(())
}

struct Dock {
    controller: Controller,
    password_network: Option<NetworkId>,
    submitted_password_for: Option<NetworkId>,
    password: String,
    select_all: bool,
    local_error: Option<String>,
    focus: FocusHandle,
    reveal: crate::design::MenuReveal,
    _observer: gpui::Subscription,
    _refresh_task: gpui::Task<()>,
}
impl Dock {
    fn clear_password(&mut self) {
        self.password.clear();
        self.select_all = false;
        self.password_network = None;
    }

    fn request(&mut self, action: Action, cx: &mut Context<Self>) {
        self.local_error = self.controller.request(action).err();
        cx.notify();
    }

    fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.clear_password();
        let _ = self.controller.request(Action::CancelPassword);
        self.reveal.set_open(false, Instant::now());
        cx.notify();
    }

    fn sync_password_prompt(&mut self, snapshot: &Snapshot) {
        if snapshot.password_required.is_none() {
            self.submitted_password_for = None;
        }
        self.password_network = password_prompt_target(
            self.password_network.as_ref(),
            snapshot.password_required.as_ref(),
            self.submitted_password_for.as_ref(),
        );
    }

    fn cancel_password(&mut self, cx: &mut Context<Self>) {
        if let Some(network) = cx.global::<WifiUi>().snapshot.password_required.as_ref() {
            self.submitted_password_for = Some(network.clone());
        }
        self.clear_password();
        self.request(Action::CancelPassword, cx);
    }

    fn begin_password(&mut self, network: &Network, cx: &mut Context<Self>) {
        self.password.clear();
        self.password_network = Some(network.id.clone());
        self.local_error = None;
        cx.notify();
    }

    fn submit_password(&mut self, cx: &mut Context<Self>) {
        let Some(network) = self.password_network.take() else {
            return;
        };
        let password = std::mem::take(&mut self.password);
        self.select_all = false;
        match self
            .controller
            .request(Action::Connect(network.clone(), Some(password)))
        {
            Ok(()) => {
                self.submitted_password_for = Some(network);
                self.local_error = None;
            }
            Err(error) => {
                self.password_network = Some(network);
                self.local_error = Some(error);
            }
        }
        cx.notify();
    }

    fn handle_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        if key == "escape" {
            self.dismiss(cx);
            window.prevent_default();
            cx.stop_propagation();
            return;
        }
        let snapshot = cx.global::<WifiUi>().snapshot.clone();
        self.sync_password_prompt(&snapshot);

        if self.password_network.is_none() {
            return;
        }
        let modifiers = event.keystroke.modifiers;
        match key {
            "enter" if !event.is_held => self.submit_password(cx),
            "backspace" => {
                if self.select_all {
                    self.password.clear();
                } else {
                    self.password.pop();
                }
                self.select_all = false;
                cx.notify();
            }
            "a" if modifiers.control => {
                self.select_all = true;
                cx.notify();
            }
            "v" if modifiers.control => {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    self.insert_password(&text.replace(['\n', '\r'], " "));
                    cx.notify();
                }
            }
            _ => {
                if let Some(chars) = &event.keystroke.key_char {
                    let chars = chars
                        .chars()
                        .filter(|ch| !ch.is_control())
                        .collect::<String>();
                    self.insert_password(&chars);
                    cx.notify();
                } else {
                    return;
                }
            }
        }
        window.prevent_default();
        cx.stop_propagation();
    }

    fn insert_password(&mut self, text: &str) {
        insert_password(&mut self.password, &mut self.select_all, text);
    }

    fn action_button(
        &self,
        id: impl Into<String>,
        label: impl Into<String>,
        action: impl Fn() -> Action + 'static,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let colors = *cx.global::<ThemeColors>();
        div()
            .id(id.into())
            .px(px(11.))
            .py(px(7.))
            .rounded(px(16.))
            .text_size(px(12.))
            .bg(colors.background.rgb())
            .text_color(if enabled {
                colors.foreground.rgb()
            } else {
                colors.foreground_muted.rgb()
            })
            .when(enabled, |button| {
                button.cursor_pointer().on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |view, _, _, cx| {
                        view.request(action(), cx);
                        cx.stop_propagation();
                    }),
                )
            })
            .child(label.into())
            .into_any_element()
    }

    fn network_row(
        &self,
        network: &Network,
        row_key: usize,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let colors = *cx.global::<ThemeColors>();
        let state = if network.connected {
            "Connected"
        } else if network.saved {
            "Saved"
        } else {
            "Nearby"
        };
        let status = format!("{state} · {}%", network.signal);
        let security = match &network.security {
            Security::Open => "Open",
            Security::Personal => "Secure",
            Security::Enterprise => "Enterprise",
            Security::Unsupported => "Unsupported security",
        };
        let row = div()
            .id(format!("wifi-network-{row_key}"))
            .flex_shrink_0()
            .w_full()
            .px(px(12.))
            .py(px(9.))
            .rounded(px(12.))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(8.))
            .bg(if network.connected {
                colors.surface_raised.rgb()
            } else {
                colors.background.rgb()
            })
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(colors.foreground.rgb())
                            .overflow_hidden()
                            .child(network.label.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(colors.foreground_muted.rgb())
                            .child(format!("{status} · {security}")),
                    ),
            );
        if !network.connected
            && matches!(
                &network.security,
                Security::Enterprise | Security::Unsupported
            )
        {
            return row
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(colors.foreground_muted.rgb())
                        .child(if network.security == Security::Enterprise {
                            "Enterprise sign-in is unavailable"
                        } else {
                            "Unsupported network security"
                        }),
                )
                .into_any_element();
        }
        if network.connected {
            return row
                .child(self.action_button(
                    format!("wifi-disconnect-{row_key}"),
                    "Disconnect",
                    || Action::Disconnect,
                    true,
                    cx,
                ))
                .into_any_element();
        }
        let action_id = row_key;
        let network_for_click = network.clone();
        let label = if network.saved {
            "Connect"
        } else if network.security == Security::Personal {
            "Enter password"
        } else {
            "Connect"
        };
        row.child(
            div()
                .id(format!("wifi-connect-{action_id}"))
                .px(px(11.))
                .py(px(7.))
                .rounded(px(16.))
                .text_size(px(12.))
                .bg(colors.background.rgb())
                .text_color(colors.foreground.rgb())
                .cursor_pointer()
                .child(label)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |view, _, _, cx| {
                        if opens_local_password_prompt(
                            network_for_click.security,
                            network_for_click.saved,
                        ) {
                            view.begin_password(&network_for_click, cx);
                        } else {
                            view.request(Action::Connect(network_for_click.id.clone(), None), cx);
                        }
                        cx.stop_propagation();
                    }),
                ),
        )
        .into_any_element()
    }
}

fn close_toggle_panel(same_display: bool, previous_removed: bool) -> bool {
    same_display && previous_removed
}

fn opens_local_password_prompt(security: Security, saved: bool) -> bool {
    security == Security::Personal && !saved
}

fn password_prompt_target<T: Clone + Eq>(
    current: Option<&T>,
    requested: Option<&T>,
    suppressed: Option<&T>,
) -> Option<T> {
    current
        .or_else(|| requested.filter(|requested| Some(*requested) != suppressed))
        .cloned()
}

fn insert_password(password: &mut String, select_all: &mut bool, text: &str) {
    if *select_all {
        password.clear();
    }
    *select_all = false;
    let available = MAX_PASSWORD_BYTES.saturating_sub(password.len());
    let end = text
        .char_indices()
        .map(|(index, ch)| index + ch.len_utf8())
        .take_while(|end| *end <= available)
        .last()
        .unwrap_or(0);
    password.push_str(&text[..end]);
}

impl Render for Dock {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (progress, moving) = self
            .reveal
            .sample_with_motion(Instant::now(), cx.reduce_motion());
        if moving {
            window.request_animation_frame();
        } else if !self.reveal.is_open() {
            window.remove_window();
        }
        let colors = *cx.global::<ThemeColors>();
        let snapshot = cx.global::<WifiUi>().snapshot.clone();
        self.sync_password_prompt(&snapshot);
        let viewport = window.viewport_size();
        let width = PANEL_WIDTH.min((f32::from(viewport.width) - 16.).max(1.));
        let max_height = PANEL_MAX_HEIGHT.min((f32::from(viewport.height) - 32.).max(1.));
        let panel_top = crate::geometry::TOP_BAR_HEIGHT as f32;
        let right = crate::geometry::RIGHT_INSET as f32;
        let panel_bounds = Bounds {
            origin: point(px(f32::from(viewport.width) - width - right), px(panel_top)),
            size: gpui::Size::new(px(width), px(max_height)),
        };
        let powered = matches!(snapshot.availability, Availability::Enabled);
        let mut content = div()
            .w(px(width))
            .h(px(max_height))
            .flex()
            .flex_col()
            .gap(px(10.))
            .p(px(14.))
            .opacity(((progress - 0.25) / 0.65).clamp(0., 1.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(
                                div()
                                    .text_size(px(14.))
                                    .text_color(colors.foreground.rgb())
                                    .child(snapshot.connection_title()),
                            )
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(colors.foreground_muted.rgb())
                                    .child(snapshot.connection_label()),
                            )
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(colors.foreground_muted.rgb())
                                    .child(availability_label(&snapshot.availability)),
                            ),
                    )
                    .child(self.action_button(
                        "wifi-toggle",
                        if powered { "Wi-Fi on" } else { "Wi-Fi off" },
                        move || Action::SetEnabled(!powered),
                        !matches!(snapshot.availability, Availability::Unavailable(_)),
                        cx,
                    )),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(colors.foreground.rgb())
                            .child("Available networks"),
                    )
                    .child(self.action_button(
                        "wifi-scan",
                        if snapshot.scanning {
                            "Scanning…"
                        } else {
                            "Scan"
                        },
                        || Action::Scan,
                        powered,
                        cx,
                    )),
            );

        if let Some(error) = snapshot
            .error
            .as_ref()
            .or(self.local_error.as_ref())
            .or(snapshot.observation_error.as_ref())
        {
            content = content.child(
                div()
                    .w_full()
                    .p(px(9.))
                    .rounded(px(10.))
                    .bg(colors.surface_raised.rgb())
                    .text_size(px(11.))
                    .text_color(colors.foreground.rgb())
                    .child(error.clone()),
            );
        }

        if let Some(network_id) = self.password_network.as_ref() {
            let requested = snapshot.password_required.as_ref() == Some(network_id);
            let prompt_title = if requested {
                "Password required"
            } else {
                "Connect to secure network"
            };
            content = content.child(
                div()
                    .w_full()
                    .p(px(12.))
                    .rounded(px(12.))
                    .bg(colors.surface_raised.rgb())
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(colors.foreground.rgb())
                            .child(prompt_title),
                    )
                    .child(
                        div()
                            .id("wifi-password-input")
                            .w_full()
                            .px(px(10.))
                            .py(px(8.))
                            .rounded(px(8.))
                            .bg(colors.background.rgb())
                            .text_size(px(13.))
                            .text_color(colors.foreground.rgb())
                            .child(if self.password.is_empty() {
                                "Password".to_owned()
                            } else {
                                masked_password(&self.password)
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .child(
                                div()
                                    .id("wifi-password-cancel")
                                    .px(px(11.))
                                    .py(px(7.))
                                    .rounded(px(16.))
                                    .text_size(px(12.))
                                    .text_color(colors.foreground.rgb())
                                    .cursor_pointer()
                                    .child("Cancel")
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|view, _, _, cx| {
                                            view.cancel_password(cx);
                                            cx.stop_propagation();
                                        }),
                                    ),
                            )
                            .child(
                                div()
                                    .id("wifi-password-connect")
                                    .px(px(11.))
                                    .py(px(7.))
                                    .rounded(px(16.))
                                    .text_size(px(12.))
                                    .bg(colors.primary.rgb())
                                    .text_color(colors.selection_foreground.rgb())
                                    .cursor_pointer()
                                    .child("Connect")
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|view, _, _, cx| {
                                            view.submit_password(cx);
                                            cx.stop_propagation();
                                        }),
                                    ),
                            ),
                    ),
            );
        }

        let mut networks = snapshot.networks.iter().collect::<Vec<_>>();
        networks.sort_by(network_order);
        if networks.is_empty() {
            content = content.child(
                div()
                    .w_full()
                    .p(px(12.))
                    .text_size(px(11.))
                    .text_color(colors.foreground_muted.rgb())
                    .child(match &snapshot.availability {
                        Availability::Unavailable(_) => "NetworkManager is unavailable.",
                        Availability::Disabled => "Turn Wi-Fi on to find networks.",
                        Availability::Enabled if snapshot.scanning => "Looking for networks…",
                        Availability::Enabled => "No networks found. Scan to look again.",
                    }),
            );
        } else {
            let show_scroll_hint = networks.len() > 6;
            let mut list = div()
                .id("wifi-network-list")
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap(px(6.));
            for (row_key, network) in networks.into_iter().enumerate() {
                list = list.child(self.network_row(network, row_key, cx));
            }
            content = content.child(list);
            if show_scroll_hint {
                content = content.child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(10.))
                        .text_color(colors.foreground_muted.rgb())
                        .child("Scroll for more networks"),
                );
            }
        }

        div()
            .size_full()
            .font_family(crate::power::FONT_FAMILY)
            .font_features(crate::power::font_features())
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|view, event: &KeyDownEvent, window, cx| {
                view.handle_key(event, window, cx);
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |view, event: &gpui::MouseDownEvent, _, cx| {
                    if !panel_bounds.contains(&event.position) {
                        view.dismiss(cx);
                    }
                }),
            )
            .child(crate::design::shell_extension(
                Bounds::new(
                    panel_bounds.origin,
                    gpui::size(px(width), px(max_height * progress)),
                ),
                colors.background.rgb(),
                true,
            ))
            .child(
                div()
                    .id("wifi-panel")
                    .absolute()
                    .top(px(panel_top))
                    .right(px(right))
                    .w(px(width))
                    .h(px(max_height * progress))
                    .overflow_hidden()
                    .child(content),
            )
    }
}

fn availability_label(availability: &Availability) -> String {
    match availability {
        Availability::Unavailable(reason) => reason.clone(),
        Availability::Disabled => "Wi-Fi is off".into(),
        Availability::Enabled => "Wi-Fi is on".into(),
    }
}

fn network_order(left: &&Network, right: &&Network) -> std::cmp::Ordering {
    (!left.connected)
        .cmp(&(!right.connected))
        .then_with(|| right.signal.cmp(&left.signal))
        .then_with(|| left.label.to_lowercase().cmp(&right.label.to_lowercase()))
}

fn masked_password(password: &str) -> String {
    "•".repeat(password.chars().count())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_display_never_contains_entered_text() {
        let secret = "private-network-passphrase";
        let display = masked_password(secret);
        assert_eq!(display, "•".repeat(secret.chars().count()));
        assert!(!display.contains(secret));
    }

    #[test]
    fn stale_same_display_handle_reopens_instead_of_closing_a_missing_panel() {
        assert!(close_toggle_panel(true, true));
        assert!(!close_toggle_panel(true, false));
        assert!(!close_toggle_panel(false, true));
    }

    #[test]
    fn saved_personal_profiles_skip_the_local_password_prompt() {
        assert!(!opens_local_password_prompt(Security::Personal, true));
        assert!(opens_local_password_prompt(Security::Personal, false));
        assert!(!opens_local_password_prompt(Security::Open, false));
    }

    #[test]
    fn network_manager_prompt_reopens_after_submit_but_not_while_stale() {
        assert_eq!(password_prompt_target(None, Some(&7), None), Some(7));
        assert_eq!(password_prompt_target(None, Some(&7), Some(&7)), None);
        assert_eq!(password_prompt_target(Some(&8), Some(&7), None), Some(8));
        assert_eq!(password_prompt_target(None, None, Some(&7)), None);
    }

    #[test]
    fn password_insertion_honors_select_all_and_byte_limit() {
        let mut password = "old".to_owned();
        let mut select_all = true;
        insert_password(&mut password, &mut select_all, "new");
        assert_eq!(password, "new");
        insert_password(
            &mut password,
            &mut select_all,
            &"x".repeat(MAX_PASSWORD_BYTES + 4),
        );
        assert_eq!(password.len(), MAX_PASSWORD_BYTES);
    }
}
