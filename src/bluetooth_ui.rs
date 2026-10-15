use std::sync::Arc;
use std::time::Instant;

use crate::bluetooth::{Action, Availability, Controller, Device, DeviceId, PromptKind, Snapshot};
use crate::config::ThemeColors;
use gpui::{
    App, AppContext, Bounds, Context, DisplayId, FocusHandle, IntoElement, KeyDownEvent,
    MouseButton, Render, Window, WindowBackgroundAppearance, WindowBounds, WindowHandle,
    WindowKind, WindowOptions, div, layer_shell, point, prelude::*, px,
};

pub struct BluetoothUi {
    pub snapshot: Arc<Snapshot>,
    controller: Controller,
    panel: Option<WindowHandle<Dock>>,
    _updates: gpui::Task<()>,
}
impl gpui::Global for BluetoothUi {}

pub fn initialize(cx: &mut App) {
    let (controller, changes) = Controller::start().expect("start Bluetooth controller");
    install(cx, controller, changes);
}
fn install(cx: &mut App, controller: Controller, changes: async_channel::Receiver<()>) {
    let updates_controller = controller.clone();
    let task = cx.spawn(async move |cx| {
        while changes.recv().await.is_ok() {
            let snapshot = updates_controller.snapshot();
            let _ =
                cx.update(|cx| cx.update_global::<BluetoothUi, _>(|ui, _| ui.snapshot = snapshot));
        }
    });
    cx.set_global(BluetoothUi {
        snapshot: controller.snapshot(),
        controller,
        panel: None,
        _updates: task,
    });
}

pub fn close(cx: &mut App) {
    let (panel, controller) =
        cx.update_global::<BluetoothUi, _>(|ui, _| (ui.panel.take(), ui.controller.clone()));
    if let Some(panel) = panel {
        let _ = panel.update(cx, |_, window, _| window.remove_window());
    }
    controller.close_panel();
}
pub fn toggle(display_id: DisplayId, cx: &mut App) -> anyhow::Result<()> {
    let (previous, controller) =
        cx.update_global::<BluetoothUi, _>(|ui, _| (ui.panel.take(), ui.controller.clone()));
    if let Some(panel) = previous {
        if panel
            .update(cx, |view, _, cx| {
                if view.reveal.is_open() {
                    view.dismiss(cx);
                } else {
                    view.reveal.set_open(true, Instant::now());
                    cx.notify();
                }
            })
            .is_ok()
        {
            cx.update_global::<BluetoothUi, _>(|ui, _| ui.panel = Some(panel));
            return Ok(());
        }
    }
    let display = cx
        .displays()
        .into_iter()
        .find(|d| d.id() == display_id)
        .ok_or_else(|| anyhow::anyhow!("Bluetooth output disappeared"))?;
    let panel = cx.open_window(
        WindowOptions {
            display_id: Some(display_id),
            window_bounds: Some(WindowBounds::Windowed(display.bounds())),
            titlebar: None,
            app_id: Some("dojo".into()),
            window_background: WindowBackgroundAppearance::Transparent,
            kind: WindowKind::LayerShell(layer_shell::LayerShellOptions {
                namespace: "dojo-bluetooth-dock".into(),
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
                let observer = cx.observe_global::<BluetoothUi>(|_, cx| cx.notify());
                let release_controller = controller.clone();
                cx.on_release(move |_, _| release_controller.close_panel())
                    .detach();
                Dock {
                    controller,
                    expanded: None,
                    show_nearby: false,
                    show_unnamed: false,
                    input: String::new(),
                    input_prompt: None,
                    error: None,
                    focus,
                    reveal: crate::design::MenuReveal::opening_for(
                        Instant::now(),
                        std::time::Duration::from_millis(150),
                    ),
                    content_height: 280.,
                    _observer: observer,
                }
            })
        },
    )?;
    cx.update_global::<BluetoothUi, _>(|ui, _| ui.panel = Some(panel));
    Ok(())
}

