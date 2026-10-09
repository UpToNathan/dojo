use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use gpui::{
    AnyElement, App, Bounds, Context, DisplayId, FillOptions, FillRule, FontFeatures, ImageSource,
    MouseButton, PathBuilder, PathStyle, Size, Window, WindowBackgroundAppearance, WindowBounds,
    WindowKind, WindowOptions, div, img, layer_shell::Anchor, point, prelude::*, px, rgb,
};

use crate::geometry::{BOTTOM_INSET, LEFT_PANEL_WIDTH, RIGHT_INSET, TOP_BAR_HEIGHT};
use crate::power::{self, PowerStatus};
use crate::tray::{TrayAction, TrayHandle, TrayItem, TrayMenu, TrayMenuItem};

const CORNER_RADIUS: f32 = 28.;
const SHELL_SURFACE: u32 = 0x1e1e2e;

struct FrameView {
    display_width: gpui::Pixels,
    power: PowerStatus,
    tray: TrayHandle,
    tray_items: Vec<TrayItem>,
    open_menu: Option<(String, TrayItem)>,
    menu_data: Option<TrayMenu>,
    active_submenu: Option<i32>,
    hovered_menu_item: Option<i32>,
    last_power_update: Instant,
}
struct ReservationView {
    exclusive_zone: f32,
}

pub struct DisplayFrame {
    display_id: DisplayId,
    bounds: Bounds<gpui::Pixels>,
    frame: gpui::WindowHandle<FrameView>,
    reservations: Vec<gpui::WindowHandle<ReservationView>>,
}

impl DisplayFrame {
    pub fn open(
        cx: &mut App,
        display_id: DisplayId,
        bounds: Bounds<gpui::Pixels>,
        tray: TrayHandle,
    ) -> anyhow::Result<Self> {
        let reservations = [
            (
                Anchor::TOP | Anchor::LEFT | Anchor::RIGHT,
                Anchor::TOP,
                Size::new(bounds.size.width, px(TOP_BAR_HEIGHT as f32)),
                "top",
                TOP_BAR_HEIGHT as f32,
            ),
            (
                Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
                Anchor::BOTTOM,
                Size::new(bounds.size.width, px(BOTTOM_INSET as f32)),
                "bottom",
                BOTTOM_INSET as f32,
            ),
            (
                Anchor::LEFT | Anchor::TOP | Anchor::BOTTOM,
                Anchor::LEFT,
                Size::new(px(LEFT_PANEL_WIDTH as f32), bounds.size.height),
                "left",
                LEFT_PANEL_WIDTH as f32,
            ),
            (
                Anchor::RIGHT | Anchor::TOP | Anchor::BOTTOM,
                Anchor::RIGHT,
                Size::new(px(RIGHT_INSET as f32), bounds.size.height),
                "right",
                RIGHT_INSET as f32,
            ),
        ];

        let mut opened_reservations = Vec::with_capacity(reservations.len());
        for (anchor, edge, size, name, exclusive_zone) in reservations {
            match cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: bounds.origin,
                        size,
                    })),
                    titlebar: None,
                    app_id: Some("dojo".to_string()),
                    display_id: Some(display_id),
                    window_background: WindowBackgroundAppearance::Transparent,
                    kind: WindowKind::LayerShell(gpui::layer_shell::LayerShellOptions {
                        namespace: format!("dojo-reservation-{name}-{}", u64::from(display_id)),
                        layer: gpui::layer_shell::Layer::Top,
                        anchor,
                        exclusive_zone: Some(px(exclusive_zone)),
                        exclusive_edge: Some(edge),
                        keyboard_interactivity: gpui::layer_shell::KeyboardInteractivity::None,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |_, cx| cx.new(|_| ReservationView { exclusive_zone }),
            ) {
                Ok(window) => opened_reservations.push(window),
                Err(error) => {
                    let errors = close_windows(opened_reservations, cx);
                    if !errors.is_empty() {
                        eprintln!(
                            "Failed to close partial edge reservations: {}",
                            errors.join("; ")
                        );
                    }
                    return Err(error).context(format!("opening {name} edge reservation"));
                }
            }
        }

        let frame = match cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: None,
                app_id: Some("dojo".to_string()),
                display_id: Some(display_id),
                window_background: WindowBackgroundAppearance::Transparent,
                kind: WindowKind::LayerShell(gpui::layer_shell::LayerShellOptions {
                    namespace: format!("dojo-frame-{}", u64::from(display_id)),
                    layer: gpui::layer_shell::Layer::Overlay,
                    anchor: Anchor::TOP | Anchor::RIGHT | Anchor::BOTTOM | Anchor::LEFT,
                    exclusive_zone: Some(px(-1.)),
                    keyboard_interactivity: gpui::layer_shell::KeyboardInteractivity::None,
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| FrameView::new(bounds.size.width, tray.clone(), window, cx)),
        ) {
            Ok(window) => window,
            Err(error) => {
                let errors = close_windows(opened_reservations, cx);
                if !errors.is_empty() {
                    eprintln!(
                        "Failed to close partial edge reservations: {}",
                        errors.join("; ")
                    );
                }
                return Err(error).context("opening full-screen frame overlay");
            }
        };

        Ok(Self {
            display_id,
            bounds,
            frame,
            reservations: opened_reservations,
        })
    }

    pub fn can_reuse(&self, cx: &App, bounds: Bounds<gpui::Pixels>) -> bool {
        self.bounds.size == bounds.size
            && self.frame.read(cx).is_ok()
            && self
                .reservations
                .iter()
                .all(|window| window.read(cx).is_ok())
    }

    pub fn close(self, cx: &mut App) -> anyhow::Result<()> {
        let mut errors = close_windows(self.reservations, cx);
        if let Err(error) = self.frame.update(cx, |_, window, _| window.remove_window()) {
            if !is_already_closed_window(&error) {
                errors.push(error.to_string());
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            anyhow::bail!(
                "failed to close display {} surfaces: {}",
                u64::from(self.display_id),
                errors.join("; ")
            )
        }
    }
}

