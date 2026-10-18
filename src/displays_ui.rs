use std::sync::Arc;
use std::time::{Duration, Instant};

use dojo::displays::{
    Controller, OutputSettings, Snapshot, logical_size, rotated, rotation_label, snapped_position,
};
use gpui::{
    App, AppContext, Bounds, Context, DisplayId, Entity, FocusHandle, IntoElement, KeyDownEvent,
    MouseButton, Render, Window, WindowBackgroundAppearance, WindowBounds, WindowHandle,
    WindowKind, WindowOptions, div, layer_shell, point, prelude::*, px,
};

use crate::config::ThemeColors;

pub struct DisplaysUi {
    snapshot: Arc<Snapshot>,
    controller: Controller,
    panel: Option<WindowHandle<Editor>>,
    _updates: gpui::Task<()>,
}
impl gpui::Global for DisplaysUi {}

pub fn initialize(cx: &mut App) -> anyhow::Result<()> {
    let (controller, changes) = Controller::start()?;
    let updates = controller.clone();
    let task = cx.spawn(async move |cx| {
        while changes.recv().await.is_ok() {
            let snapshot = updates.snapshot();
            let _ =
                cx.update(|cx| cx.update_global::<DisplaysUi, _>(|ui, _| ui.snapshot = snapshot));
        }
    });
    cx.set_global(DisplaysUi {
        snapshot: controller.snapshot(),
        controller,
        panel: None,
        _updates: task,
    });
    Ok(())
}

pub fn close(cx: &mut App) {
    if !cx.has_global::<DisplaysUi>() {
        return;
    }
    let (panel, controller) =
        cx.update_global::<DisplaysUi, _>(|ui, _| (ui.panel.take(), ui.controller.clone()));
    if let Err(error) = controller.close() {
        eprintln!("Could not close display editor: {error}");
    }
    if let Some(panel) = panel {
        let _ = panel.update(cx, |_, window, _| window.remove_window());
    }
}

