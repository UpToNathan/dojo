//! Pure geometry from the shell's resting frame to the lock screen's dial.
//!
//! Closing occasions evaluate [`layout`] at progress `t`; opening occasions
//! evaluate the same function at `1 - t`. Progress comes from the caller
//! (see [`progress`]), so this module holds no clock.
//!
//! The shell's clock label and the dial's time are different text, so each has
//! its own alpha and the painter draws whichever is visible.

use std::time::Duration;

use gpui::{Bounds, Pixels, Size, point, px, size};

use crate::geometry::{OPENING_RADIUS, TOP_BAR_CLOCK_PX, desktop_opening, top_bar_clock_bounds};

/// Matches the shell's top-bar controls on a typical output, so the closing and
/// the opening meet on the shell's own clock bounds there.
pub const SHELL_CONTROLS_WIDTH: f32 = 300.;
const SHELL_CLOCK_WEIGHT: f32 = 400.;
const DIAL_CLOCK_WEIGHT: f32 = 250.;
const DIAL_DIAMETER_OF_HEIGHT: f32 = 0.317;
const DIAL_CENTRE_Y_OF_HEIGHT: f32 = 0.417;
const DIAL_CLOCK_PX_OF_HEIGHT: f32 = 0.087;
/// Height of the shell clock's box relative to its text size (24 px bar, 11 px text).
const CLOCK_BOX_PER_PX: f32 = crate::geometry::TOP_BAR_HEIGHT as f32 / TOP_BAR_CLOCK_PX;

/// Everything that moves or fades between the shell frame and the dial.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub opening: Bounds<Pixels>,
    pub radius: Pixels,
    pub clock: Bounds<Pixels>,
    pub clock_px: f32,
    pub clock_weight: f32,
    /// Alpha of the shell's date and time label.
    pub shell_label: f32,
    /// Alpha of the dial's time.
    pub dial_time: f32,
    /// The frame colour around the opening. It stays under the wallpaper.
    pub surround: f32,
    pub wallpaper: f32,
    pub ring: f32,
    /// Alpha of the hint, greeting and farewell line. Not the clock.
    pub text: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Morph,
    /// Reduced motion: nothing moves and the two pictures only fade.
    Crossfade,
}

/// What to draw at one instant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pose {
    Moving(Layout),
    /// Draw `shell` at alpha `1 - mix` and `dial` at alpha `mix`.
    Fading {
        shell: Layout,
        dial: Layout,
        mix: f32,
    },
}

/// One part's staggered window of the overall progress.
#[derive(Clone, Copy)]
struct Span(f32, f32);

/// The workspace opening shrinks and rounds. It ends before the wallpaper
/// starts, because the closing's overlay can paint the wallpaper only over a
/// covered opening, not over the live desktop.
const OPENING: Span = Span(0.12, 0.66);
const CLOCK: Span = Span(0.10, 0.74);
/// Painted only once the opening is covered (see [`workspace_cover`]).
const WALLPAPER: Span = Span(0.62, 1.0);
const RING: Span = Span(0.60, 0.90);
const TEXT: Span = Span(0.84, 1.0);
/// The shell label fades out exactly as the dial time fades in, so the clock text never dips.
const LABELS: Span = Span(0.18, 0.56);
const FADE: Span = Span(0., 1.);
/// The overlay's frame closes over the top bar's controls and the side panel's
/// icons, which only the live frame windows draw, before the opening moves.
const FRAME_IN: Span = Span(0., OPENING.0);
/// The live workspace inside the shrinking opening gives way to the dial's disc.
const COVER: Span = Span(0.36, WALLPAPER.0);
/// Reduced motion: the opening is covered before anything else happens.
const CROSSFADE_COVER: Span = Span(0., 0.16);

/// Where `elapsed` stands in `total`, from 0 to 1 and linear. Every moving part
/// eases within its own window of this, so easing the whole run as well would
/// only squeeze the motion into its middle.
pub fn progress(elapsed: Duration, total: Duration) -> f32 {
    if total.is_zero() {
        return 1.;
    }
    (elapsed.as_secs_f32() / total.as_secs_f32()).clamp(0., 1.)
}

/// How much of the live workspace inside the opening is hidden at progress `t`:
/// clear at 0, the dial's disc once the opening has shrunk. Only the shell's
/// closing, whose overlay is transparent, has a workspace to hide.
pub fn workspace_cover(t: f32, style: Style) -> f32 {
    match style {
        Style::Morph => COVER.ease(t),
        Style::Crossfade => CROSSFADE_COVER.ease(t),
    }
}