fn close_windows(
    windows: impl IntoIterator<Item = gpui::WindowHandle<ReservationView>>,
    cx: &mut App,
) -> Vec<String> {
    windows
        .into_iter()
        .filter_map(|window| {
            window
                .update(cx, |_, window, _| window.remove_window())
                .err()
                .filter(|error| !is_already_closed_window(error))
                .map(|error| error.to_string())
        })
        .collect()
}

fn is_already_closed_window(error: &anyhow::Error) -> bool {
    error
        .chain()
        .any(|cause| cause.to_string().contains("window not found"))
}

fn append_rounded_rectangle(
    builder: &mut PathBuilder,
    bounds: Bounds<gpui::Pixels>,
    radius: gpui::Pixels,
) {
    let radius = radius
        .min(bounds.size.width / 2.)
        .min(bounds.size.height / 2.);
    let left = bounds.left();
    let top = bounds.top();
    let right = bounds.right();
    let bottom = bounds.bottom();

    if radius == px(0.) {
        builder.add_polygon(
            &[
                point(left, top),
                point(right, top),
                point(right, bottom),
                point(left, bottom),
            ],
            true,
        );
        return;
    }

    builder.move_to(point(left + radius, top));
    builder.line_to(point(right - radius, top));
    builder.arc_to(
        point(radius, radius),
        px(0.),
        false,
        true,
        point(right, top + radius),
    );
    builder.line_to(point(right, bottom - radius));
    builder.arc_to(
        point(radius, radius),
        px(0.),
        false,
        true,
        point(right - radius, bottom),
    );
    builder.line_to(point(left + radius, bottom));
    builder.arc_to(
        point(radius, radius),
        px(0.),
        false,
        true,
        point(left, bottom - radius),
    );
    builder.line_to(point(left, top + radius));
    builder.arc_to(
        point(radius, radius),
        px(0.),
        false,
        true,
        point(left + radius, top),
    );
    builder.close();
}

