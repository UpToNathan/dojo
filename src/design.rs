pub const BORDER_WIDTH: f32 = 1.;
pub const SHELL_BORDER_OPACITY: f32 = 0.1;
pub const BORDER_ALPHA: u8 = 0x33;
pub const BORDER_OPACITY: f32 = BORDER_ALPHA as f32 / 255.;
pub const PILL_RADIUS: f32 = 9999.;

pub const SURFACE_RADIUS: f32 = 22.;

fn shadow_color(background: gpui::Rgba) -> gpui::Hsla {
    let brightness = 0.2126 * background.r + 0.7152 * background.g + 0.0722 * background.b;
    gpui::Rgba {
        r: background.r * 0.12,
        g: background.g * 0.12,
        b: background.b * 0.12,
        a: (0.09 + 0.07 * (1. - brightness)) * background.a,
    }
    .into()
}

fn paint_menu_shadow(
    window: &mut gpui::Window,
    bounds: gpui::Bounds<gpui::Pixels>,
    background: gpui::Rgba,
    attached_right: bool,
    attached_bottom: bool,
) {
    use gpui::{Bounds, BoxShadow, ContentMask, Corners, point, px};
    let radius = px(SURFACE_RADIUS).min(bounds.size.height / 2.);
    let viewport = window.viewport_size();
    let mask = Bounds::from_corners(
        point(
            px(0.),
            if attached_bottom {
                px(0.)
            } else {
                bounds.top()
            },
        ),
        point(
            if attached_right {
                bounds.right()
            } else {
                viewport.width
            },
            if attached_bottom {
                bounds.bottom()
            } else {
                viewport.height
            },
        ),
    );
    let corners = Corners {
        top_left: if attached_bottom { radius } else { px(0.) },
        top_right: px(0.),
        bottom_left: if attached_bottom { px(0.) } else { radius },
        bottom_right: if attached_right { px(0.) } else { radius },
    };
    window.with_content_mask(Some(ContentMask { bounds: mask }), |window| {
        window.paint_drop_shadows(
            bounds,
            corners,
            &[BoxShadow {
                color: shadow_color(background),
                offset: point(px(0.), px(3.)),
                blur_radius: px(16.),
                spread_radius: px(0.),
                inset: false,
            }],
        );
    });
}

pub fn paint_frame_shadow(
    window: &mut gpui::Window,
    opening: gpui::Bounds<gpui::Pixels>,
    background: gpui::Rgba,
    radius: gpui::Pixels,
) {
    use gpui::{BoxShadow, Corners, point, px};
    window.paint_inset_shadows(
        opening,
        Corners::all(radius),
        &[BoxShadow {
            color: shadow_color(background).opacity(0.65),
            offset: point(px(0.), px(1.)),
            blur_radius: px(10.),
            spread_radius: px(0.),
            inset: true,
        }],
    );
}

pub fn paint_desktop_frame(
    window: &mut gpui::Window,
    bounds: gpui::Bounds<gpui::Pixels>,
    opening: gpui::Bounds<gpui::Pixels>,
    radius: gpui::Pixels,
    surround: gpui::Rgba,
    border: gpui::Rgba,
) {
    use gpui::{FillOptions, FillRule, PathBuilder, PathStyle, px};
    let mut builder = PathBuilder::fill().with_style(PathStyle::Fill(
        FillOptions::default().with_fill_rule(FillRule::NonZero),
    ));
    append_rounded_rectangle(&mut builder, bounds, px(0.));
    append_rounded_rectangle_reverse(&mut builder, opening, radius);

    match builder.build() {
        Ok(path) => window.paint_path(path, surround),
        Err(error) => {
            eprintln!("Failed to render Dojo's desktop surround: {error}")
        }
    }
    paint_frame_shadow(window, opening, surround, radius);
    let mut edge = PathBuilder::stroke(px(BORDER_WIDTH));
    append_rounded_rectangle(
        &mut edge,
        opening.dilate(px(-BORDER_WIDTH / 2.)),
        radius - px(BORDER_WIDTH / 2.),
    );
    if let Ok(edge) = edge.build() {
        window.paint_path(edge, border.opacity(SHELL_BORDER_OPACITY));
    }
}