/// How much of the shell's frame (surround, top-bar controls, side panel) the
/// overlay has painted over by progress `t`. The live frame windows under it
/// still draw their controls, so a closing starts on them and fades them out.
pub fn frame_cover(t: f32) -> f32 {
    FRAME_IN.ease(t)
}

impl Span {
    fn ease(self, t: f32) -> f32 {
        let x = ((t - self.0) / (self.1 - self.0)).clamp(0., 1.);
        x * x * (3. - 2. * x)
    }
}

/// `a` at 0 and `b` at 1 exactly, with no rounding error at either end.
fn mix(a: f32, b: f32, s: f32) -> f32 {
    if a == b {
        return a;
    }
    a * (1. - s) + b * s
}

fn mix_px(a: Pixels, b: Pixels, s: f32) -> Pixels {
    px(mix(f32::from(a), f32::from(b), s))
}

fn mix_bounds(a: Bounds<Pixels>, b: Bounds<Pixels>, s: f32) -> Bounds<Pixels> {
    Bounds::new(
        point(
            mix_px(a.origin.x, b.origin.x, s),
            mix_px(a.origin.y, b.origin.y, s),
        ),
        size(
            mix_px(a.size.width, b.size.width, s),
            mix_px(a.size.height, b.size.height, s),
        ),
    )
}

/// The resting desktop frame: workspace opening, top-bar clock, no lock-screen parts.
pub fn shell_layout(viewport: Size<Pixels>, controls_width: f32) -> Layout {
    Layout {
        opening: desktop_opening(viewport),
        radius: px(OPENING_RADIUS),
        clock: top_bar_clock_bounds(viewport, controls_width),
        clock_px: TOP_BAR_CLOCK_PX,
        clock_weight: SHELL_CLOCK_WEIGHT,
        shell_label: 1.,
        dial_time: 0.,
        surround: 1.,
        wallpaper: 0.,
        ring: 0.,
        text: 0.,
    }
}

/// The lock screen at rest: a centred disc with the large time inside it.
pub fn dial_layout(viewport: Size<Pixels>) -> Layout {
    let height = f32::from(viewport.height);
    let diameter = DIAL_DIAMETER_OF_HEIGHT * height;
    let centre = point(
        f32::from(viewport.width) / 2.,
        DIAL_CENTRE_Y_OF_HEIGHT * height,
    );
    let centred = |width: f32, height: f32| {
        Bounds::new(
            point(px(centre.x - width / 2.), px(centre.y - height / 2.)),
            size(px(width), px(height)),
        )
    };
    let clock_px = DIAL_CLOCK_PX_OF_HEIGHT * height;
    Layout {
        opening: centred(diameter, diameter),
        radius: px(diameter / 2.),
        clock: centred(diameter, clock_px * CLOCK_BOX_PER_PX),
        clock_px,
        clock_weight: DIAL_CLOCK_WEIGHT,
        shell_label: 0.,
        dial_time: 1.,
        surround: 1.,
        wallpaper: 1.,
        ring: 1.,
        text: 1.,
    }
}

/// Staggered blend of two layouts. Each part moves on its own window of `t`.
pub fn interpolate(a: &Layout, b: &Layout, t: f32) -> Layout {
    let opening = OPENING.ease(t);
    let clock = CLOCK.ease(t);
    let labels = LABELS.ease(t);
    Layout {
        opening: mix_bounds(a.opening, b.opening, opening),
        radius: mix_px(a.radius, b.radius, opening),
        clock: mix_bounds(a.clock, b.clock, clock),
        clock_px: mix(a.clock_px, b.clock_px, clock),
        clock_weight: mix(a.clock_weight, b.clock_weight, clock),
        shell_label: mix(a.shell_label, b.shell_label, labels),
        dial_time: mix(a.dial_time, b.dial_time, labels),
        surround: mix(a.surround, b.surround, t),
        wallpaper: mix(a.wallpaper, b.wallpaper, WALLPAPER.ease(t)),
        ring: mix(a.ring, b.ring, RING.ease(t)),
        text: mix(a.text, b.text, TEXT.ease(t)),
    }
}

