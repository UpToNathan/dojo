use std::collections::BTreeMap;
use std::process::Command;

use gpui::{
    App, Bounds, Context, DisplayId, FillOptions, FillRule, PathBuilder, PathStyle, Size, Window,
    WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions, div, layer_shell::Anchor,
    point, prelude::*, px, rgb,
};
use gpui_platform::application;
use serde::Deserialize;

struct DesktopFrame;
struct EdgeReservation;

#[derive(Deserialize)]
struct NiriWindow {
    id: u64,
    workspace_id: Option<u64>,
    is_floating: bool,
    layout: NiriWindowLayout,
}

#[derive(Deserialize)]
struct NiriWindowLayout {
    pos_in_scrolling_layout: Option<(usize, usize)>,
    tile_size: (f64, f64),
}

#[derive(Clone, Copy)]
struct WindowWidthSnapshot {
    id: u64,
    width: i32,
}

fn niri_windows() -> anyhow::Result<Vec<NiriWindow>> {
    let output = Command::new("niri")
        .args(["msg", "--json", "windows"])
        .output()?;
    anyhow::ensure!(
        output.status.success(),
        "niri msg --json windows failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(serde_json::from_slice(&output.stdout)?)
}

fn snapshot_column_widths() -> anyhow::Result<Vec<WindowWidthSnapshot>> {
    let mut columns = BTreeMap::new();
    for window in niri_windows()? {
        if window.is_floating {
            continue;
        }
        let (Some(workspace_id), Some((column_index, _))) =
            (window.workspace_id, window.layout.pos_in_scrolling_layout)
        else {
            continue;
        };

        columns
            .entry((workspace_id, column_index))
            .or_insert_with(|| WindowWidthSnapshot {
                id: window.id,
                width: window.layout.tile_size.0.round() as i32,
            });
    }
    Ok(columns.into_values().collect())
}

fn set_window_width(snapshot: WindowWidthSnapshot, width: i32) -> anyhow::Result<()> {
    let id = snapshot.id.to_string();
    let width = width.to_string();
    let output = Command::new("niri")
        .args(["msg", "action", "set-window-width", "--id", &id, &width])
        .output()?;
    anyhow::ensure!(
        output.status.success(),
        "niri failed to set width for window {}: {}",
        snapshot.id,
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

fn resize_existing_columns(snapshots: &[WindowWidthSnapshot]) -> anyhow::Result<()> {
    for snapshot in snapshots {
        set_window_width(*snapshot, (snapshot.width - 24).max(120))?;
    }
    Ok(())
}

fn restore_column_widths(snapshots: &[WindowWidthSnapshot]) {
    for snapshot in snapshots {
        if let Err(error) = set_window_width(*snapshot, snapshot.width) {
            eprintln!(
                "Failed to restore Niri window {} width: {error:#}",
                snapshot.id
            );
        }
    }
}

fn open_edge_reservations(
    cx: &mut App,
    display_id: DisplayId,
    display: Bounds<gpui::Pixels>,
) -> anyhow::Result<()> {
    let reservations = [
        (
            Anchor::TOP | Anchor::LEFT | Anchor::RIGHT,
            Anchor::TOP,
            Size::new(display.size.width, px(12.)),
            "top",
        ),
        (
            Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
            Anchor::BOTTOM,
            Size::new(display.size.width, px(12.)),
            "bottom",
        ),
        (
            Anchor::LEFT | Anchor::TOP | Anchor::BOTTOM,
            Anchor::LEFT,
            Size::new(px(12.), display.size.height),
            "left",
        ),
        (
            Anchor::RIGHT | Anchor::TOP | Anchor::BOTTOM,
            Anchor::RIGHT,
            Size::new(px(12.), display.size.height),
            "right",
        ),
    ];

    for (anchor, edge, size, name) in reservations {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: display.origin,
                    size,
                })),
                titlebar: None,
                app_id: Some("khanda".to_string()),
                window_background: WindowBackgroundAppearance::Transparent,
                kind: WindowKind::LayerShell(gpui::layer_shell::LayerShellOptions {
                    namespace: format!("khanda-reservation-{name}-{}", u64::from(display_id)),
                    layer: gpui::layer_shell::Layer::Top,
                    anchor,
                    exclusive_zone: Some(px(12.)),
                    exclusive_edge: Some(edge),
                    keyboard_interactivity: gpui::layer_shell::KeyboardInteractivity::None,
                    ..Default::default()
                }),
                display_id: Some(display_id),
                ..Default::default()
            },
            |_, cx| cx.new(|_| EdgeReservation),
        )?;
    }

    Ok(())
}

fn open_frame(
    cx: &mut App,
    display_id: DisplayId,
    bounds: Bounds<gpui::Pixels>,
) -> anyhow::Result<()> {
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: None,
            app_id: Some("khanda".to_string()),
            display_id: Some(display_id),
            window_background: WindowBackgroundAppearance::Transparent,
            kind: WindowKind::LayerShell(gpui::layer_shell::LayerShellOptions {
                namespace: format!("khanda-frame-{}", u64::from(display_id)),
                layer: gpui::layer_shell::Layer::Overlay,
                anchor: Anchor::TOP | Anchor::RIGHT | Anchor::BOTTOM | Anchor::LEFT,
                exclusive_zone: Some(px(-1.)),
                keyboard_interactivity: gpui::layer_shell::KeyboardInteractivity::None,
                ..Default::default()
            }),
            ..Default::default()
        },
        |_, cx| cx.new(|_| DesktopFrame),
    )?;
    Ok(())
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

impl Render for DesktopFrame {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // Keep the full-output layer visible without intercepting pointer or touch input.
        window.set_input_region(Some(&[]));

        div().size_full().child(
            gpui::canvas(
                |_, _, _| (),
                |bounds, (), window, _| {
                    let inset = px(12.);
                    let inner_bounds = Bounds::from_corners(
                        point(bounds.left() + inset, bounds.top() + inset),
                        point(bounds.right() - inset, bounds.bottom() - inset),
                    );
                    let mut builder = PathBuilder::fill().with_style(PathStyle::Fill(
                        FillOptions::default().with_fill_rule(FillRule::EvenOdd),
                    ));
                    append_rounded_rectangle(&mut builder, bounds, px(0.));
                    append_rounded_rectangle(&mut builder, inner_bounds, px(28.));

                    match builder.build() {
                        Ok(path) => window.paint_path(path, rgb(0x1e1e2e)),
                        Err(error) => {
                            eprintln!("Failed to render Khanda's desktop surround: {error}")
                        }
                    }
                },
            )
            .size_full(),
        )
    }
}

impl Render for EdgeReservation {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        window.set_input_region(Some(&[]));
        div().size_full()
    }
}

fn main() {
    let original_widths = match snapshot_column_widths() {
        Ok(widths) => widths,
        Err(error) => {
            eprintln!("Could not read Niri windows for frame resizing: {error:#}");
            Vec::new()
        }
    };
    let widths_for_run = original_widths.clone();
    application().run(move |cx: &mut App| {
        let displays = cx.displays();
        for display in displays {
            let display_id = display.id();
            let bounds = display.bounds();
            if let Err(error) = open_edge_reservations(cx, display_id, bounds)
                .and_then(|()| open_frame(cx, display_id, bounds))
            {
                eprintln!("Failed to open Khanda on display {display_id:?}: {error:#}");
                cx.quit();
                return;
            }
        }

        if let Err(error) = resize_existing_columns(&widths_for_run) {
            eprintln!("Failed to resize Niri columns for Khanda: {error:#}");
        }
    });
    restore_column_widths(&original_widths);
}
