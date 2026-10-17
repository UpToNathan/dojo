use gpui::{Bounds, Pixels, Size, point, px};

pub const LEFT_PANEL_WIDTH: i32 = 64;
pub const TOP_BAR_HEIGHT: i32 = 24;
pub const RIGHT_INSET: i32 = 12;
pub const BOTTOM_INSET: i32 = 12;

pub const OPENING_RADIUS: f32 = 28.;
pub const TOP_BAR_CLOCK_PX: f32 = 11.;

/// The rounded opening the desktop frame leaves for windows.
pub fn desktop_opening(viewport: Size<Pixels>) -> Bounds<Pixels> {
    Bounds::from_corners(
        point(px(LEFT_PANEL_WIDTH as f32), px(TOP_BAR_HEIGHT as f32)),
        point(
            viewport.width - px(RIGHT_INSET as f32),
            viewport.height - px(BOTTOM_INSET as f32),
        ),
    )
}

pub fn top_bar_clock_bounds(viewport: Size<Pixels>, controls_width: f32) -> Bounds<Pixels> {
    let width = 240_f32
        .min((f32::from(viewport.width) - 2. * (controls_width + RIGHT_INSET as f32 + 8.)).max(0.));
    Bounds::new(
        point((viewport.width - px(width)) / 2., px(0.)),
        Size::new(px(width), px(TOP_BAR_HEIGHT as f32)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_leaves_the_panel_top_bar_and_insets() {
        let opening = desktop_opening(gpui::size(px(1920.), px(1080.)));
        assert_eq!(opening.origin, point(px(64.), px(24.)));
        assert_eq!(opening.bottom_right(), point(px(1908.), px(1068.)));
        assert_eq!(OPENING_RADIUS, 28.);
    }

    #[test]
    fn clock_is_centred_and_shrinks_before_touching_the_controls() {
        let clock = top_bar_clock_bounds(gpui::size(px(1920.), px(1080.)), 300.);
        assert_eq!(clock.origin, point(px(840.), px(0.)));
        assert_eq!(clock.size, gpui::size(px(240.), px(24.)));
        let narrow = top_bar_clock_bounds(gpui::size(px(800.), px(600.)), 300.);
        assert_eq!(narrow.origin.x, px(320.));
        assert_eq!(narrow.size.width, px(160.));
    }
}