fn append_rounded_rectangle_reverse(
    builder: &mut PathBuilder,
    bounds: Bounds<gpui::Pixels>,
    radius: gpui::Pixels,
) {
    let radius = radius
        .min(bounds.size.width / 2.)
        .min(bounds.size.height / 2.);
    let left = bounds.left();
    let top = bounds.top();
    let right = bounds.right();
    let bottom = bounds.bottom();

    builder.move_to(point(left + radius, top));
    builder.arc_to(
        point(radius, radius),
        px(0.),
        false,
        false,
        point(left, top + radius),
    );
    builder.line_to(point(left, bottom - radius));
    builder.arc_to(
        point(radius, radius),
        px(0.),
        false,
        false,
        point(left + radius, bottom),
    );
    builder.line_to(point(right - radius, bottom));
    builder.arc_to(
        point(radius, radius),
        px(0.),
        false,
        false,
        point(right, bottom - radius),
    );
    builder.line_to(point(right, top + radius));
    builder.arc_to(
        point(radius, radius),
        px(0.),
        false,
        false,
        point(right - radius, top),
    );
    builder.line_to(point(left + radius, top));
    builder.close();
}

fn tray_menu_height(items: &[TrayMenuItem]) -> f32 {
    16. + items
        .iter()
        .filter(|item| item.visible)
        .map(|item| if item.separator { 9. } else { 30. })
        .sum::<f32>()
}

impl FrameView {
    fn new(
        display_width: gpui::Pixels,
        tray: TrayHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let tray_for_updates = tray.clone();
        cx.spawn_in(window, async move |this, cx| {
            loop {
                if this
                    .update_in(cx, |this, _window, cx| {
                        if this.last_power_update.elapsed() >= Duration::from_secs(1) {
                            this.power = power::read_status();
                            this.last_power_update = Instant::now();
                        }
                        this.tray_items = tray_for_updates.items();
                        this.menu_data = this
                            .open_menu
                            .as_ref()
                            .and_then(|(key, _)| tray_for_updates.menu(key));
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
                cx.background_executor().timer(Duration::from_secs(1)).await;
            }
        })
        .detach();
        Self {
            display_width,
            power: power::read_status(),
            tray_items: tray.items(),
            open_menu: None,
            menu_data: None,
            active_submenu: None,
            hovered_menu_item: None,
            last_power_update: Instant::now(),
            tray,
        }
    }
}

impl FrameView {
    fn open_tray_menu(&mut self, item: TrayItem, window: &mut Window, cx: &mut Context<Self>) {
        let key = item.key.clone();
        if self
            .open_menu
            .as_ref()
            .is_some_and(|(open_key, _)| open_key == &key)
        {
            self.close_tray_menu(cx);
            return;
        }
        self.tray.open_menu(&item);
        self.open_menu = Some((key.clone(), item));
        self.menu_data = None;
        self.active_submenu = None;
        self.hovered_menu_item = None;

        let tray = self.tray.clone();
        cx.spawn_in(window, async move |this, cx| {
            for _ in 0..50 {
                if let Some(menu) = tray.menu(&key) {
                    let _ = this.update_in(cx, |this, _window, cx| {
                        if this
                            .open_menu
                            .as_ref()
                            .is_some_and(|(open_key, _)| open_key == &key)
                        {
                            this.menu_data = Some(menu);
                            cx.notify();
                        }
                    });
                    return;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(10))
                    .await;
            }
        })
        .detach();
        cx.notify();
    }

    fn close_tray_menu(&mut self, cx: &mut Context<Self>) {
        self.open_menu = None;
        self.menu_data = None;
        self.active_submenu = None;
        self.hovered_menu_item = None;
        cx.notify();
    }

    fn render_menu_items(
        &self,
        items: &[TrayMenuItem],
        menu_owner: &TrayItem,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        items
            .iter()
            .filter(|item| item.visible)
            .map(|item| {
                if item.separator {
                    return div()
                        .mx(px(10.))
                        .my(px(4.))
                        .h(px(1.))
                        .bg(rgb(0x585b70))
                        .into_any_element();
                }

                let item_id = item.id;
                let enabled = item.enabled;
                let has_submenu = item.has_submenu;
                let mut row = div()
                    .id(format!("dbus-menu-item-{item_id}"))
                    .mx(px(8.))
                    .w(px(208.))
                    .h(px(30.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_size(px(12.))
                    .rounded(px(15.))
                    .bg(if self.hovered_menu_item == Some(item_id) {
                        rgb(0x45475a)
                    } else {
                        rgb(SHELL_SURFACE)
                    })
                    .text_color(if enabled {
                        rgb(0xcdd6f4)
                    } else {
                        rgb(0x6c7086)
                    })
                    .child(if item.label.is_empty() {
                        " ".to_string()
                    } else {
                        item.label.replace('_', "")
                    });

                if item.checked {
                    row = row.child("✓");
                } else if has_submenu {
                    row = row.child("›");
                }

                row = row.on_hover(cx.listener(move |this, hovered, _, cx| {
                    if *hovered {
                        this.hovered_menu_item = Some(item_id);
                    } else if this.hovered_menu_item == Some(item_id) {
                        this.hovered_menu_item = None;
                    }
                    if *hovered {
                        this.active_submenu = has_submenu.then_some(item_id);
                    }
                    cx.notify();
                }));

                if has_submenu {
                    row = row.on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.active_submenu = Some(item_id);
                            cx.notify();
                        }),
                    );
                } else if enabled {
                    let menu_owner = menu_owner.clone();
                    row = row.on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.tray.menu_event(&menu_owner, item_id);
                            this.close_tray_menu(cx);
                        }),
                    );
                }

                row.into_any_element()
            })
            .collect()
    }
}