/// The frame at progress `t` (0 is the shell frame, 1 the dial).
///
/// Both styles are exactly the shell at `t <= 0` and exactly the dial at
/// `t >= 1`. Between them `Crossfade` returns both pictures untouched.
pub fn layout(viewport: Size<Pixels>, controls_width: f32, t: f32, style: Style) -> Pose {
    let shell = shell_layout(viewport, controls_width);
    let dial = dial_layout(viewport);
    if t <= 0. {
        return Pose::Moving(shell);
    }
    if t >= 1. {
        return Pose::Moving(dial);
    }
    match style {
        Style::Morph => Pose::Moving(interpolate(&shell, &dial, t)),
        Style::Crossfade => Pose::Fading {
            shell,
            dial,
            mix: FADE.ease(t),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn viewports() -> [Size<Pixels>; 4] {
        [
            size(px(1920.), px(1080.)),
            size(px(1280.), px(800.)),
            size(px(1536.), px(864.)),
            size(px(1707.2), px(960.4)),
        ]
    }

    fn moving(pose: Pose) -> Layout {
        match pose {
            Pose::Moving(layout) => layout,
            Pose::Fading { .. } => panic!("expected a moving pose"),
        }
    }

    #[test]
    fn endpoints_are_exact_for_both_styles() {
        for viewport in viewports() {
            for controls in [0., 300.] {
                for style in [Style::Morph, Style::Crossfade] {
                    assert_eq!(
                        layout(viewport, controls, 0., style),
                        Pose::Moving(shell_layout(viewport, controls))
                    );
                    assert_eq!(
                        layout(viewport, controls, 1., style),
                        Pose::Moving(dial_layout(viewport))
                    );
                }
                assert_eq!(
                    interpolate(
                        &shell_layout(viewport, controls),
                        &dial_layout(viewport),
                        0.
                    ),
                    shell_layout(viewport, controls)
                );
                assert_eq!(
                    interpolate(
                        &shell_layout(viewport, controls),
                        &dial_layout(viewport),
                        1.
                    ),
                    dial_layout(viewport)
                );
            }
        }
    }

    #[test]
    fn dial_is_a_centred_disc_with_the_documented_proportions() {
        let dial = dial_layout(size(px(1920.), px(1080.)));
        let diameter = 0.317 * 1080.;
        assert_eq!(dial.opening.size, size(px(diameter), px(diameter)));
        assert_eq!(dial.radius, px(diameter / 2.));
        let centre = dial.opening.center();
        assert_eq!(centre.x, px(960.));
        assert!((f32::from(centre.y) - 0.417 * 1080.).abs() < 0.01);
        assert_eq!(dial.clock.center(), centre);
        assert!((dial.clock_px - 0.087 * 1080.).abs() < 0.01);
        assert_eq!(dial.clock_weight, 250.);
        assert_eq!((dial.ring, dial.text, dial.wallpaper), (1., 1., 1.));
        assert_eq!((dial.shell_label, dial.dial_time), (0., 1.));
    }

    #[test]
    fn shell_layout_is_the_geometry_module_verbatim() {
        let viewport = size(px(1920.), px(1080.));
        let shell = shell_layout(viewport, 300.);
        assert_eq!(shell.opening, desktop_opening(viewport));
        assert_eq!(shell.clock, top_bar_clock_bounds(viewport, 300.));
        assert_eq!(shell.radius, px(28.));
        assert_eq!(shell.clock_px, 11.);
        assert_eq!((shell.wallpaper, shell.ring, shell.text), (0., 0., 0.));
        assert_eq!((shell.shell_label, shell.dial_time), (1., 0.));
    }

    #[test]
    fn opening_shrinks_and_rounds_monotonically() {
        for viewport in viewports() {
            let mut previous = moving(layout(viewport, 300., 0., Style::Morph));
            for step in 1..=200 {
                let now = moving(layout(viewport, 300., step as f32 / 200., Style::Morph));
                assert!(now.opening.size.width <= previous.opening.size.width);
                assert!(now.opening.size.height <= previous.opening.size.height);
                assert!(now.radius >= previous.radius);
                assert!(now.clock_px >= previous.clock_px);
                assert!(now.shell_label <= previous.shell_label);
                assert!(now.dial_time >= previous.dial_time);
                assert!(now.wallpaper >= previous.wallpaper);
                assert!(now.ring >= previous.ring);
                assert!(now.text >= previous.text);
                previous = now;
            }
        }
    }

    #[test]
    fn parts_hold_until_their_window_opens() {
        let viewport = size(px(1920.), px(1080.));
        let shell = shell_layout(viewport, 300.);
        let early = moving(layout(viewport, 300., 0.09, Style::Morph));
        assert_eq!(early.clock, shell.clock);
        assert_eq!(early.opening, shell.opening);
        assert_eq!((early.shell_label, early.dial_time), (1., 0.));
        let mid = moving(layout(viewport, 300., 0.5, Style::Morph));
        assert!(mid.opening.size.width < shell.opening.size.width);
        assert!(mid.opening.size.width > dial_layout(viewport).opening.size.width);
        assert_eq!((mid.wallpaper, mid.ring, mid.text), (0., 0., 0.));
        let after_the_opening = moving(layout(viewport, 300., 0.8, Style::Morph));
        assert_eq!(after_the_opening.opening, dial_layout(viewport).opening);
        assert!(after_the_opening.wallpaper > 0. && after_the_opening.wallpaper < 1.);
        assert!(after_the_opening.ring > 0.);
        assert_eq!(after_the_opening.text, 0.);
        assert_eq!(
            (after_the_opening.shell_label, after_the_opening.dial_time),
            (0., 1.)
        );
    }

    #[test]
    fn the_wallpaper_waits_for_the_opening_to_be_covered_and_the_frame_for_nothing() {
        for step in 0..=1000 {
            let t = step as f32 / 1000.;
            let Pose::Moving(layout) = layout(size(px(1920.), px(1080.)), 300., t, Style::Morph)
            else {
                panic!("morph never fades")
            };
            if layout.wallpaper > 0. {
                assert_eq!(workspace_cover(t, Style::Morph), 1., "t = {t}");
            }
            if layout.opening != shell_layout(size(px(1920.), px(1080.)), 300.).opening {
                assert_eq!(frame_cover(t), 1., "t = {t}");
            }
        }
        assert_eq!(workspace_cover(0., Style::Morph), 0.);
        assert_eq!(frame_cover(0.), 0.);
        assert_eq!(frame_cover(1.), 1.);
    }

    #[test]
    fn progress_is_linear_and_clamped() {
        let total = Duration::from_millis(450);
        assert_eq!(progress(Duration::ZERO, total), 0.);
        assert_eq!(progress(Duration::from_millis(225), total), 0.5);
        assert_eq!(progress(Duration::from_secs(9), total), 1.);
        assert_eq!(progress(Duration::ZERO, Duration::ZERO), 1.);
    }

    #[test]
    fn both_clock_labels_are_partly_visible_while_one_hands_over_to_the_other() {
        let viewport = size(px(1920.), px(1080.));
        let crossover = moving(layout(viewport, 300., 0.37, Style::Morph));
        assert!(crossover.shell_label > 0. && crossover.shell_label < 1.);
        assert!(crossover.dial_time > 0. && crossover.dial_time < 1.);
    }

    #[test]
    fn clock_label_alphas_always_sum_to_one() {
        let viewport = size(px(1920.), px(1080.));
        for step in 0..=200 {
            let pose = moving(layout(viewport, 300., step as f32 / 200., Style::Morph));
            assert!(
                (pose.shell_label + pose.dial_time - 1.).abs() < 1e-6,
                "t = {}: {} + {}",
                step as f32 / 200.,
                pose.shell_label,
                pose.dial_time
            );
        }
        let middle = moving(layout(viewport, 300., 0.37, Style::Morph));
        assert!((middle.shell_label - 0.5).abs() < 1e-6);
    }

    #[test]
    fn crossfade_keeps_both_pictures_fixed_and_only_mixes() {
        for viewport in viewports() {
            let shell = shell_layout(viewport, 300.);
            let dial = dial_layout(viewport);
            let mut previous = 0.;
            for step in 1..100 {
                let t = step as f32 / 100.;
                let Pose::Fading {
                    shell: now_shell,
                    dial: now_dial,
                    mix,
                } = layout(viewport, 300., t, Style::Crossfade)
                else {
                    panic!("crossfade moved at t = {t}");
                };
                assert_eq!(now_shell, shell);
                assert_eq!(now_dial, dial);
                assert!(mix > 0. && mix < 1.);
                assert!(mix >= previous);
                previous = mix;
            }
            let Pose::Fading { mix, .. } = layout(viewport, 300., 0.5, Style::Crossfade) else {
                panic!("crossfade moved at t = 0.5");
            };
            assert_eq!(mix, 0.5);
        }
    }
}
