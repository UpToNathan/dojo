use std::time::{Duration, Instant};

use anyhow::Context as _;
use gpui::{
    AnyElement, App, Bounds, Context, DisplayId, FillOptions, FillRule, ImageSource, MouseButton,
    PathBuilder, PathStyle, Size, Window, WindowBackgroundAppearance, WindowBounds, WindowKind,
    WindowOptions, div, img, layer_shell::Anchor, point, prelude::*, px,
};

use crate::config::ThemeColors;
use crate::geometry::{BOTTOM_INSET, LEFT_PANEL_WIDTH, RIGHT_INSET, TOP_BAR_HEIGHT};
use crate::power::{self, PowerStatus};
use crate::tray::{TrayAction, TrayHandle, TrayItem, TrayMenu, TrayMenuItem};

const CORNER_RADIUS: f32 = 28.;
const LAUNCHER_BUTTON_SIZE: f32 = 36.;
const TOP_BAR_GAP: f32 = 3.;
const TOP_BAR_BUTTON_WIDTH: f32 = 14.;

struct FrameView {
    power: PowerStatus,
    tray: TrayHandle,
    tray_items: Vec<TrayItem>,
    open_menu: Option<(String, TrayItem)>,
    menu_data: Option<TrayMenu>,
    active_submenu: Option<i32>,
    hovered_menu_item: Option<i32>,
    pending_menu_hover: Option<(i32, Instant)>,
    submenu_bounds: Option<Bounds<gpui::Pixels>>,
    last_menu_pointer: Option<gpui::Point<gpui::Pixels>>,
    last_power_update: Instant,
    launcher_hovered: bool,
    wlsunset_hovered: bool,
    theme_hovered: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PrimaryTrayAction {
    ShowMenu,
    Activate,
    ContextMenu,
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
                    layer: gpui::layer_shell::Layer::Top,
                    anchor: Anchor::TOP | Anchor::RIGHT | Anchor::BOTTOM | Anchor::LEFT,
                    exclusive_zone: Some(px(-1.)),
                    keyboard_interactivity: gpui::layer_shell::KeyboardInteractivity::None,
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| FrameView::new(tray.clone(), window, cx)),
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
                return Err(error).context("opening desktop frame surface");
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
        .map(tray_menu_row_height)
        .sum::<f32>()
}