impl Render for FrameView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let top_bar_input = [Bounds {
            origin: point(px(0.), px(0.)),
            size: Size::new(self.display_width, px(TOP_BAR_HEIGHT as f32)),
        }];

        let battery_glyph =
            power::battery_glyph(self.power.capacity, self.power.battery_status.as_deref());
        let charging = self.power.battery_status.as_deref() == Some("Charging");
        let battery_percent = self
            .power
            .capacity
            .map_or("?".to_string(), |capacity| format!("{capacity}%"));
        let visible_items = self
            .tray_items
            .iter()
            .filter(|item| item.status != "Passive")
            .collect::<Vec<_>>();
        let mut tray_icons = Vec::with_capacity(visible_items.len());

        for (index, item) in visible_items.iter().enumerate() {
            let icon: AnyElement = if let Some(image) = item.render_image() {
                img(ImageSource::Render(image))
                    .size(px(11.))
                    .into_any_element()
            } else {
                div()
                    .text_size(px(11.))
                    .child(fallback_tray_glyph(item))
                    .into_any_element()
            };
            let left_item = (*item).clone();
            let mut button = div()
                .id(format!("tray-item-{index}"))
                .aria_label(if item.title.is_empty() {
                    item.icon_name.clone()
                } else {
                    item.title.clone()
                })
                .w(px(18.))
                .h(px(18.))
                .flex()
                .items_center()
                .justify_center()
                .child(icon)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        if left_item.menu_path.is_some() {
                            this.open_tray_menu(left_item.clone(), window, cx);
                        } else if left_item.item_is_menu {
                            this.tray.activate(&left_item, TrayAction::ContextMenu);
                        } else {
                            this.tray.activate(&left_item, TrayAction::Activate);
                        }
                    }),
                );

            let right_item = (*item).clone();
            button = button.on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _, window, cx| {
                    if right_item.menu_path.is_some() {
                        this.open_tray_menu(right_item.clone(), window, cx);
                    } else {
                        this.tray.activate(&right_item, TrayAction::ContextMenu);
                    }
                }),
            );
            let middle_item = (*item).clone();
            button = button.on_mouse_down(
                MouseButton::Middle,
                cx.listener(move |this, _, _, _| {
                    this.tray
                        .activate(&middle_item, TrayAction::SecondaryActivate);
                }),
            );
            tray_icons.push(button.into_any_element());
        }

        let tray_count = tray_icons.len();
        let top_bar_width = if tray_count == 0 {
            72.
        } else {
            77. + tray_count as f32 * 21.
        };
        let tray_group = div().flex().items_center().gap(px(3.)).children(tray_icons);

        let mut top_bar = div()
            .absolute()
            .top(px(0.))
            .right(px(RIGHT_INSET as f32))
            .w(px(top_bar_width))
            .h(px(TOP_BAR_HEIGHT as f32))
            .flex()
            .items_center()
            .justify_end()
            .gap(px(8.))
            .text_color(rgb(0xcdd6f4));

        if tray_count > 0 {
            top_bar = top_bar.child(tray_group);
        }
        top_bar = top_bar.child(
            div()
                .id("battery-status")
                .aria_label(
                    if self.power.battery_status.as_deref() == Some("Charging") {
                        "Battery charging"
                    } else {
                        "Battery status"
                    },
                )
                .flex()
                .items_center()
                .gap(px(2.))
                .text_size(px(11.))
                .child(
                    div()
                        // MDI charging variants occupy only about 52% of the em vertically,
                        // versus about 91% for the regular battery glyphs.
                        .text_size(px(if charging { 18.5 } else { 11. }))
                        .child(battery_glyph.to_string()),
                )
                .child(div().child(battery_percent)),
        );

        let active_item = self.open_menu.as_ref().map(|(_, item)| item);
        let active_menu = self.menu_data.as_ref();
        let menu_width = 224.;
        let menu_height = active_menu.map_or(46., |menu| tray_menu_height(&menu.items));
        let menu_right = active_item.map(|_| RIGHT_INSET as f32);
        let menu_bounds = menu_right.map(|right| Bounds {
            origin: point(
                self.display_width - px(right + menu_width),
                px(TOP_BAR_HEIGHT as f32),
            ),
            size: Size::new(px(menu_width), px(menu_height.max(46.))),
        });

        let mut menu_hit_bounds = menu_bounds.into_iter().collect::<Vec<_>>();

        let mut menu_surface = active_menu.and_then(|_| menu_bounds);
        let mut menu_content: Vec<AnyElement> = Vec::new();
        if let (Some((_, owner)), Some(menu), Some(bounds), Some(right)) = (
            self.open_menu.as_ref(),
            active_menu,
            menu_bounds,
            menu_right,
        ) {
            let rows = self.render_menu_items(&menu.items, owner, cx);
            menu_content.push(
                div()
                    .absolute()
                    .top(bounds.origin.y)
                    .right(px(right))
                    .w(px(menu_width))
                    .flex()
                    .flex_col()
                    .py(px(8.))
                    .children(rows)
                    .into_any_element(),
            );

            if let Some(submenu) = self
                .active_submenu
                .and_then(|id| menu.items.iter().find(|item| item.id == id))
            {
                let submenu_height = tray_menu_height(&submenu.children);
                let submenu_right = right + menu_width - 2.;
                let submenu_rows = self.render_menu_items(&submenu.children, owner, cx);
                let submenu_bounds = Bounds {
                    origin: point(
                        self.display_width - px(submenu_right + menu_width),
                        bounds.origin.y,
                    ),
                    size: Size::new(px(menu_width), px(submenu_height.max(46.))),
                };
                menu_hit_bounds.push(submenu_bounds);
                menu_content.push(
                    div()
                        .absolute()
                        .top(bounds.origin.y)
                        .right(px(submenu_right))
                        .w(px(menu_width))
                        .flex()
                        .flex_col()
                        .py(px(8.))
                        .children(submenu_rows)
                        .into_any_element(),
                );
                menu_surface = Some(Bounds {
                    origin: submenu_bounds.origin,
                    size: Size::new(
                        bounds.right() - submenu_bounds.left(),
                        bounds.size.height.max(submenu_bounds.size.height),
                    ),
                });
            }
        }
        if active_item.is_some() {
            window.set_input_region(Some(&[Bounds {
                origin: point(px(0.), px(0.)),
                size: window.viewport_size(),
            }]));
        } else {
            window.set_input_region(Some(&top_bar_input));
        }

        let mut shell = div()
            .size_full()
            .font_family(power::FONT_FAMILY)
            .font_features(FontFeatures(Arc::new(vec![("ss01".to_string(), 1)])))
            .child(
                gpui::canvas(
                    |_, _, _| (),
                    move |bounds, (), window, _| {
                        let inner_bounds = Bounds::from_corners(
                            point(
                                bounds.left() + px(LEFT_PANEL_WIDTH as f32),
                                bounds.top() + px(TOP_BAR_HEIGHT as f32),
                            ),
                            point(
                                bounds.right() - px(RIGHT_INSET as f32),
                                bounds.bottom() - px(BOTTOM_INSET as f32),
                            ),
                        );
                        let mut builder = PathBuilder::fill().with_style(PathStyle::Fill(
                            FillOptions::default().with_fill_rule(FillRule::NonZero),
                        ));
                        append_rounded_rectangle(&mut builder, bounds, px(0.));
                        append_rounded_rectangle_reverse(
                            &mut builder,
                            inner_bounds,
                            px(CORNER_RADIUS),
                        );
                        if let Some(menu_bounds) = menu_surface {
                            append_rounded_rectangle(&mut builder, menu_bounds, px(24.));
                        }

                        match builder.build() {
                            Ok(path) => window.paint_path(path, rgb(SHELL_SURFACE)),
                            Err(error) => {
                                eprintln!("Failed to render Dojo's desktop surround: {error}")
                            }
                        }
                    },
                )
                .size_full(),
            )
            .child(top_bar);
        if let Some((menu_key, _)) = &self.open_menu {
            let menu_key = menu_key.clone();
            let hit_bounds = menu_hit_bounds.clone();
            for button in [MouseButton::Left, MouseButton::Right, MouseButton::Middle] {
                let menu_key = menu_key.clone();
                let hit_bounds = hit_bounds.clone();
                shell = shell.on_mouse_down(
                    button,
                    cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                        let inside_menu = hit_bounds
                            .iter()
                            .any(|bounds| bounds.contains(&event.position));
                        if !inside_menu
                            && this
                                .open_menu
                                .as_ref()
                                .is_some_and(|(open_key, _)| open_key == &menu_key)
                        {
                            this.close_tray_menu(cx);
                        }
                    }),
                );
            }
        }
        for child in menu_content {
            shell = shell.child(child);
        }
        shell
    }
}