pub fn toggle(display_id: DisplayId, cx: &mut App) -> anyhow::Result<()> {
    if !cx.has_global::<DisplaysUi>() {
        initialize(cx)?;
    }
    let previous = cx.update_global::<DisplaysUi, _>(|ui, _| ui.panel.take());
    if let Some(panel) = previous {
        if panel
            .update(cx, |view, window, _| {
                let _ = view.controller.close();
                window.remove_window();
            })
            .is_ok()
        {
            return Ok(());
        }
    }
    let controller = cx.global::<DisplaysUi>().controller.clone();
    controller.refresh().map_err(anyhow::Error::msg)?;
    let display = cx
        .displays()
        .into_iter()
        .find(|d| d.id() == display_id)
        .ok_or_else(|| anyhow::anyhow!("Monitor disappeared"))?;
    let selected_output = cx.new(|_| SelectedOutput(None));
    let editor_selection = selected_output.clone();
    let panel = cx.open_window(
        WindowOptions {
            display_id: Some(display_id),
            window_bounds: Some(WindowBounds::Windowed(display.bounds())),
            titlebar: None,
            app_id: Some("dojo".into()),
            window_background: WindowBackgroundAppearance::Transparent,
            kind: WindowKind::LayerShell(layer_shell::LayerShellOptions {
                namespace: "dojo-displays-dock".into(),
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
                let snapshot = cx.global::<DisplaysUi>().snapshot.clone();
                let observer = cx.observe_global::<DisplaysUi>(|view: &mut Editor, cx| {
                    let snapshot = cx.global::<DisplaysUi>().snapshot.clone();
                    let names: Vec<_> = snapshot
                        .displays
                        .iter()
                        .map(|d| d.settings.name.as_str())
                        .collect();
                    if !view.dirty
                        || (view.previewing
                            && snapshot.preview_remaining.is_none()
                            && !snapshot.busy)
                    {
                        view.draft = snapshot
                            .displays
                            .iter()
                            .map(|d| d.settings.clone())
                            .collect();
                        view.dirty = false;
                    } else if view.draft.iter().any(|d| !names.contains(&d.name.as_str()))
                        || view.draft.len() != names.len()
                    {
                        view.draft = snapshot
                            .displays
                            .iter()
                            .map(|d| d.settings.clone())
                            .collect();
                        view.dirty = false;
                        view.drag = None;
                        view.error = Some(
                            "The connected monitors changed. Review the arrangement again.".into(),
                        );
                    }
                    if let Some(error) = &snapshot.error {
                        view.error = Some(error.clone());
                    }
                    view.selected = view.selected.min(view.draft.len().saturating_sub(1));
                    view.previewing = snapshot.preview_remaining.is_some();
                    cx.notify();
                });
                let poll_controller = controller.clone();
                let polling = cx.spawn(async move |this, cx| {
                    loop {
                        cx.background_executor().timer(Duration::from_secs(1)).await;
                        if this
                            .update(cx, |view, _| view.reveal.is_open())
                            .unwrap_or(false)
                        {
                            let _ = poll_controller.refresh();
                        } else {
                            break;
                        }
                    }
                });
                let release = controller.clone();
                cx.on_release(move |editor: &mut Editor, cx| {
                    let _ = release.close();
                    for badge in editor.badges.drain(..) {
                        let _ = badge.update(cx, |_, window, _| window.remove_window());
                    }
                })
                .detach();
                Editor {
                    controller,
                    draft: snapshot
                        .displays
                        .iter()
                        .map(|d| d.settings.clone())
                        .collect(),
                    selected: 0,
                    dirty: false,
                    previewing: snapshot.preview_remaining.is_some(),
                    error: snapshot.error.clone(),
                    menu: None,
                    drag: None,
                    badges: Vec::new(),
                    selected_output: editor_selection,
                    focus,
                    reveal: dojo::design::MenuReveal::opening_for(
                        Instant::now(),
                        Duration::from_millis(150),
                    ),
                    _observer: observer,
                    _polling: polling,
                }
            })
        },
    )?;
    let badges = open_badges(&selected_output, cx);
    let _ = panel.update(cx, |editor, _, _| editor.badges = badges);
    cx.update_global::<DisplaysUi, _>(|ui, _| ui.panel = Some(panel));
    Ok(())
}

/// Which monitor is selected in the editor, so the badge on each screen can show it.
struct SelectedOutput(Option<String>);

/// A badge on each connected monitor naming it, numbered like its box in the editor.
struct Badge {
    number: usize,
    output: String,
    label: String,
    detail: String,
    selected: Entity<SelectedOutput>,
}
impl Render for Badge {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = *cx.global::<ThemeColors>();
        let selected = self.selected.read(cx).0.as_deref() == Some(self.output.as_str());
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .font_family(crate::power::FONT_FAMILY)
            .font_features(crate::power::font_features())
            .text_color(colors.foreground.rgb())
            .child(
                div()
                    .px(px(48.))
                    .py(px(32.))
                    .rounded(px(28.))
                    .border_4()
                    .border_color(if selected {
                        colors.primary.rgb()
                    } else {
                        colors.border.rgb()
                    })
                    .bg(colors.background.rgb())
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(6.))
                    .child(div().text_size(px(120.)).child(self.number.to_string()))
                    .child(div().text_size(px(22.)).child(self.label.clone()))
                    .child(
                        div()
                            .text_size(px(14.))
                            .text_color(colors.foreground_muted.rgb())
                            .child(self.detail.clone()),
                    ),
            )
    }
}