fn toggle_wlsunset() {
    let result = std::thread::Builder::new()
        .name("dojo-wlsunset-toggle".into())
        .spawn(
            || match std::process::Command::new("toggle-wlsunset").output() {
                Ok(output) if output.status.success() => {}
                Ok(output) => eprintln!(
                    "Dojo could not toggle wlsunset: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
                Err(error) => eprintln!("Dojo could not start the wlsunset toggle: {error}"),
            },
        );
    if let Err(error) = result {
        eprintln!("Dojo could not start the wlsunset toggle thread: {error}");
    }
}

fn tray_menu_row_height(item: &TrayMenuItem) -> f32 {
    if item.separator {
        9.
    } else {
        30. + item.label.lines().count().saturating_sub(1) as f32 * 18.
    }
}

fn tray_menu_width(items: &[TrayMenuItem], window: &Window, colors: ThemeColors) -> f32 {
    let text_system = window.text_system();
    let widest = items
        .iter()
        .filter(|item| item.visible && !item.separator)
        .flat_map(|item| item.label.lines())
        .map(|line| {
            let label = line.replace('_', "");
            let run = gpui::TextRun {
                len: label.len(),
                font: power::font(),
                color: colors.foreground.rgb().into(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            f32::from(
                text_system
                    .shape_line(label.into(), px(12.), &[run], None)
                    .width,
            )
        })
        .fold(0., f32::max);
    (widest + 60.).max(224.)
}

fn primary_tray_action(item: &TrayItem) -> PrimaryTrayAction {
    if !item.item_is_menu {
        PrimaryTrayAction::Activate
    } else if item.menu_path.is_some() {
        PrimaryTrayAction::ShowMenu
    } else {
        PrimaryTrayAction::ContextMenu
    }
}

fn top_bar_input_region(viewport: Size<gpui::Pixels>) -> Bounds<gpui::Pixels> {
    Bounds {
        origin: point(px(0.), px(0.)),
        size: Size::new(viewport.width, px(TOP_BAR_HEIGHT as f32)),
    }
}

fn launcher_button_bounds(viewport: Size<gpui::Pixels>) -> Bounds<gpui::Pixels> {
    Bounds {
        origin: point(
            px((LEFT_PANEL_WIDTH as f32 - LAUNCHER_BUTTON_SIZE) / 2. - 1.),
            viewport.height - px(BOTTOM_INSET as f32 + 6. + LAUNCHER_BUTTON_SIZE),
        ),
        size: Size::new(px(LAUNCHER_BUTTON_SIZE), px(LAUNCHER_BUTTON_SIZE)),
    }
}

fn menu_aim_contains(
    previous: gpui::Point<gpui::Pixels>,
    current: gpui::Point<gpui::Pixels>,
    submenu: Bounds<gpui::Pixels>,
) -> bool {
    let edge = if previous.x >= submenu.right() {
        submenu.right()
    } else if previous.x <= submenu.left() {
        submenu.left()
    } else {
        return submenu.contains(&current);
    };
    let distance = f32::from(edge - previous.x);
    if distance.abs() < f32::EPSILON {
        return submenu.contains(&current);
    }
    let progress = f32::from(current.x - previous.x) / distance;
    if !(0. ..=1.).contains(&progress) || progress == 0. {
        return false;
    }
    let upper = previous.y + (submenu.top() - px(8.) - previous.y) * progress;
    let lower = previous.y + (submenu.bottom() + px(8.) - previous.y) * progress;
    current.y >= upper && current.y <= lower
}

fn submenu_hover_target(
    active: Option<i32>,
    is_submenu: bool,
    item_id: i32,
    has_submenu: bool,
) -> Option<i32> {
    if is_submenu {
        active
    } else {
        has_submenu.then_some(item_id)
    }
}

impl FrameView {
    fn new(tray: TrayHandle, window: &mut Window, cx: &mut Context<Self>) -> Self {
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
            power: power::read_status(),
            tray_items: tray.items(),
            open_menu: None,
            menu_data: None,
            active_submenu: None,
            hovered_menu_item: None,
            pending_menu_hover: None,
            submenu_bounds: None,
            last_menu_pointer: None,
            last_power_update: Instant::now(),
            launcher_hovered: false,
            wlsunset_hovered: false,
            theme_hovered: false,
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
        self.pending_menu_hover = None;
        self.last_menu_pointer = None;

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
        self.pending_menu_hover = None;
        self.submenu_bounds = None;
        self.last_menu_pointer = None;
        cx.notify();
    }

    fn render_menu_items(
        &self,
        items: &[TrayMenuItem],
        menu_owner: &TrayItem,
        menu_width: f32,
        is_submenu: bool,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let colors = *cx.global::<ThemeColors>();
        items
            .iter()
            .filter(|item| item.visible)
            .map(|item| {
                if item.separator {
                    return div()
                        .mx(px(10.))
                        .my(px(4.))
                        .h(px(1.))
                        .bg(colors.border.rgb().opacity(crate::design::BORDER_OPACITY))
                        .into_any_element();
                }

                let item_id = item.id;
                let enabled = item.enabled;
                let has_submenu = item.has_submenu;
                let mut row =
                    div()
                        .id(format!("dbus-menu-item-{item_id}"))
                        .mx(px(8.))
                        .w(px(menu_width - 16.))
                        .h(px(tray_menu_row_height(item)))
                        .px(px(10.))
                        .flex()
                        .items_center()
                        .justify_between()
                        .text_size(px(12.))
                        .rounded(px(15.))
                        .bg(if self.hovered_menu_item == Some(item_id) {
                            colors.background_highlight.rgb()
                        } else {
                            colors.background.rgb()
                        })
                        .text_color(if enabled {
                            colors.foreground.rgb()
                        } else {
                            colors.foreground_muted.rgb()
                        })
                        .child(div().flex().flex_col().children(item.label.split('\n').map(
                            |line| {
                                div().h(px(18.)).child(if line.is_empty() {
                                    " ".to_string()
                                } else {
                                    line.replace('_', "")
                                })
                            },
                        )));

                if item.checked {
                    row = row.child("✓");
                } else if has_submenu {
                    row = row.child("›");
                }

                row = row.on_hover(cx.listener(move |this, hovered, window, cx| {
                    if *hovered {
                        this.hovered_menu_item = Some(item_id);
                    } else if this.hovered_menu_item == Some(item_id) {
                        this.hovered_menu_item = None;
                    }
                    if *hovered {
                        let pointer = window.mouse_position();
                        let target = submenu_hover_target(
                            this.active_submenu,
                            is_submenu,
                            item_id,
                            has_submenu,
                        );
                        if is_submenu {
                            this.pending_menu_hover = None;
                            this.active_submenu = target;
                        } else {
                            let heading_to_submenu = this.active_submenu != target
                                && this.last_menu_pointer.zip(this.submenu_bounds).is_some_and(
                                    |(previous, bounds)| {
                                        menu_aim_contains(previous, pointer, bounds)
                                    },
                                );
                            if heading_to_submenu {
                                let request = (item_id, Instant::now());
                                this.pending_menu_hover = Some(request);
                                cx.spawn_in(window, async move |this, cx| {
                                    cx.background_executor()
                                        .timer(Duration::from_millis(300))
                                        .await;
                                    if let Err(error) = this.update_in(cx, |this, _, cx| {
                                        if this.pending_menu_hover == Some(request) {
                                            this.pending_menu_hover = None;
                                            this.active_submenu = target;
                                            cx.notify();
                                        }
                                    }) {
                                        eprintln!("Dojo could not update submenu hover: {error}");
                                    }
                                })
                                .detach();
                            } else {
                                this.pending_menu_hover = None;
                                this.active_submenu = target;
                            }
                        }
                        this.last_menu_pointer = Some(pointer);
                    }
                    cx.notify();
                }));

                if has_submenu {
                    row = row.on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.active_submenu = Some(item_id);
                            this.pending_menu_hover = None;
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
        let colors = *cx.global::<ThemeColors>();
        let viewport = window.viewport_size();
        let top_bar_input = [top_bar_input_region(viewport)];
        let launcher_bounds = launcher_button_bounds(viewport);
        let launcher_hovered = self.launcher_hovered && window.is_window_hovered();
        let wlsunset_hovered = self.wlsunset_hovered && window.is_window_hovered();
        let theme_hovered = self.theme_hovered && window.is_window_hovered();

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
                .w(px(TOP_BAR_BUTTON_WIDTH))
                .h(px(18.))
                .flex()
                .items_center()
                .justify_center()
                .child(icon)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(
                        move |this, _, window, cx| match primary_tray_action(&left_item) {
                            PrimaryTrayAction::ShowMenu => {
                                this.open_tray_menu(left_item.clone(), window, cx);
                            }
                            PrimaryTrayAction::Activate => {
                                this.tray.activate(&left_item, TrayAction::Activate);
                            }
                            PrimaryTrayAction::ContextMenu => {
                                this.tray.activate(&left_item, TrayAction::ContextMenu);
                            }
                        },
                    ),
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
        let top_bar_width = 124. + tray_count as f32 * (TOP_BAR_BUTTON_WIDTH + TOP_BAR_GAP);
        let tray_group = div()
            .flex()
            .items_center()
            .gap(px(TOP_BAR_GAP))
            .children(tray_icons);

        let mut top_bar = div()
            .absolute()
            .top(px(0.))
            .right(px(RIGHT_INSET as f32))
            .w(px(top_bar_width))
            .h(px(TOP_BAR_HEIGHT as f32))
            .flex()
            .items_center()
            .justify_end()
            .gap(px(TOP_BAR_GAP))
            .text_color(colors.foreground.rgb())
            .child(
                div()
                    .id("wlsunset-button")
                    .aria_label("Toggle wlsunset eye-comfort filter")
                    .w(px(TOP_BAR_BUTTON_WIDTH))
                    .h(px(18.))
                    .flex_shrink_0()
                    .rounded(px(crate::design::PILL_RADIUS))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .text_size(px(18.5))
                    .bg(if wlsunset_hovered {
                        colors.background_highlight.rgb().opacity(0.35)
                    } else {
                        colors.background.rgb()
                    })
                    .text_color(colors.foreground.rgb())
                    .on_hover(cx.listener(|this, hovered, _, cx| {
                        if this.wlsunset_hovered != *hovered {
                            this.wlsunset_hovered = *hovered;
                            cx.notify();
                        }
                    }))
                    .child("\u{f2a8}")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.wlsunset_hovered = false;
                            this.close_tray_menu(cx);
                            cx.notify();
                            cx.stop_propagation();
                            toggle_wlsunset();
                        }),
                    ),
            )
            .child(
                div()
                    .id("theme-picker-button")
                    .aria_label("Choose theme")
                    .w(px(TOP_BAR_BUTTON_WIDTH))
                    .h(px(18.))
                    .flex_shrink_0()
                    .rounded(px(crate::design::PILL_RADIUS))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .text_size(px(18.5))
                    .bg(if theme_hovered {
                        colors.background_highlight.rgb().opacity(0.35)
                    } else {
                        colors.background.rgb()
                    })
                    .text_color(colors.foreground.rgb())
                    .on_hover(cx.listener(|this, hovered, _, cx| {
                        if this.theme_hovered != *hovered {
                            this.theme_hovered = *hovered;
                            cx.notify();
                        }
                    }))
                    .child("\u{f1fc}")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            this.theme_hovered = false;
                            this.close_tray_menu(cx);
                            cx.notify();
                            cx.stop_propagation();
                            window.dispatch_action(
                                Box::new(crate::theme_picker::ToggleThemePicker),
                                cx,
                            );
                        }),
                    ),
            );

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
                .ml(px(2.))
                .gap(px(TOP_BAR_GAP))
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
        let menu_width = active_menu
            .map_or(224., |menu| tray_menu_width(&menu.items, window, colors))
            .min((f32::from(viewport.width) - RIGHT_INSET as f32 * 2.).max(1.));
        let menu_height = active_menu.map_or(46., |menu| tray_menu_height(&menu.items));
        let menu_right = active_item.map(|_| RIGHT_INSET as f32);
        let menu_bounds = menu_right.map(|right| Bounds {
            origin: point(
                viewport.width - px(right + menu_width),
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
            let rows = self.render_menu_items(&menu.items, owner, menu_width, false, cx);
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
                let submenu_width = tray_menu_width(&submenu.children, window, colors)
                    .min((f32::from(viewport.width) - submenu_right).max(1.));
                let submenu_rows =
                    self.render_menu_items(&submenu.children, owner, submenu_width, true, cx);
                let submenu_bounds = Bounds {
                    origin: point(
                        viewport.width - px(submenu_right + submenu_width),
                        bounds.origin.y,
                    ),
                    size: Size::new(px(submenu_width), px(submenu_height.max(46.))),
                };
                menu_hit_bounds.push(submenu_bounds);
                menu_content.push(
                    div()
                        .absolute()
                        .top(bounds.origin.y)
                        .right(px(submenu_right))
                        .w(px(submenu_width))
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
        self.submenu_bounds = menu_hit_bounds.get(1).copied();
        if active_item.is_some() {
            window.set_input_region(Some(&[Bounds {
                origin: point(px(0.), px(0.)),
                size: window.viewport_size(),
            }]));
        } else {
            window.set_input_region(Some(&[top_bar_input[0], launcher_bounds]));
        }

        let mut shell = div()
            .size_full()
            .font_family(power::FONT_FAMILY)
            .font_features(power::font_features())
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
                            Ok(path) => window.paint_path(path, colors.background.rgb()),
                            Err(error) => {
                                eprintln!("Failed to render Dojo's desktop surround: {error}")
                            }
                        }
                    },
                )
                .size_full(),
            )
            .child(top_bar)
            .child(
                div()
                    .absolute()
                    .left(px(-1.))
                    .top(launcher_bounds.origin.y)
                    .w(px(LEFT_PANEL_WIDTH as f32))
                    .h(px(LAUNCHER_BUTTON_SIZE))
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .id("open-launcher")
                            .aria_label("Open application launcher")
                            .size(px(LAUNCHER_BUTTON_SIZE))
                            .rounded(px(LAUNCHER_BUTTON_SIZE / 2.))
                            .text_color(colors.primary.rgb())
                            .text_size(px(32.))
                            .line_height(px(LAUNCHER_BUTTON_SIZE))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .bg(if launcher_hovered {
                                colors.background_highlight.rgb().opacity(0.35)
                            } else {
                                colors.background.rgb()
                            })
                            .on_hover(cx.listener(|this, hovered, _, cx| {
                                if this.launcher_hovered != *hovered {
                                    this.launcher_hovered = *hovered;
                                    cx.notify();
                                }
                            }))
                            .child(
                                gpui::canvas(
                                    |_, _, _| (),
                                    move |bounds, (), window, cx| {
                                        let text: gpui::SharedString = "\u{f303}".into();
                                        let run = gpui::TextRun {
                                            len: text.len(),
                                            font: window.text_style().font(),
                                            color: colors.primary.rgb().into(),
                                            background_color: None,
                                            underline: None,
                                            strikethrough: None,
                                        };
                                        let line = window.text_system().shape_line(
                                            text,
                                            px(32.),
                                            &[run],
                                            None,
                                        );
                                        // Compensate for the circle's leftward offset without
                                        // shifting the icon's screen position.
                                        if let Err(error) = line.paint(
                                            point(bounds.origin.x + px(1.), bounds.origin.y),
                                            bounds.size.height,
                                            gpui::TextAlign::Center,
                                            Some(bounds.size.width),
                                            window,
                                            cx,
                                        ) {
                                            eprintln!(
                                                "Dojo could not paint launcher icon: {error}"
                                            );
                                        }
                                    },
                                )
                                .size_full(),
                            )
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, window, cx| {
                                    this.launcher_hovered = false;
                                    this.close_tray_menu(cx);
                                    window.dispatch_action(
                                        Box::new(crate::launcher::ToggleLauncher),
                                        cx,
                                    );
                                    cx.stop_propagation();
                                }),
                            ),
                    ),
            );
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
    fn launcher_button_has_an_input_target_inside_the_left_reserved_edge() {
        let viewport = Size::new(px(1645.), px(1028.));
        let bounds = launcher_button_bounds(viewport);
        assert!(bounds.left() >= px(0.));
        assert!(bounds.right() <= px(LEFT_PANEL_WIDTH as f32));
        assert_eq!(
            bounds.left() + bounds.size.width / 2.,
            px(LEFT_PANEL_WIDTH as f32 / 2. - 1.)
        );
        assert!(bounds.bottom() < viewport.height);
        assert!(bounds.contains(&point(px(32.), px(988.))));
        assert!(!bounds.contains(&point(px(100.), px(988.))));
    }

    #[test]
    fn hovering_a_submenu_leaf_keeps_its_parent_open() {
        assert_eq!(submenu_hover_target(Some(17), true, 18, false), Some(17));
        assert_eq!(submenu_hover_target(Some(17), false, 19, false), None);
        assert_eq!(submenu_hover_target(Some(17), false, 20, true), Some(20));
    }

    #[test]
    fn menu_aim_protects_diagonal_travel_toward_a_left_submenu() {
        let submenu = Bounds {
            origin: point(px(0.), px(0.)),
            size: Size::new(px(200.), px(300.)),
        };
        let previous = point(px(400.), px(150.));
        assert!(menu_aim_contains(
            previous,
            point(px(300.), px(100.)),
            submenu
        ));
        assert!(menu_aim_contains(
            previous,
            point(px(300.), px(200.)),
            submenu
        ));
        assert!(!menu_aim_contains(
            previous,
            point(px(450.), px(150.)),
            submenu
        ));
        assert!(!menu_aim_contains(
            previous,
            point(px(300.), px(350.)),
            submenu
        ));
    }

    #[test]
    fn top_bar_accepts_clicks_at_the_configured_viewports_right_edge() {
        // A 2880-wide output can initially report 1440 logical pixels at integer
        // scale 2, then configure a 1645-wide viewport at fractional scale 1.75.
        let viewport = Size::new(px(1645.), px(1028.));
        let region = top_bar_input_region(viewport);
        assert!(region.contains(&point(px(1600.), px(12.))));
        assert_eq!(region.size.width, viewport.width);
        assert!(!region.contains(&point(px(1600.), px(100.))));
    }

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
        let mut multiline = entries;
        multiline[0].label = "venus.example.ts.net\naccount@example.com".into();
        assert_eq!(tray_menu_row_height(&multiline[0]), 48.);
        assert_eq!(tray_menu_height(&multiline), 16. + 48. + 9. + 30.);
    }

    #[test]
    fn primary_click_activates_items_that_are_not_menus() {
        let item = TrayItem {
            key: "1Password_status_icon_1".into(),
            service: ":1.702".into(),
            object_path: "/StatusNotifierItem".into(),
            title: String::new(),
            status: "Active".into(),
            icon_name: String::new(),
            icon_pixmaps: Vec::new(),
            item_is_menu: false,
            menu_path: Some("/com/canonical/dbusmenu".into()),
        };

        assert_eq!(primary_tray_action(&item), PrimaryTrayAction::Activate);
    }
}