struct Dock {
    controller: Controller,
    expanded: Option<DeviceId>,
    show_nearby: bool,
    show_unnamed: bool,
    input: String,
    input_prompt: Option<u64>,
    error: Option<String>,
    focus: FocusHandle,
    reveal: crate::design::MenuReveal,
    content_height: f32,
    _observer: gpui::Subscription,
}
impl Dock {
    fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.reveal.set_open(false, Instant::now());
        self.controller.close_panel();
        cx.notify();
    }

    fn submit(&mut self, action: Action, cx: &mut Context<Self>) {
        self.error = self.controller.submit(action).err().map(|e| e.to_string());
        cx.notify();
    }
    fn action_button(
        &self,
        id: &str,
        label: &str,
        action: Action,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let colors = *cx.global::<ThemeColors>();
        div()
            .id(id.to_owned())
            .px(px(11.))
            .py(px(7.))
            .rounded(px(18.))
            .text_size(px(11.))
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
                        view.submit(action.clone(), cx);
                        cx.stop_propagation();
                    }),
                )
            })
            .child(label.to_owned())
            .into_any_element()
    }
    fn tile(
        &self,
        device: &Device,
        width: f32,
        snapshot: &Snapshot,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let colors = *cx.global::<ThemeColors>();
        let id = device.id.clone();
        let expanded = self.expanded.as_ref() == Some(&id);
        let pending = snapshot.pending.get(&id);
        let action = if device.connected {
            Action::Connect(id.clone(), false)
        } else if device.paired {
            Action::Connect(id.clone(), true)
        } else {
            Action::Pair(id.clone())
        };
        let label = pending.map_or_else(
            || {
                if device.connected {
                    "Disconnect"
                } else if device.paired {
                    "Connect"
                } else {
                    "Pair"
                }
            },
            String::as_str,
        );
        let status = pending.cloned().unwrap_or_else(|| {
            let state = if device.connected {
                "Connected"
            } else if device.paired {
                "Saved"
            } else {
                "Nearby"
            };
            device.battery.map_or_else(
                || state.to_owned(),
                |battery| format!("{state} · {battery}%"),
            )
        });
        let mut tile = div()
            .id(format!("bluetooth-device-{}", device.address))
            .w(px(width))
            .flex_shrink_0()
            .p(px(12.))
            .rounded(px(16.))
            .border_1()
            .border_color(if device.connected {
                colors.primary.rgb().opacity(0.35)
            } else {
                colors.border.rgb().opacity(0.15)
            })
            .bg(if device.connected {
                colors.surface_raised.rgb()
            } else {
                colors.background.rgb()
            })
            .flex()
            .flex_col()
            .gap(px(10.))
            .cursor_pointer()
            .hover(|style| style.border_color(colors.primary.rgb().opacity(0.5)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |view, _, _, cx| {
                    view.expanded = if view.expanded.as_ref() == Some(&id) {
                        None
                    } else {
                        Some(id.clone())
                    };
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        div()
                            .w(px(32.))
                            .flex_shrink_0()
                            .text_size(px(22.))
                            .text_color(if device.connected {
                                colors.primary.rgb()
                            } else {
                                colors.foreground_muted.rgb()
                            })
                            .child(device_glyph(&device.icon).to_string()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .truncate()
                                    .child(device.name.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(if device.connected {
                                        colors.primary.rgb()
                                    } else {
                                        colors.foreground_muted.rgb()
                                    })
                                    .child(status),
                            ),
                    )
                    .child(self.action_button(
                        &format!("bluetooth-action-{}", device.address),
                        label,
                        action,
                        pending.is_none() && snapshot.adapters.iter().any(|a| a.powered),
                        cx,
                    )),
            );
        if expanded {
            tile = tile.child(
                div()
                    .text_size(px(10.))
                    .text_color(colors.foreground_muted.rgb())
                    .child(device.address.clone()),
            );
        }
        if pending.is_some_and(|label| label == "Pairing…") {
            tile = tile.child(self.action_button(
                &format!("bluetooth-cancel-{}", device.address),
                "Cancel pairing",
                Action::Cancel(device.id.clone()),
                true,
                cx,
            ));
        }
        tile.into_any_element()
    }
}
fn unnamed_device(device: &Device) -> bool {
    device.name.eq_ignore_ascii_case(&device.address)
        || device
            .name
            .replace('-', ":")
            .eq_ignore_ascii_case(&device.address)
}
fn device_glyph(icon: &str) -> char {
    match icon {
        "audio-headphones" | "audio-headset" => '\u{f02cb}',
        "audio-card" | "audio-speakers" => '\u{f04c3}',
        "input-keyboard" => '\u{f030c}',
        "input-mouse" => '\u{f037d}',
        "input-gaming" => '\u{f0296}',
        "phone" => '\u{f011c}',
        "computer" => '\u{f0322}',
        _ => '\u{f00af}',
    }
}
impl Render for Dock {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = *cx.global::<ThemeColors>();
        let snapshot = cx.global::<BluetoothUi>().snapshot.clone();
        let viewport = window.viewport_size();
        let width = 360_f32.min(f32::from(viewport.width) - 24.).max(220.);
        let right = 12.;
        let now = Instant::now();
        let (entry, moving) = self.reveal.sample_with_motion(now, cx.reduce_motion());
        if moving {
            window.request_animation_frame();
        }
        if !moving && !self.reveal.is_open() {
            window.remove_window();
        }
        let top = crate::geometry::TOP_BAR_HEIGHT as f32;
        let max_height = (f32::from(viewport.height) - top - 12.).max(100.);
        let height = self.content_height.min(max_height);
        let measured_view = cx.entity().downgrade();
        let bounds = Bounds::new(
            point(viewport.width - px(width + right), px(top)),
            gpui::size(px(width), px(height * entry)),
        );
        let mut content = div()
            .on_children_prepainted(move |children, _, cx| {
                if let Some(bounds) = children.into_iter().reduce(|a, b| a.union(&b)) {
                    let natural_height = f32::from(bounds.size.height) + 46.;
                    let _ = measured_view.update(cx, |view, cx| {
                        if (view.content_height - natural_height).abs() > 0.5 {
                            view.content_height = natural_height;
                            cx.notify();
                        }
                    });
                }
            })
            .id("bluetooth-dock-content")
            .absolute()
            .top(px(0.))
            .left(px(0.))
            .w(px(width))
            .p(px(20.))
            .pb(px(26.))
            .text_color(colors.foreground.rgb())
            .opacity(((entry - 0.25) / 0.65).clamp(0., 1.))
            .flex()
            .flex_col()
            .gap(px(14.))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div().flex().items_center().justify_between().child(
                    div()
                        .flex()
                        .gap(px(9.))
                        .items_center()
                        .child(
                            div()
                                .text_size(px(18.))
                                .text_color(colors.primary.rgb())
                                .child("\u{f00af}"),
                        )
                        .child(div().text_size(px(14.)).child("Bluetooth")),
                ),
            );
        if let Some(prompt) = &snapshot.prompt {
            if self.input_prompt != Some(prompt.token) {
                self.input_prompt = Some(prompt.token);
                self.input.clear();
            }
            let device = snapshot
                .devices
                .iter()
                .find(|d| d.id == prompt.device)
                .map_or("Device", |d| d.name.as_str());
            let mut pairing = div()
                .p(px(14.))
                .rounded(px(18.))
                .border_1()
                .border_color(colors.primary.rgb().opacity(0.6))
                .bg(colors.surface_raised.rgb())
                .flex()
                .flex_col()
                .gap(px(10.))
                .text_size(px(12.))
                .child(format!("Pair with {device}"));
            let mut input = false;
            match &prompt.kind {
                PromptKind::Confirm(code) => {
                    pairing = pairing
                        .child("Do the codes match?")
                        .child(div().text_size(px(24.)).child(format!("{code:06}")));
                }
                PromptKind::Pin => {
                    pairing = pairing.child("Enter the device PIN");
                    input = true;
                }
                PromptKind::Passkey => {
                    pairing = pairing.child("Enter the device passkey");
                    input = true;
                }
                PromptKind::Authorize(message) => pairing = pairing.child(message.clone()),
                PromptKind::Display { code, entered } => {
                    pairing = pairing
                        .child("Enter this code on the device, then press Enter there.")
                        .child(div().text_size(px(24.)).child(code.clone()))
                        .child(format!("{entered} digits entered"))
                }
            }
            if input {
                pairing = pairing.child(
                    div()
                        .p(px(10.))
                        .rounded(px(8.))
                        .bg(colors.background.rgb())
                        .text_size(px(18.))
                        .child(if self.input.is_empty() {
                            "Type here…".into()
                        } else {
                            self.input.clone()
                        }),
                );
            }
            if !matches!(prompt.kind, PromptKind::Display { .. }) {
                pairing = pairing.child(self.action_button(
                    "bluetooth-confirm",
                    if input { "Continue" } else { "Confirm" },
                    Action::Answer(prompt.token, Some(self.input.clone())),
                    true,
                    cx,
                ));
            }
            pairing = pairing.child(self.action_button(
                "bluetooth-reject",
                "Cancel",
                Action::Cancel(prompt.device.clone()),
                true,
                cx,
            ));
            content = content.child(pairing);
        }
        match snapshot.availability {
            Availability::Starting => {
                content = content.child(div().text_size(px(12.)).child("Checking Bluetooth…"))
            }
            Availability::ServiceMissing | Availability::BusUnavailable => {
                content = content.child(div().text_size(px(12.)).child("Bluetooth isn't set up yet."))
                    .child(div().text_size(px(11.)).text_color(colors.foreground_muted.rgb()).child("The system Bluetooth service is missing or unavailable. Run the setup command in a terminal, then this dock will update automatically."))
                    .child(div().p(px(12.)).rounded(px(10.)).bg(colors.surface_raised.rgb()).text_size(px(10.)).child("bash ~/Dojo/scripts/setup-bluetooth.sh"));
            }
            Availability::NoAdapter => {
                content = content.child(
                    div()
                        .text_size(px(12.))
                        .child("No Bluetooth adapter found."),
                )
            }
            Availability::Ready => {
                for adapter in &snapshot.adapters {
                    let action = Action::Power(adapter.id.clone(), !adapter.powered);
                    content = content.child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(10.))
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(colors.foreground_muted.rgb())
                                    .child(adapter.name.clone()),
                            )
                            .child(self.action_button(
                                "bluetooth-power",
                                if snapshot.power_pending.contains(&adapter.id) {
                                    "Changing…"
                                } else if adapter.powered {
                                    "Turn off"
                                } else {
                                    "Turn on"
                                },
                                action,
                                !snapshot.power_pending.contains(&adapter.id),
                                cx,
                            )),
                    );
                }
                let mut saved = snapshot
                    .devices
                    .iter()
                    .filter(|d| d.paired || d.connected)
                    .collect::<Vec<_>>();
                saved.sort_by_key(|d| (!d.connected, d.name.to_lowercase()));
                content = content.child(
                    div()
                        .text_size(px(11.))
                        .text_color(colors.foreground_muted.rgb())
                        .child("Your devices"),
                );
                if saved.is_empty() {
                    content = content.child(
                        div()
                            .text_size(px(12.))
                            .child("Pair a device to keep it here."),
                    );
                }
                let tile_width = width - 42.;
                content = content.child(
                    div().flex().flex_col().items_start().gap(px(10.)).children(
                        saved
                            .into_iter()
                            .map(|d| self.tile(d, tile_width, &snapshot, cx)),
                    ),
                );
                content = content.child(
                    div()
                        .id("bluetooth-find")
                        .py(px(9.))
                        .px(px(12.))
                        .rounded(px(20.))
                        .border_1()
                        .border_color(colors.border.rgb())
                        .text_size(px(11.))
                        .cursor_pointer()
                        .child(if snapshot.scanning {
                            "Stop searching"
                        } else {
                            "＋ Find a device"
                        })
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |view, _, _, cx| {
                                let scanning = cx.global::<BluetoothUi>().snapshot.scanning;
                                view.show_nearby = true;
                                view.controller.scan(!scanning);
                                view.error = None;
                                cx.stop_propagation();
                                cx.notify();
                            }),
                        ),
                );
                if self.show_nearby {
                    content = content.child(
                        div()
                            .text_size(px(11.))
                            .text_color(colors.foreground_muted.rgb())
                            .child(if snapshot.scanning {
                                "Looking nearby…"
                            } else {
                                "Nearby devices"
                            }),
                    );
                    let unnamed_count = snapshot
                        .devices
                        .iter()
                        .filter(|d| !d.paired && !d.connected && unnamed_device(d))
                        .count();
                    let mut nearby = snapshot
                        .devices
                        .iter()
                        .filter(|d| {
                            !d.paired && !d.connected && (self.show_unnamed || !unnamed_device(d))
                        })
                        .collect::<Vec<_>>();
                    nearby.sort_by_key(|d| (unnamed_device(d), d.name.to_lowercase()));
                    if nearby.is_empty() {
                        content = content.child(
                            div()
                                .text_size(px(11.))
                                .child("Put your device in pairing mode."),
                        );
                    }
                    content = content.child(
                        div().flex().flex_col().items_start().gap(px(10.)).children(
                            nearby
                                .into_iter()
                                .map(|d| self.tile(d, tile_width, &snapshot, cx)),
                        ),
                    );
                    if unnamed_count > 0 {
                        content = content.child(
                            div()
                                .id("bluetooth-unnamed")
                                .text_size(px(11.))
                                .text_color(colors.foreground_muted.rgb())
                                .cursor_pointer()
                                .child(if self.show_unnamed {
                                    "Hide unnamed devices".to_owned()
                                } else {
                                    format!("Show unnamed devices ({unnamed_count})")
                                })
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|view, _, _, cx| {
                                        view.show_unnamed = !view.show_unnamed;
                                        cx.stop_propagation();
                                        cx.notify();
                                    }),
                                ),
                        );
                    }
                }
            }
        }
        if let Some(error) = self.error.as_ref().or(snapshot.error.as_ref()) {
            content = content.child(
                div()
                    .text_size(px(11.))
                    .text_color(colors.foreground.rgb())
                    .child(error.clone()),
            );
        }
        div()
            .size_full()
            .font_family(crate::power::FONT_FAMILY)
            .font_features(crate::power::font_features())
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|view, event: &KeyDownEvent, _window, cx| {
                if event.keystroke.key == "escape" {
                    view.dismiss(cx);
                    cx.stop_propagation();
                    return;
                }
                let snapshot = cx.global::<BluetoothUi>().snapshot.clone();
                if let Some(prompt) = &snapshot.prompt {
                    if matches!(prompt.kind, PromptKind::Pin | PromptKind::Passkey) {
                        match event.keystroke.key.as_str() {
                            "backspace" => {
                                view.input.pop();
                            }
                            "enter" => {
                                view.submit(
                                    Action::Answer(prompt.token, Some(view.input.clone())),
                                    cx,
                                );
                            }
                            _ => {
                                if let Some(text) = &event.keystroke.key_char {
                                    for c in text.chars().filter(|c| c.is_ascii_alphanumeric()) {
                                        if view.input.len() < 16 {
                                            view.input.push(c);
                                        }
                                    }
                                }
                            }
                        }
                        cx.stop_propagation();
                        cx.notify();
                    }
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |view, event: &gpui::MouseDownEvent, _, cx| {
                    if !bounds.contains(&event.position) {
                        view.dismiss(cx);
                    }
                }),
            )
            .child(crate::design::shell_extension(
                bounds,
                colors.background.rgb(),
                true,
            ))
            .child(
                div()
                    .id("bluetooth-shell-extension")
                    .absolute()
                    .right(px(right))
                    .top(px(top))
                    .w(px(width))
                    .h(px(height * entry))
                    .overflow_y_scroll()
                    .child(content),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[gpui::test]
    fn dock_can_open_render_and_close_with_shared_state_present(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            cx.set_global(ThemeColors::default());
            let (controller, changes) = Controller::unavailable_for_test();
            install(cx, controller, changes);
            let display = cx.primary_display().unwrap().id();
            toggle(display, cx).unwrap();
            let panel = cx.global::<BluetoothUi>().panel.unwrap();
            panel
                .update(cx, |_, window, cx| {
                    assert_eq!(window.display(cx).unwrap().id(), display)
                })
                .unwrap();
            cx.update_window(panel.into(), |_, window, cx| {
                let _ = window.draw(cx);
            })
            .unwrap();
            assert!(cx.has_global::<BluetoothUi>());
            close(cx);
            assert!(cx.global::<BluetoothUi>().panel.is_none());
        });
    }
}