pub fn shell_clock_element(
    label: impl Into<gpui::SharedString>,
    bounds: gpui::Bounds<gpui::Pixels>,
    text_px: f32,
    colors: &crate::config::ThemeColors,
) -> gpui::Stateful<gpui::Div> {
    use gpui::{InteractiveElement as _, ParentElement as _, Styled as _, div, px};
    div()
        .id("shell-clock")
        .absolute()
        .top(px(0.))
        .left(bounds.origin.x)
        .w(bounds.size.width)
        .h(bounds.size.height)
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(text_px))
        .overflow_hidden()
        .text_ellipsis()
        .text_color(colors.foreground.rgb())
        .child(label.into())
}

fn append_rounded_rectangle(
    builder: &mut gpui::PathBuilder,
    bounds: gpui::Bounds<gpui::Pixels>,
    radius: gpui::Pixels,
) {
    use gpui::{point, px};
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
    builder: &mut gpui::PathBuilder,
    bounds: gpui::Bounds<gpui::Pixels>,
    radius: gpui::Pixels,
) {
    use gpui::{point, px};
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

pub struct MenuReveal {
    from: f32,
    to: f32,
    started: std::time::Instant,
    duration: std::time::Duration,
}
impl MenuReveal {
    pub fn opening(now: std::time::Instant) -> Self {
        Self::opening_for(now, std::time::Duration::from_millis(100))
    }
    pub fn opening_for(now: std::time::Instant, duration: std::time::Duration) -> Self {
        Self {
            from: 0.,
            to: 1.,
            started: now,
            duration,
        }
    }
    pub fn set_open(&mut self, open: bool, now: std::time::Instant) {
        let to = if open { 1. } else { 0. };
        if self.to != to {
            self.from = self.sample(now).0;
            self.to = to;
            self.started = now;
        }
    }
    pub fn is_open(&self) -> bool {
        self.to == 1.
    }
    pub fn sample_with_motion(&self, now: std::time::Instant, reduced: bool) -> (f32, bool) {
        if reduced {
            (self.to, false)
        } else {
            self.sample(now)
        }
    }
    pub fn sample(&self, now: std::time::Instant) -> (f32, bool) {
        let t = (now.saturating_duration_since(self.started).as_secs_f32()
            / self.duration.as_secs_f32())
        .min(1.);
        let eased = t * t * t * (t * (t * 6. - 15.) + 10.);
        (self.from + (self.to - self.from) * eased, t < 1.)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    #[test]
    fn interrupted_menu_reveal_reverses_from_its_visible_shape() {
        let start = Instant::now();
        let mut reveal = MenuReveal::opening(start);
        assert_eq!(reveal.sample(start), (0., true));
        let middle = start + Duration::from_millis(50);
        assert_eq!(reveal.sample(middle).0, 0.5);
        reveal.set_open(false, middle);
        assert_eq!(reveal.sample(middle).0, 0.5);
        assert_eq!(
            reveal.sample(middle + Duration::from_millis(100)),
            (0., false)
        );
        reveal.set_open(true, middle + Duration::from_millis(100));
        assert_eq!(
            reveal.sample(middle + Duration::from_millis(200)),
            (1., false)
        );
    }
}

pub fn paint_shell_extension(
    window: &mut gpui::Window,
    bounds: gpui::Bounds<gpui::Pixels>,
    color: gpui::Rgba,
    border: gpui::Rgba,
    attached_right: bool,
) {
    use gpui::{PathBuilder, point, px};
    if bounds.size.height <= px(0.) {
        return;
    }
    paint_menu_shadow(window, bounds, color, attached_right, false);
    let radius = px(SURFACE_RADIUS).min(bounds.size.height / 2.);
    let left = bounds.left();
    let right = bounds.right();
    let top = bounds.top();
    let bottom = bounds.bottom();
    for outline in [false, true] {
        let mut path = if outline {
            PathBuilder::stroke(px(BORDER_WIDTH))
        } else {
            PathBuilder::fill()
        };
        path.move_to(point(left - radius, top));
        path.arc_to(
            point(radius, radius),
            px(0.),
            false,
            true,
            point(left, top + radius),
        );
        path.line_to(point(left, bottom - radius));
        path.arc_to(
            point(radius, radius),
            px(0.),
            false,
            false,
            point(left + radius, bottom),
        );
        path.line_to(point(right - radius, bottom));
        if attached_right {
            path.arc_to(
                point(radius, radius),
                px(0.),
                false,
                true,
                point(right, bottom + radius),
            );
            if !outline {
                path.line_to(point(window.viewport_size().width, bottom + radius));
                path.line_to(point(window.viewport_size().width, top));
            }
        } else {
            path.arc_to(
                point(radius, radius),
                px(0.),
                false,
                false,
                point(right, bottom - radius),
            );
            path.line_to(point(right, top + radius));
            path.arc_to(
                point(radius, radius),
                px(0.),
                false,
                true,
                point(right + radius, top),
            );
        }
        if !outline {
            path.close();
        }
        if let Ok(path) = path.build() {
            window.paint_path(
                path,
                if outline {
                    border.opacity(SHELL_BORDER_OPACITY)
                } else {
                    color
                },
            );
        }
    }
}

pub fn shell_extension(
    bounds: gpui::Bounds<gpui::Pixels>,
    color: gpui::Rgba,
    border: gpui::Rgba,
    attached_right: bool,
) -> gpui::AnyElement {
    use gpui::prelude::*;
    gpui::canvas(
        |_, _, _| (),
        move |_, (), window, _| {
            paint_shell_extension(window, bounds, color, border, attached_right)
        },
    )
    .absolute()
    .top(gpui::px(0.))
    .left(gpui::px(0.))
    .size_full()
    .into_any_element()
}

pub fn bottom_right_shell_extension(
    bounds: gpui::Bounds<gpui::Pixels>,
    color: gpui::Rgba,
    border: gpui::Rgba,
) -> gpui::AnyElement {
    use gpui::{PathBuilder, point, prelude::*, px};
    gpui::canvas(
        |_, _, _| (),
        move |_, (), window, _| {
            if bounds.size.height <= px(0.) {
                return;
            }
            paint_menu_shadow(window, bounds, color, true, true);
            let radius = px(SURFACE_RADIUS).min(bounds.size.height / 2.);
            let left = bounds.left();
            let right = bounds.right();
            let top = bounds.top();
            let bottom = bounds.bottom();
            for outline in [false, true] {
                let mut path = if outline {
                    PathBuilder::stroke(px(BORDER_WIDTH))
                } else {
                    PathBuilder::fill()
                };
                path.move_to(point(left - radius, bottom));
                path.arc_to(
                    point(radius, radius),
                    px(0.),
                    false,
                    false,
                    point(left, bottom - radius),
                );
                path.line_to(point(left, top + radius));
                path.arc_to(
                    point(radius, radius),
                    px(0.),
                    false,
                    true,
                    point(left + radius, top),
                );
                path.line_to(point(right - radius, top));
                path.arc_to(
                    point(radius, radius),
                    px(0.),
                    false,
                    false,
                    point(right, top - radius),
                );
                if !outline {
                    path.line_to(point(window.viewport_size().width, top - radius));
                    path.line_to(point(
                        window.viewport_size().width,
                        window.viewport_size().height,
                    ));
                    path.line_to(point(left - radius, window.viewport_size().height));
                    path.close();
                }
                if let Ok(path) = path.build() {
                    window.paint_path(
                        path,
                        if outline {
                            border.opacity(SHELL_BORDER_OPACITY)
                        } else {
                            color
                        },
                    );
                }
            }
        },
    )
    .absolute()
    .top(px(0.))
    .left(px(0.))
    .size_full()
    .into_any_element()
}