fn fallback_tray_glyph(item: &TrayItem) -> String {
    item.icon_name
        .chars()
        .find(|character| character.is_alphanumeric())
        .map(|character| character.to_uppercase().to_string())
        .or_else(|| {
            item.title
                .chars()
                .find(|character| character.is_alphanumeric())
                .map(|character| character.to_uppercase().to_string())
        })
        .unwrap_or_else(|| "•".to_string())
}

impl Render for ReservationView {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        window.set_input_region(Some(&[]));
        window.set_exclusive_zone(px(self.exclusive_zone));
        div().size_full()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_height_accounts_for_compact_separators() {
        let entries = vec![
            TrayMenuItem {
                id: 1,
                label: "Open".into(),
                enabled: true,
                visible: true,
                separator: false,
                checked: false,
                has_submenu: false,
                children: Vec::new(),
            },
            TrayMenuItem {
                id: 2,
                label: String::new(),
                enabled: true,
                visible: true,
                separator: true,
                checked: false,
                has_submenu: false,
                children: Vec::new(),
            },
            TrayMenuItem {
                id: 3,
                label: "Quit".into(),
                enabled: true,
                visible: true,
                separator: false,
                checked: false,
                has_submenu: false,
                children: Vec::new(),
            },
        ];

        assert_eq!(tray_menu_height(&entries), 16. + 30. + 9. + 30.);
    }
}