/// Opens a click-through badge on every connected monitor.
fn open_badges(selected: &Entity<SelectedOutput>, cx: &mut App) -> Vec<WindowHandle<Badge>> {
    let snapshot = cx.global::<DisplaysUi>().snapshot.clone();
    let mut badges = Vec::new();
    for display in cx.displays() {
        let Ok(uuid) = display.uuid() else { continue };
        let Some((index, shown)) = snapshot.displays.iter().enumerate().find(|(_, shown)| {
            shown.settings.enabled
                && crate::edge_reservation::display_uuid(&shown.settings.name) == uuid
        }) else {
            continue;
        };
        let model = shown.output.model.as_str();
        let badge = Badge {
            number: index + 1,
            output: shown.settings.name.clone(),
            label: if model.is_empty() || model == "Unknown" {
                shown.settings.name.clone()
            } else {
                model.to_owned()
            },
            detail: match shown.settings.mode {
                Some(mode) => format!("{} · {}×{}", shown.settings.name, mode.width, mode.height),
                None => shown.settings.name.clone(),
            },
            selected: selected.clone(),
        };
        let opened = cx.open_window(
            WindowOptions {
                display_id: Some(display.id()),
                window_bounds: Some(WindowBounds::Windowed(display.bounds())),
                titlebar: None,
                app_id: Some("dojo".into()),
                window_background: WindowBackgroundAppearance::Transparent,
                kind: WindowKind::LayerShell(layer_shell::LayerShellOptions {
                    namespace: format!("dojo-display-badge-{}", shown.settings.name),
                    layer: layer_shell::Layer::Overlay,
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
            |window, cx| {
                window.set_input_region(Some(&[]));
                cx.new(|cx| {
                    cx.observe(&badge.selected, |_, _, cx| cx.notify()).detach();
                    badge
                })
            },
        );
        match opened {
            Ok(handle) => badges.push(handle),
            Err(error) => eprintln!("Could not open a monitor badge: {error:#}"),
        }
    }
    badges
}

#[derive(Clone, Copy)]
struct Canvas {
    scale: f64,
    offset: (f64, f64),
}
impl Canvas {
    fn fit(outputs: &[OutputSettings], width: f64, height: f64) -> Self {
        let mut bounds: Option<(f64, f64, f64, f64)> = None;
        for output in outputs.iter().filter(|o| o.enabled) {
            let (w, h) = logical_size(output);
            let (x, y) = (output.position.0 as f64, output.position.1 as f64);
            bounds = Some(match bounds {
                Some((l, t, r, b)) => (l.min(x), t.min(y), r.max(x + w), b.max(y + h)),
                None => (x, y, x + w, y + h),
            });
        }
        let (l, t, r, b) = bounds.unwrap_or((0., 0., 1920., 1080.));
        let scale =
            ((width - 48.).max(1.) / (r - l).max(1.)).min((height - 48.).max(1.) / (b - t).max(1.));
        Self {
            scale,
            offset: (
                (width - (r - l) * scale) / 2. - l * scale,
                (height - (b - t) * scale) / 2. - t * scale,
            ),
        }
    }
    fn position(self, position: (i32, i32)) -> (f32, f32) {
        (
            (position.0 as f64 * self.scale + self.offset.0) as f32,
            (position.1 as f64 * self.scale + self.offset.1) as f32,
        )
    }
    fn moved(self, origin: (i32, i32), delta: (f32, f32)) -> (i32, i32) {
        (
            origin
                .0
                .saturating_add((delta.0 as f64 / self.scale).round() as i32),
            origin
                .1
                .saturating_add((delta.1 as f64 / self.scale).round() as i32),
        )
    }
}
#[derive(Clone, Copy)]
struct Drag {
    start: (f32, f32),
    origin: (i32, i32),
    canvas: Canvas,
}
#[derive(Clone, Copy, PartialEq)]
enum Menu {
    Resolution,
    Refresh,
}

fn mode_choices(
    modes: &[niri_ipc::Mode],
    current: Option<niri_ipc::Mode>,
    menu: Menu,
) -> Vec<niri_ipc::Mode> {
    let mut choices = modes.to_vec();
    choices.sort_by_key(|m| (m.width, m.height, m.refresh_rate));
    if menu == Menu::Refresh {
        if let Some(current) = current {
            choices.retain(|m| m.width == current.width && m.height == current.height);
        }
    } else {
        choices.sort_by_key(|m| {
            (
                m.width,
                m.height,
                std::cmp::Reverse(m.is_preferred),
                std::cmp::Reverse(m.refresh_rate),
            )
        });
        choices.dedup_by_key(|m| (m.width, m.height));
    }
    choices
}

#[derive(Clone, Copy)]
enum Edit {
    Resolution,
    Refresh,
    Mode(niri_ipc::Mode),
    Scale(f64),
    Rotate(bool),
    Enabled,
    Apply,
    Keep,
    Revert,
    Reset,
}
struct Editor {
    controller: Controller,
    draft: Vec<OutputSettings>,
    selected: usize,
    dirty: bool,
    previewing: bool,
    error: Option<String>,
    drag: Option<Drag>,
    menu: Option<Menu>,
    badges: Vec<WindowHandle<Badge>>,
    selected_output: Entity<SelectedOutput>,
    focus: FocusHandle,
    reveal: dojo::design::MenuReveal,
    _observer: gpui::Subscription,
    _polling: gpui::Task<()>,
}
impl Editor {
    fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.error = self.controller.close().err();
        if self.error.is_none() {
            self.reveal.set_open(false, Instant::now());
        }
        cx.notify();
    }
    fn edit(&mut self, action: Edit, cx: &mut Context<Self>) {
        let snapshot = cx.global::<DisplaysUi>().snapshot.clone();
        self.error = None;
        if !matches!(action, Edit::Resolution | Edit::Refresh) {
            self.menu = None;
        }
        match action {
            Edit::Apply => {
                self.error = dojo::displays::validate(&self.draft).err();
                if self.error.is_none() {
                    self.error = self.controller.preview(self.draft.clone()).err();
                }
            }
            Edit::Keep => self.error = self.controller.keep().err(),
            Edit::Revert => self.error = self.controller.revert().err(),
            Edit::Reset => {
                self.draft = snapshot
                    .displays
                    .iter()
                    .map(|d| d.settings.clone())
                    .collect();
                self.dirty = false;
            }
            _ if snapshot.busy || snapshot.preview_remaining.is_some() => (),
            _ => {
                let enabled_count = self.draft.iter().filter(|d| d.enabled).count();
                if let Some(output) = self.draft.get_mut(self.selected) {
                    match action {
                        Edit::Scale(delta) => output.scale = (output.scale + delta).clamp(0.5, 4.),
                        Edit::Rotate(clockwise) => {
                            output.transform = rotated(output.transform, clockwise);
                        }
                        Edit::Enabled => {
                            if output.enabled && enabled_count == 1 {
                                self.error = Some("Keep at least one monitor enabled.".into());
                            } else {
                                output.enabled = !output.enabled;
                                if output.enabled && output.mode.is_none() {
                                    output.mode = snapshot
                                        .displays
                                        .iter()
                                        .find(|display| display.settings.name == output.name)
                                        .and_then(|display| {
                                            display
                                                .output
                                                .modes
                                                .iter()
                                                .find(|mode| mode.is_preferred)
                                                .or_else(|| display.output.modes.first())
                                        })
                                        .copied();
                                }
                            }
                        }
                        Edit::Resolution | Edit::Refresh => {
                            let menu = if matches!(action, Edit::Resolution) {
                                Menu::Resolution
                            } else {
                                Menu::Refresh
                            };
                            self.menu = if self.menu == Some(menu) {
                                None
                            } else {
                                Some(menu)
                            };
                        }
                        Edit::Mode(mode) => {
                            output.mode = Some(mode);
                            self.menu = None;
                        }
                        _ => (),
                    }
                    if !matches!(action, Edit::Resolution | Edit::Refresh) {
                        self.dirty = true;
                    }
                }
            }
        }
        cx.notify();
    }
    fn button(
        &self,
        id: &str,
        label: String,
        action: Edit,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let colors = *cx.global::<ThemeColors>();
        div()
            .id(id.to_owned())
            .px(px(12.))
            .py(px(8.))
            .rounded(px(dojo::design::PILL_RADIUS))
            .bg(colors.surface_raised.rgb())
            .text_size(px(12.))
            .text_color(if enabled {
                colors.foreground.rgb()
            } else {
                colors.foreground_muted.rgb()
            })
            .when(enabled, |el| {
                el.cursor_pointer().on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |view, _, _, cx| {
                        view.edit(action, cx);
                        cx.stop_propagation();
                    }),
                )
            })
            .child(label)
            .into_any_element()
    }
}
impl Render for Editor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = *cx.global::<ThemeColors>();
        let snapshot = cx.global::<DisplaysUi>().snapshot.clone();
        let viewport = window.viewport_size();
        let width = (f32::from(viewport.width) - 32.).clamp(260., 880.);
        let right = crate::geometry::RIGHT_INSET as f32;
        let top = crate::geometry::TOP_BAR_HEIGHT as f32;
        let map_height = (f32::from(viewport.height) * 0.38).clamp(120., 300.);
        let canvas = self
            .drag
            .map(|d| d.canvas)
            .unwrap_or_else(|| Canvas::fit(&self.draft, (width - 40.) as f64, map_height as f64));
        let (entry, animating) = self
            .reveal
            .sample_with_motion(Instant::now(), cx.reduce_motion());
        if animating {
            window.request_animation_frame();
        }
        if !self.reveal.is_open() && entry < 0.001 {
            window.remove_window();
        }
        let editing = !snapshot.busy && snapshot.preview_remaining.is_none();
        let selected_name = self.draft.get(self.selected).map(|o| o.name.clone());
        if self.selected_output.read(cx).0 != selected_name {
            let shared = self.selected_output.clone();
            cx.defer(move |cx| {
                shared.update(cx, |selected, cx| {
                    selected.0 = selected_name;
                    cx.notify();
                })
            });
        }
        let mut map = div()
            .id("display-arrangement")
            .relative()
            .w_full()
            .h(px(map_height))
            .rounded(px(14.))
            .bg(colors.surface_raised.rgb())
            .overflow_hidden();
        for (index, output) in self.draft.iter().enumerate().filter(|(_, o)| o.enabled) {
            let (x, y) = canvas.position(output.position);
            let (w, h) = logical_size(output);
            let name = output.name.clone();
            let label = snapshot
                .displays
                .iter()
                .find(|d| d.output.name == name)
                .map(|d| d.output.model.clone())
                .filter(|s| s != "Unknown" && !s.is_empty())
                .unwrap_or_else(|| name.clone());
            map = map.child(
                div()
                    .id(format!("display-rectangle-{name}"))
                    .absolute()
                    .left(px(x))
                    .top(px(y))
                    .w(px((w * canvas.scale) as f32))
                    .h(px((h * canvas.scale) as f32))
                    .rounded(px(8.))
                    .border_2()
                    .border_color(if index == self.selected {
                        colors.primary.rgb()
                    } else {
                        colors.foreground_muted.rgb()
                    })
                    .bg(colors.background.rgb())
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .overflow_hidden()
                    .gap(px(4.))
                    .text_size(px(12.))
                    .cursor_pointer()
                    .child(div().text_size(px(20.)).child((index + 1).to_string()))
                    .child(label)
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(colors.foreground_muted.rgb())
                            .child(name),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |view, event: &gpui::MouseDownEvent, _, cx| {
                            if view.menu.take().is_some() {
                                cx.stop_propagation();
                                cx.notify();
                                return;
                            }
                            view.selected = index;
                            view.error = None;
                            if editing {
                                view.drag = Some(Drag {
                                    start: (
                                        f32::from(event.position.x),
                                        f32::from(event.position.y),
                                    ),
                                    origin: view.draft[index].position,
                                    canvas,
                                });
                            }
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    ),
            );
        }
        let mut selectors = div().flex().flex_wrap().gap(px(8.));
        for (index, output) in self.draft.iter().enumerate() {
            selectors = selectors.child(
                div()
                    .id(format!("display-select-{}", output.name))
                    .px(px(10.))
                    .py(px(5.))
                    .rounded(px(dojo::design::PILL_RADIUS))
                    .bg(if index == self.selected {
                        colors.background_highlight.rgb()
                    } else {
                        colors.background.rgb()
                    })
                    .cursor_pointer()
                    .text_size(px(11.))
                    .text_color(if index == self.selected {
                        colors.selection_foreground.rgb()
                    } else {
                        colors.foreground.rgb()
                    })
                    .child(format!(
                        "{} {}{}",
                        index + 1,
                        output.name,
                        if output.enabled { "" } else { " · off" }
                    ))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |view, _, _, cx| {
                            view.selected = index;
                            view.menu = None;
                            view.error = None;
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    ),
            );
        }
        let mut content = div()
            .id("display-editor-content")
            .p(px(20.))
            .flex()
            .flex_col()
            .gap(px(14.))
            .text_color(colors.foreground.rgb())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, _, _, cx| {
                    view.menu = None;
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(div().text_size(px(16.)).child("Monitors")),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(colors.foreground_muted.rgb())
                    .child(
                        "Drag monitors to match your desk. Numbers match the badges on each screen. Changes apply after you press Apply.",
                    ),
            )
            .child(map)
            .child(selectors);
        if let Some(output) = self.draft.get(self.selected) {
            let resolution = output
                .mode
                .map(|m| format!("{} × {}", m.width, m.height))
                .unwrap_or_else(|| "Automatic resolution".into())
                + " ▾";
            let refresh = output
                .mode
                .map(|m| format!("{:.3} Hz", m.refresh_rate as f64 / 1000.))
                .unwrap_or_else(|| "Automatic refresh".into())
                + " ▾";
            content=content.child(div().flex().flex_wrap().gap(px(8.))
                .child(self.button("display-resolution",resolution,Edit::Resolution,editing,cx))
                .child(self.button("display-refresh",refresh,Edit::Refresh,editing,cx))
                .child(self.button("display-scale-down","−".into(),Edit::Scale(-0.25),editing && output.scale>0.5,cx))
                .child(div().py(px(8.)).text_size(px(12.)).child(format!("{:.0}%",output.scale*100.)))
                .child(self.button("display-scale-up","+".into(),Edit::Scale(0.25),editing && output.scale<4.,cx))
                .child(self.button("display-rotate-ccw","↺".into(),Edit::Rotate(false),editing && output.enabled,cx))
                .child(div().py(px(8.)).text_size(px(12.)).child(rotation_label(output.transform)))
                .child(self.button("display-rotate-cw","↻".into(),Edit::Rotate(true),editing && output.enabled,cx))
                .child(self.button("display-enabled",if output.enabled { "Enabled" } else { "Disabled" }.into(),Edit::Enabled,editing,cx)))
                .child(div().text_size(px(10.)).text_color(colors.foreground_muted.rgb()).child(format!("Position {}, {} · Arrow keys move the selected monitor. Shift moves one pixel.",output.position.0,output.position.1)));
        } else {
            content = content.child(div().child("No connected monitors."));
        }
        if let (Some(menu), Some(output)) = (self.menu, self.draft.get(self.selected)) {
            if let Some(display) = snapshot
                .displays
                .iter()
                .find(|d| d.settings.name == output.name)
            {
                let choices = mode_choices(&display.output.modes, output.mode, menu);
                let mut picker = div()
                    .id("display-mode-picker")
                    .max_h(px(180.))
                    .overflow_y_scroll()
                    .p(px(8.))
                    .rounded(px(12.))
                    .bg(colors.surface_raised.rgb())
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
                for mode in choices {
                    let selected = output.mode.is_some_and(|current| {
                        if menu == Menu::Resolution {
                            current.width == mode.width && current.height == mode.height
                        } else {
                            current.width == mode.width
                                && current.height == mode.height
                                && current.refresh_rate == mode.refresh_rate
                        }
                    });
                    let label = if menu == Menu::Resolution {
                        format!("{} × {}", mode.width, mode.height)
                    } else {
                        format!("{:.3} Hz", mode.refresh_rate as f64 / 1000.)
                    };
                    picker = picker.child(self.button(
                        &format!(
                            "display-mode-{}-{}-{}",
                            mode.width, mode.height, mode.refresh_rate
                        ),
                        format!("{}{}", if selected { "✓ " } else { "" }, label),
                        Edit::Mode(mode),
                        editing,
                        cx,
                    ));
                }
                content = content.child(picker);
            }
        }
        if let Some(remaining) = snapshot.preview_remaining {
            content = content.child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        div()
                            .text_size(px(12.))
                            .child(format!("Keep this arrangement? Reverting in {remaining}s.")),
                    )
                    .child(self.button(
                        "display-keep",
                        "Keep & save".into(),
                        Edit::Keep,
                        !snapshot.busy,
                        cx,
                    ))
                    .child(self.button(
                        "display-revert",
                        "Revert".into(),
                        Edit::Revert,
                        !snapshot.busy,
                        cx,
                    )),
            );
        } else {
            content = content.child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        self.button(
                            "display-apply",
                            if snapshot.busy {
                                "Applying…"
                            } else {
                                "Apply"
                            }
                            .into(),
                            Edit::Apply,
                            editing && self.dirty,
                            cx,
                        ),
                    )
                    .child(self.button(
                        "display-reset",
                        "Reset".into(),
                        Edit::Reset,
                        editing && self.dirty,
                        cx,
                    )),
            );
        }
        if let Some(error) = self.error.as_ref().or(snapshot.error.as_ref()) {
            content = content.child(
                div()
                    .id("display-error")
                    .text_size(px(12.))
                    .child(error.clone()),
            );
        }
        let height = (map_height + 280.).min((f32::from(viewport.height) - top - 20.).max(100.));
        let bounds = Bounds::new(
            point(viewport.width - px(width + right), px(top)),
            gpui::size(px(width), px(height * entry)),
        );
        div()
            .size_full()
            .font_family(crate::power::FONT_FAMILY)
            .font_features(crate::power::font_features())
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|view, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    if view.menu.take().is_some() {
                        cx.notify();
                    } else {
                        view.dismiss(cx);
                    }
                    cx.stop_propagation();
                    return;
                }
                let snapshot = cx.global::<DisplaysUi>().snapshot.clone();
                if snapshot.busy || snapshot.preview_remaining.is_some() {
                    return;
                }
                let step = if event.keystroke.modifiers.shift {
                    1
                } else {
                    32
                };
                let delta = match event.keystroke.key.as_str() {
                    "left" => (-step, 0),
                    "right" => (step, 0),
                    "up" => (0, -step),
                    "down" => (0, step),
                    _ => return,
                };
                view.error = None;
                view.menu = None;
                if let Some(output) = view.draft.get_mut(view.selected) {
                    output.position = (
                        output.position.0.saturating_add(delta.0),
                        output.position.1.saturating_add(delta.1),
                    );
                    view.dirty = true;
                }
                cx.stop_propagation();
                cx.notify();
            }))
            .on_mouse_move(cx.listener(|view, event: &gpui::MouseMoveEvent, _, cx| {
                if let Some(drag) = view.drag {
                    if !event.dragging() {
                        view.drag = None;
                        cx.notify();
                        return;
                    }
                    let candidate = drag.canvas.moved(
                        drag.origin,
                        (
                            f32::from(event.position.x) - drag.start.0,
                            f32::from(event.position.y) - drag.start.1,
                        ),
                    );
                    let position = snapped_position(
                        &view.draft,
                        view.selected,
                        candidate,
                        (10. / drag.canvas.scale).round().max(1.) as i32,
                    );
                    if let Some(output) = view.draft.get_mut(view.selected) {
                        output.position = position;
                        view.dirty = true;
                    }
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|view, _, _, cx| {
                    if view.drag.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |view, event: &gpui::MouseDownEvent, _, cx| {
                    if view.menu.take().is_some() {
                        cx.stop_propagation();
                        cx.notify();
                    } else if !bounds.contains(&event.position) {
                        view.dismiss(cx);
                    }
                }),
            )
            .child(dojo::design::shell_extension(
                bounds,
                colors.background.rgb(),
                colors.border.rgb(),
                true,
            ))
            .child(
                div()
                    .id("display-editor")
                    .absolute()
                    .top(px(top))
                    .right(px(right))
                    .w(px(width))
                    .h(px(height * entry))
                    .overflow_y_scroll()
                    .child(
                        content
                            .absolute()
                            .top(px(0.))
                            .left(px(0.))
                            .w(px(width))
                            .opacity(((entry - 0.25) / 0.65).clamp(0., 1.)),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fitting_negative_coordinates_centers_the_monitor() {
        let output = OutputSettings {
            name: "DP-1".into(),
            enabled: true,
            position: (-1920, -1080),
            scale: 1.,
            mode: Some(niri_ipc::Mode {
                width: 1920,
                height: 1080,
                refresh_rate: 60000,
                is_preferred: true,
            }),
            transform: niri_ipc::Transform::Normal,
        };
        let canvas = Canvas::fit(&[output], 800., 300.);
        let (x, y) = canvas.position((-1920, -1080));
        assert!((x - 176.).abs() < 0.01);
        assert!((y - 24.).abs() < 0.01);
    }
    #[test]
    fn dragging_uses_the_captured_scale_in_logical_pixels() {
        let canvas = Canvas {
            scale: 0.25,
            offset: (100., 100.),
        };
        assert_eq!(canvas.moved((-1920, 40), (48., -8.)), (-1728, 8));
    }
    #[test]
    fn resolution_options_list_each_size_once_and_prefer_advertised_default() {
        let modes = [
            niri_ipc::Mode {
                width: 1920,
                height: 1080,
                refresh_rate: 144000,
                is_preferred: false,
            },
            niri_ipc::Mode {
                width: 1280,
                height: 720,
                refresh_rate: 60000,
                is_preferred: false,
            },
            niri_ipc::Mode {
                width: 1920,
                height: 1080,
                refresh_rate: 60000,
                is_preferred: true,
            },
        ];
        let choices = mode_choices(&modes, Some(modes[0]), Menu::Resolution);
        assert_eq!(
            choices
                .iter()
                .map(|m| (m.width, m.height, m.refresh_rate))
                .collect::<Vec<_>>(),
            vec![(1280, 720, 60000), (1920, 1080, 60000)]
        );
    }
    #[test]
    fn refresh_options_only_offer_rates_for_the_selected_resolution() {
        let modes = [
            niri_ipc::Mode {
                width: 1920,
                height: 1080,
                refresh_rate: 59940,
                is_preferred: true,
            },
            niri_ipc::Mode {
                width: 1280,
                height: 720,
                refresh_rate: 60000,
                is_preferred: false,
            },
            niri_ipc::Mode {
                width: 1920,
                height: 1080,
                refresh_rate: 144000,
                is_preferred: false,
            },
        ];
        let choices = mode_choices(&modes, Some(modes[0]), Menu::Refresh);
        assert_eq!(
            choices.iter().map(|m| m.refresh_rate).collect::<Vec<_>>(),
            vec![59940, 144000]
        );
    }
    #[gpui::test]
    fn closing_before_the_first_open_requires_no_display_service(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            close(cx);
            assert!(!cx.has_global::<DisplaysUi>());
        });
    }
}
