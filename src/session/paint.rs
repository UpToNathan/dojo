//! GPUI painting of a lock `Scene`: the desktop frame and wallpaper, the dial,
//! its tick ring, the clock texts, the password pill and the panel text.
//!
//! The locker draws its scenes with [`scene`], and the resident shell draws the
//! closing through the same function, so the two meet on identical pixels.
//! Placement is pure and tested here; only the `paint_*` functions touch a window.

use std::f32::consts::{FRAC_PI_2, TAU};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, App, BorderStyle, Bounds, Corners, Div, FontWeight, MouseButton, PathBuilder,
    Pixels, Point, RenderImage, Rgba, Size, Window, canvas, div, fill, point, prelude::*, px,
    size,
};

use crate::config::ThemeColors;
use crate::design::{paint_desktop_frame, paint_frame_shadow, shell_clock_element};
use crate::geometry::TOP_BAR_CLOCK_PX;
use crate::lock::{Leave, Line, Panel, Pill, Ring, Role, Scene, Tone};
use crate::session::transition::{self, Layout, Pose, Style};

const AGENT_INPUT_NOTICE: &str = "Agent input stopped while locked";
/// The power chip's glyph, from the same symbol font as the battery and network chips.
const POWER_GLYPH: &str = "\u{f011}";

/// A click on one of the login screen's chips.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChipClick {
    User,
    Session,
    Power,
    Leave(Leave),
}

/// Receives a click on a login chip. The lock screen has none and passes `None`.
pub type OnChip = Rc<dyn Fn(ChipClick, &mut Window, &mut App)>;

/// How much of the live desktop the closing's transparent overlay has covered.
/// The locker's surface is opaque, so it always draws [`Over::CLOSED`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Over {
    /// The opening's fill: 0 shows the workspace, 1 is the dial's disc.
    pub workspace: f32,
    /// The surround's opacity over the live frame windows and their controls.
    pub frame: f32,
}

impl Over {
    pub const CLOSED: Over = Over {
        workspace: 1.,
        frame: 1.,
    };

    /// What a closing at `progress` has covered. It is the shell's own picture at 0 and closed at 1.
    pub fn at(progress: f32, pose: &Pose) -> Over {
        let style = match pose {
            Pose::Moving(_) => Style::Morph,
            Pose::Fading { .. } => Style::Crossfade,
        };
        Over {
            workspace: transition::workspace_cover(progress, style),
            frame: transition::frame_cover(progress),
        }
    }
}

pub struct Palette {
    pub background: Rgba,
    pub foreground: Rgba,
    pub muted: Rgba,
    pub primary: Rgba,
    pub border: Rgba,
    /// Stands in for the wallpaper when the theme has not written one.
    pub flat_wallpaper: Rgba,
    pub error: Rgba,
}

impl Palette {
    pub fn new(colors: &ThemeColors, error: Rgba) -> Palette {
        Palette {
            background: colors.background.rgb(),
            foreground: colors.foreground.rgb(),
            muted: colors.foreground_muted.rgb(),
            primary: colors.primary.rgb(),
            border: colors.border.rgb(),
            flat_wallpaper: colors.surface_raised.rgb(),
            error,
        }
    }

    fn tone(&self, tone: Tone, normal: Rgba) -> Rgba {
        match tone {
            Tone::Normal => normal,
            Tone::Muted => self.muted,
            Tone::Wrong => self.error,
        }
    }
}

/// The theme as the lock screen draws it, read once by the caller.
pub struct Look {
    /// The shell's colours, for the shell clock label drawn by `design`.
    pub colors: ThemeColors,
    pub palette: Palette,
    /// The pre-blurred lock wallpaper; `None` paints a flat palette colour.
    pub wallpaper: Option<Arc<RenderImage>>,
}

impl Look {
    pub fn new(colors: ThemeColors, error: Rgba, wallpaper: Option<Arc<RenderImage>>) -> Look {
        Look {
            palette: Palette::new(&colors, error),
            colors,
            wallpaper,
        }
    }
}

/// How the login screen draws a look swap `mix` of the way from one user's look to the next.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Fade {
    /// Paint the new look; before the middle the old one is still the picture.
    pub incoming: bool,
    /// How much of the picture the veil hides: 1 at the middle, 0 at both ends.
    pub veil: f32,
    pub colour: Rgba,
}

/// GPUI cannot fade an image, so a swap dips instead: the old look is veiled until it is hidden,
/// and the new one is uncovered from the veil. The veil's colour moves from one background to the
/// other, so it is whole at the middle where the picture changes.
pub fn fade(mix: f32, from: Rgba, to: Rgba) -> Fade {
    let mix = mix.clamp(0., 1.);
    let lerp = |a: f32, b: f32| a + (b - a) * mix;
    Fade {
        incoming: mix >= 0.5,
        veil: 1. - (2. * mix - 1.).abs(),
        colour: Rgba {
            r: lerp(from.r, to.r),
            g: lerp(from.g, to.g),
            b: lerp(from.b, to.b),
            a: 1.,
        },
    }
}

/// The disc, its radius and the clock box of one layout after the typing shrink.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DialPlace {
    pub disc: Bounds<Pixels>,
    pub radius: Pixels,
    pub clock: Bounds<Pixels>,
    pub clock_px: f32,
}

pub fn place_dial(layout: &Layout, scale: f32) -> DialPlace {
    let centre = layout.opening.center();
    let about = |bounds: Bounds<Pixels>| {
        Bounds::new(
            point(
                centre.x + (bounds.origin.x - centre.x) * scale,
                centre.y + (bounds.origin.y - centre.y) * scale,
            ),
            size(bounds.size.width * scale, bounds.size.height * scale),
        )
    };
    DialPlace {
        disc: about(layout.opening),
        radius: layout.radius * scale,
        clock: about(layout.clock),
        clock_px: layout.clock_px * scale,
    }
}

/// The dial-side picture of a pose and its weight.
pub fn dial_part(pose: &Pose) -> (Layout, f32) {
    match *pose {
        Pose::Moving(layout) => (layout, 1.),
        Pose::Fading { dial, mix, .. } => (dial, mix),
    }
}

/// The shell-side picture of a pose and its weight.
pub fn shell_part(pose: &Pose) -> (Layout, f32) {
    match *pose {
        Pose::Moving(layout) => (layout, 1.),
        Pose::Fading { shell, mix, .. } => (shell, 1. - mix),
    }
}

/// One clock text: centred in `bounds` at `text_px`, drawn at `alpha`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClockText {
    pub bounds: Bounds<Pixels>,
    pub text_px: f32,
    pub weight: f32,
    pub alpha: f32,
}

/// The shell's date-and-time label and the dial's time at one pose.
///
/// Both ride the same moving clock box, each with its own alpha. The label
/// keeps the top bar's text size throughout, because it is long and would
/// overflow the box at the dial's size; only the time grows.
pub fn clock_texts(pose: &Pose, scale: f32) -> (ClockText, ClockText) {
    let (shell, shell_weight) = shell_part(pose);
    let (dial, dial_weight) = dial_part(pose);
    let label = ClockText {
        bounds: place_dial(&shell, 1.).clock,
        text_px: TOP_BAR_CLOCK_PX,
        weight: FontWeight::NORMAL.0,
        alpha: shell.shell_label * shell_weight,
    };
    let place = place_dial(&dial, scale);
    let time = ClockText {
        bounds: place.clock,
        text_px: place.clock_px,
        weight: dial.clock_weight,
        alpha: dial.dial_time * dial_weight,
    };
    (label, time)
}

/// Sizes of the panel below and around the dial, proportional to the output height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanelPlace {
    pub unit: f32,
    pub width: f32,
    pub pill_centre_y: f32,
    pub pill_height: f32,
    pub line_centre_y: f32,
    pub text_px: f32,
    pub chip_margin: f32,
    pub chip_height: f32,
    pub chip_px: f32,
}

pub fn place_panel(viewport: Size<Pixels>) -> PanelPlace {
    let height = f32::from(viewport.height);
    let unit = height / 1080.;
    PanelPlace {
        unit,
        width: f32::from(viewport.width),
        pill_centre_y: 0.627 * height,
        pill_height: 46. * unit,
        line_centre_y: 0.697 * height,
        text_px: (17. * unit).max(12.),
        chip_margin: (20. * unit).max(10.),
        chip_height: (30. * unit).max(22.),
        chip_px: (13. * unit).max(11.),
    }
}

const DOT: f32 = 9.;
const DOT_PITCH: f32 = 17.;
const PILL_PADDING: f32 = 28.;
const PILL_MIN: f32 = 220.;

pub fn pill_bounds(place: &PanelPlace, pill: &Pill) -> Bounds<Pixels> {
    let content = pill.dots as f32 * DOT_PITCH * place.unit;
    let width = (content + 2. * PILL_PADDING * place.unit)
        .max(PILL_MIN * place.unit)
        .min(place.width * 0.6);
    let left = (place.width - width) / 2. + f32::from(pill.offset_x);
    Bounds::new(
        point(px(left), px(place.pill_centre_y - place.pill_height / 2.)),
        size(px(width), px(place.pill_height)),
    )
}

/// Everything `scene` shows, bottom to top. The caller adds input handling and
/// any dimming above it.
pub fn scene(
    scene: &Scene,
    look: &Rc<Look>,
    viewport: Size<Pixels>,
    on_chip: Option<&OnChip>,
) -> Vec<AnyElement> {
    let palette = &look.palette;
    let panel = match &scene.role {
        Role::Active(panel) => Some(panel),
        Role::Passive => None,
    };
    let scale = panel.map_or(1., |panel| panel.dial.scale);
    let mut children = pose_elements(
        &scene.pose,
        scale,
        panel,
        Clocks {
            shell: scene.shell_clock.as_deref(),
            dial: &scene.clock,
            quiet: scene.quiet,
        },
        Over::CLOSED,
        look,
    );
    if let Some(panel) = panel {
        let (dial_layout, dial_weight) = dial_part(&scene.pose);
        children.extend(
            panel_elements(
                panel,
                &dial_layout,
                dial_weight,
                scale,
                viewport,
                palette,
                on_chip,
            )
            .into_iter()
                .map(IntoElement::into_any_element),
        );
    }
    children
}

/// The clock texts of one picture: the top-bar label, the dial's time and how loud the time is.
struct Clocks<'a> {
    shell: Option<&'a str>,
    dial: &'a str,
    quiet: f32,
}

/// The frame, wallpaper and the two clock texts at one pose, bottom to top.
fn pose_elements(
    pose: &Pose,
    scale: f32,
    panel: Option<&Panel>,
    clocks: Clocks,
    over: Over,
    look: &Rc<Look>,
) -> Vec<AnyElement> {
    let palette = &look.palette;
    let mut children = vec![shapes(*pose, scale, panel.cloned(), over, look.clone())];

    let (label, time) = clock_texts(pose, scale);
    if let Some(text) = clocks.shell
        && label.alpha > 0.
    {
        children.push(
            div()
                .absolute()
                .left(px(0.))
                .top(label.bounds.origin.y)
                .size_full()
                .opacity(label.alpha * over.frame)
                .font_weight(FontWeight(label.weight))
                .child(shell_clock_element(
                    text.to_owned(),
                    label.bounds,
                    label.text_px,
                    &look.colors,
                ))
                .into_any_element(),
        );
    }
    if time.alpha * clocks.quiet > 0. && !clocks.dial.is_empty() {
        children.push(
            centred_text(clocks.dial, time.bounds, time.text_px)
                .font_weight(FontWeight(time.weight))
                .text_color(palette.foreground.opacity(time.alpha * clocks.quiet))
                .into_any_element(),
        );
    }
    children
}

/// The shell's closing at one instant: the frame at `pose` with both clock
/// texts, and the farewell line at `farewell.1` alpha when there is one. The
/// window above the workspace is transparent, so `progress` also sets how much
/// of the workspace the opening still shows.
pub fn closing(
    pose: &Pose,
    progress: f32,
    shell_clock: &str,
    dial_clock: &str,
    farewell: Option<(&str, f32)>,
    look: &Rc<Look>,
    viewport: Size<Pixels>,
) -> Vec<AnyElement> {
    let clocks = Clocks {
        shell: Some(shell_clock),
        dial: dial_clock,
        quiet: 1.,
    };
    let mut children = pose_elements(pose, 1., None, clocks, Over::at(progress, pose), look);
    if let Some((text, alpha)) = farewell
        && alpha > 0.
    {
        let place = place_panel(viewport);
        children.push(
            message_line(
                text,
                place.line_centre_y,
                (place.text_px * 1.3).round(),
                look.palette.foreground.opacity(alpha),
                place.width,
            )
            .into_any_element(),
        );
    }
    children
}

/// The canvas under the text: frame, wallpaper, ring and pill.
fn shapes(pose: Pose, scale: f32, panel: Option<Panel>, over: Over, look: Rc<Look>) -> AnyElement {
    canvas(
        |_, _, _| {},
        move |bounds, (), window, _| {
            let palette = &look.palette;
            paint_frame_over(
                window,
                bounds,
                &pose,
                scale,
                palette,
                look.wallpaper.as_ref(),
                over,
            );
            let Some(panel) = panel else {
                return;
            };
            let (dial_layout, dial_weight) = dial_part(&pose);
            let place = place_panel(bounds.size);
            let dial = place_dial(&dial_layout, scale);
            paint_ring(
                window,
                &dial,
                panel.dial.ring,
                dial_layout.ring * dial_weight,
                place.unit,
                palette,
            );
            if let Some(pill) = &panel.pill {
                paint_pill(
                    window,
                    &place,
                    pill,
                    dial_layout.text * dial_weight,
                    palette,
                );
            }
        },
    )
    .absolute()
    .size_full()
    .into_any_element()
}

/// The date, the pause countdown, the hint or message line and the corner chips.
fn panel_elements(
    panel: &Panel,
    layout: &Layout,
    weight: f32,
    scale: f32,
    viewport: Size<Pixels>,
    palette: &Palette,
    on_chip: Option<&OnChip>,
) -> Vec<Div> {
    let place = place_panel(viewport);
    let alpha = layout.text * weight;
    let dial = place_dial(layout, scale);
    let mut children = Vec::new();
    // A farewell is its own picture, as in the shell's closing: it does not wait for the
    // parts to be there, because the login screen plays it after they have gone.
    let farewell = match &panel.line {
        Some(Line::Farewell { text, alpha: shown }) if *shown > 0. => Some(message_line(
            text,
            place.line_centre_y,
            (place.text_px * 1.3).round(),
            palette.foreground.opacity(*shown),
            place.width,
        )),
        _ => None,
    };
    if alpha <= 0. {
        children.extend(farewell);
        return children;
    }
    let date_y = f32::from(dial.disc.center().y) + f32::from(dial.radius) * 0.5;
    children.push(message_line(
        &panel.date,
        date_y,
        (place.text_px * scale.max(0.8)).round(),
        palette.muted.opacity(alpha),
        place.width,
    ));
    if let Some(pill) = &panel.pill
        && pill.dashed
    {
        let text = match pill.countdown_minutes {
            Some(1) => "Paused · 1 minute".to_owned(),
            Some(minutes) => format!("Paused · {minutes} minutes"),
            None => "Paused".to_owned(),
        };
        children.push(
            centred_text(&text, pill_bounds(&place, pill), place.text_px)
                .text_color(palette.muted.opacity(alpha)),
        );
    }
    match &panel.line {
        Some(Line::Hint(text)) => children.push(message_line(
            text,
            place.line_centre_y,
            place.text_px,
            palette.muted.opacity(alpha),
            place.width,
        )),
        Some(Line::Greeting { text, alpha: shown }) => children.push(message_line(
            text,
            place.line_centre_y,
            (place.text_px * 1.3).round(),
            palette.foreground.opacity(alpha * shown),
            place.width,
        )),
        Some(Line::Farewell { .. }) => children.extend(farewell),
        None => {}
    }
    let chip = |text: &str| {
        chip_base(&place, palette, alpha)
            .absolute()
            .child(text.to_owned())
    };
    let clickable = |chip: Div, click: ChipClick| clickable(chip, click, on_chip);
    let margin = px(place.chip_margin);
    let chips = &panel.chips;
    let editing = chips.login.as_ref().and_then(|login| login.editing.as_ref());
    let user = match editing {
        Some(edit) if edit.text.is_empty() => format!("{}  ›", chips.user),
        Some(edit) => format!("{}▏", edit.text),
        None => chips.user.clone(),
    };
    let mut user_chip = chip(&user).left(margin).top(margin);
    if editing.is_some_and(|edit| edit.refused) {
        user_chip = user_chip.text_color(palette.error.opacity(alpha));
    }
    children.push(clickable(user_chip, ChipClick::User));
    if let Some(login) = &chips.login {
        let below = margin + px(place.chip_height + place.chip_margin * 0.5);
        children.push(clickable(
            chip(&login.session).left(margin).top(below),
            ChipClick::Session,
        ));
        children.push(power_chips(
            login.power_open,
            &place,
            palette,
            alpha,
            on_chip,
        ));
    }
    let status: Vec<&str> = [chips.network.as_deref(), chips.battery.as_deref()]
        .into_iter()
        .flatten()
        .collect();
    if !status.is_empty() {
        children.push(chip(&status.join("   ")).right(margin).top(margin));
    }
    if let Some(age) = &chips.lock_age {
        children.push(chip(age).left(margin).bottom(margin));
    }
    if chips.login.is_none() {
        children.push(chip(AGENT_INPUT_NOTICE).right(margin).bottom(margin));
    }
    children
}

/// `chip`, answering a left click with `click` when the caller listens for clicks.
fn clickable(chip: Div, click: ChipClick, on_chip: Option<&OnChip>) -> Div {
    let Some(handler) = on_chip else {
        return chip;
    };
    let handler = handler.clone();
    chip.cursor_pointer()
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            cx.stop_propagation();
            handler(click, window, cx);
        })
}

/// A corner chip's look, not yet placed: the lock screen's and the login screen's share it.
fn chip_base(place: &PanelPlace, palette: &Palette, alpha: f32) -> Div {
    div()
        .h(px(place.chip_height))
        .px(px(place.chip_height * 0.5))
        .flex()
        .items_center()
        .gap(px(place.chip_height * 0.4))
        .rounded_full()
        .bg(palette.background.opacity(0.62 * alpha))
        .text_size(px(place.chip_px))
        .text_color(palette.muted.opacity(alpha))
}

/// The power chip in the bottom-left corner. Open, it is followed by one chip per occasion.
fn power_chips(
    open: bool,
    place: &PanelPlace,
    palette: &Palette,
    alpha: f32,
    on_chip: Option<&OnChip>,
) -> Div {
    let clickable = |chip: Div, click: ChipClick| clickable(chip, click, on_chip);
    let mut row = div()
        .absolute()
        .left(px(place.chip_margin))
        .bottom(px(place.chip_margin))
        .flex()
        .gap(px(place.chip_margin * 0.5))
        .child(clickable(
            chip_base(place, palette, alpha)
                .text_size(px(place.chip_px * 1.3))
                .child(POWER_GLYPH),
            ChipClick::Power,
        ));
    if open {
        for leave in Leave::ALL {
            row = row.child(clickable(
                chip_base(place, palette, alpha).child(leave.label()),
                ChipClick::Leave(leave),
            ));
        }
    }
    row
}

/// One full-width line of text centred on `centre_y`: a hint, a greeting, a farewell or the date.
pub fn message_line(text: &str, centre_y: f32, text_px: f32, colour: Rgba, width: f32) -> Div {
    let height = text_px * 2.;
    let bounds = Bounds::new(
        point(px(0.), px(centre_y - height / 2.)),
        size(px(width), px(height)),
    );
    centred_text(text, bounds, text_px).text_color(colour)
}

/// `text` centred in `bounds`. Nothing clips it, so text larger than its box still shows whole.
fn centred_text(text: &str, bounds: Bounds<Pixels>, text_px: f32) -> Div {
    div()
        .absolute()
        .left(bounds.origin.x)
        .top(bounds.origin.y)
        .w(bounds.size.width)
        .h(bounds.size.height)
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(text_px))
        .child(text.to_owned())
}

/// The frame, wallpaper and workspace opening. A `Fading` pose draws the shell
/// picture under the dial picture at `mix`; the surface stays opaque throughout.
pub fn paint_frame(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    pose: &Pose,
    scale: f32,
    palette: &Palette,
    wallpaper: Option<&Arc<RenderImage>>,
) {
    paint_frame_over(window, bounds, pose, scale, palette, wallpaper, Over::CLOSED);
}

/// [`paint_frame`] over the live desktop, as the shell's closing draws it. The
/// wallpaper is painted only once the opening is covered, because an opaque
/// image would otherwise hide the workspace the opening still shows.
pub fn paint_frame_over(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    pose: &Pose,
    scale: f32,
    palette: &Palette,
    wallpaper: Option<&Arc<RenderImage>>,
    over: Over,
) {
    let background = palette.background;
    let Over {
        workspace: cover,
        frame,
    } = over;
    // GPUI cannot fade an image, so the wallpaper is painted opaque and the
    // frame colour veils it by the inverse of its alpha.
    match *pose {
        Pose::Moving(layout) => {
            let dial = place_dial(&layout, scale);
            if layout.wallpaper > 0. && cover >= 1. {
                
                paint_wallpaper(window, bounds, wallpaper, palette.flat_wallpaper);
            }
            window.paint_quad(fill(dial.disc, background.opacity(cover)).corner_radii(dial.radius));
            let veil = layout.surround * (1. - layout.wallpaper) * frame;
            paint_desktop_frame(
                window,
                bounds,
                dial.disc,
                dial.radius,
                background.opacity(veil),
                palette.border,
            );
            if layout.wallpaper > 0. {
                paint_frame_shadow(
                    window,
                    dial.disc,
                    background.opacity(layout.wallpaper),
                    dial.radius,
                );
            }
        }
        Pose::Fading { shell, dial, mix } => {
            let shell = place_dial(&shell, 1.);
            let dial = place_dial(&dial, scale);
            if mix > 0. && cover >= 1. {
                paint_wallpaper(window, bounds, wallpaper, palette.flat_wallpaper);
            }
            window.paint_quad(
                fill(shell.disc, background.opacity((1. - mix) * cover)).corner_radii(shell.radius),
            );
            window.paint_quad(
                fill(dial.disc, background.opacity(mix * cover)).corner_radii(dial.radius),
            );
            paint_desktop_frame(
                window,
                bounds,
                shell.disc,
                shell.radius,
                background.opacity((1. - mix) * frame),
                palette.border.opacity(1. - mix),
            );
            paint_desktop_frame(
                window,
                bounds,
                dial.disc,
                dial.radius,
                background.opacity(0.),
                palette.border.opacity(mix),
            );
            paint_frame_shadow(window, dial.disc, background.opacity(mix), dial.radius);
        }
    }
}

/// Covers `bounds`, cropping the image's longer side.
fn paint_wallpaper(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    wallpaper: Option<&Arc<RenderImage>>,
    flat: Rgba,
) {
    let Some(image) = wallpaper else {
        window.paint_quad(fill(bounds, flat));
        return;
    };
    let pixels = image.size(0);
    let (width, height) = (pixels.width.0 as f32, pixels.height.0 as f32);
    if width <= 0. || height <= 0. {
        window.paint_quad(fill(bounds, flat));
        return;
    }
    // One texel past every edge: filtering the outermost texel would blend in the atlas's empty neighbour.
    let scale = (f32::from(bounds.size.width) / (width - 2.).max(1.))
        .max(f32::from(bounds.size.height) / (height - 2.).max(1.));
    let cover = size(px(width * scale), px(height * scale));
    let origin = point(
        bounds.origin.x + (bounds.size.width - cover.width) / 2.,
        bounds.origin.y + (bounds.size.height - cover.height) / 2.,
    );
    if let Err(error) = window.paint_image(
        bounds,
        Bounds::new(origin, cover),
        Corners::default(),
        image.clone(),
        0,
        false,
    ) {
        eprintln!("dojo: could not paint the lock wallpaper: {error}");
        window.paint_quad(fill(bounds, flat));
    }
}

const TICKS: usize = 60;

/// Sixty static ticks around the disc, a flash over them, and the clockwise fill on success.
pub fn paint_ring(
    window: &mut Window,
    dial: &DialPlace,
    ring: Ring,
    alpha: f32,
    unit: f32,
    palette: &Palette,
) {
    if alpha <= 0. {
        return;
    }
    let centre = dial.disc.center();
    let radius = f32::from(dial.radius);
    let base = match ring.tone {
        Tone::Normal => palette.foreground.opacity(0.42),
        Tone::Muted => palette.muted.opacity(0.22),
        Tone::Wrong => palette.error.opacity(0.7),
    };
    let filled = (ring.fill * TICKS as f32).ceil() as usize;
    paint_ticks(
        window,
        centre,
        radius,
        unit,
        filled..TICKS,
        base.opacity(alpha),
    );
    if ring.flash > 0. {
        paint_ticks(
            window,
            centre,
            radius,
            unit,
            0..TICKS,
            palette.error.opacity(alpha * ring.flash),
        );
    }
    paint_ticks(
        window,
        centre,
        radius,
        unit,
        0..filled,
        palette.primary.opacity(alpha),
    );
}

fn paint_ticks(
    window: &mut Window,
    centre: Point<Pixels>,
    radius: f32,
    unit: f32,
    range: std::ops::Range<usize>,
    colour: Rgba,
) {
    if range.is_empty() || colour.a <= 0. {
        return;
    }
    let mut builder = PathBuilder::stroke(px((2.5 * unit).max(1.5)));
    for index in range {
        let angle = index as f32 / TICKS as f32 * TAU - FRAC_PI_2;
        let (sin, cos) = angle.sin_cos();
        let inner = radius + if index % 5 == 0 { 9. } else { 14. } * unit;
        let outer = radius + 24. * unit;
        let at = |r: f32| point(centre.x + px(cos * r), centre.y + px(sin * r));
        builder.move_to(at(inner));
        builder.line_to(at(outer));
    }
    match builder.build() {
        Ok(path) => window.paint_path(path, colour),
        Err(error) => eprintln!("dojo: could not build the tick ring: {error}"),
    }
}

/// The pill outline and one dot per character. The countdown text is an element.
pub fn paint_pill(
    window: &mut Window,
    place: &PanelPlace,
    pill: &Pill,
    alpha: f32,
    palette: &Palette,
) {
    let bounds = pill_bounds(place, pill);
    let outline = palette.tone(pill.tone, palette.foreground.opacity(0.35));
    let mut quad = fill(bounds, palette.background.opacity(0.72 * alpha))
        .corner_radii(bounds.size.height / 2.)
        .border_widths(px(1.5))
        .border_color(outline.opacity(alpha));
    if pill.dashed {
        quad.border_style = BorderStyle::Dashed;
    }
    window.paint_quad(quad);
    let dot = DOT * place.unit;
    let pitch = DOT_PITCH * place.unit;
    let room = ((f32::from(bounds.size.width) - 2. * PILL_PADDING * place.unit) / pitch)
        .floor()
        .max(0.) as usize;
    let shown = pill.dots.min(room.max(1));
    let colour = palette.tone(pill.tone, palette.foreground).opacity(alpha);
    let row = shown as f32 * pitch - (pitch - dot);
    let left = f32::from(bounds.center().x) - row / 2.;
    let top = f32::from(bounds.center().y) - dot / 2.;
    for index in 0..shown {
        let origin = point(px(left + index as f32 * pitch), px(top));
        window.paint_quad(
            fill(Bounds::new(origin, size(px(dot), px(dot))), colour).corner_radii(px(dot / 2.)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::transition::{Style, dial_layout, layout, shell_layout};

    fn assert_near(a: Point<Pixels>, b: Point<Pixels>) {
        let gap = f32::from(a.x - b.x).abs() + f32::from(a.y - b.y).abs();
        assert!(gap < 0.01, "{a:?} is not {b:?}");
    }

    #[test]
    fn a_look_swap_dips_to_a_veil_that_is_whole_where_the_picture_changes() {
        let (dark, light) = (gpui::rgb(0x101010), gpui::rgb(0xf0f0f0));
        let at = |mix| fade(mix, dark, light);
        assert_eq!((at(0.).incoming, at(0.).veil, at(0.).colour), (false, 0., dark));
        assert_eq!((at(1.).incoming, at(1.).veil, at(1.).colour), (true, 0., light));
        assert_eq!(at(0.25).veil, 0.5);
        assert!(!at(0.25).incoming && at(0.75).incoming);
        // Both pictures meet the same opaque veil at the middle.
        let middle = at(0.5);
        assert_eq!((middle.incoming, middle.veil), (true, 1.));
        assert_eq!(at(0.499).veil, 0.998);
        assert!((middle.colour.r - 0.5 * (dark.r + light.r)).abs() < 1e-6);
        // Out-of-range input stays on the ends.
        assert_eq!(at(-3.).veil, 0.);
        assert_eq!(at(9.).colour, light);
    }

    #[test]
    fn the_typing_shrink_keeps_the_dial_centred() {
        let layout = dial_layout(size(px(1920.), px(1080.)));
        let rest = place_dial(&layout, 1.);
        let typing = place_dial(&layout, 0.5);
        assert_eq!(rest.disc, layout.opening);
        assert_near(typing.disc.center(), rest.disc.center());
        assert_eq!(typing.disc.size.width, rest.disc.size.width * 0.5);
        assert_near(typing.clock.center(), rest.clock.center());
        assert_eq!(typing.clock_px, layout.clock_px * 0.5);
    }

    #[test]
    fn the_shell_label_keeps_the_top_bar_size_while_the_time_grows_in_the_same_box() {
        let viewport = size(px(1318.), px(1626.));
        let dial_px = dial_layout(viewport).clock_px;
        for step in 0..=100 {
            let t = step as f32 / 100.;
            let (label, time) = clock_texts(&layout(viewport, 300., t, Style::Morph), 1.);
            assert_eq!(label.text_px, 11., "t = {t}");
            assert_eq!(label.weight, 400., "t = {t}");
            assert_eq!(label.bounds, time.bounds, "t = {t}");
            assert!(time.text_px >= 11. && time.text_px <= dial_px, "t = {t}");
        }
        let (label, time) = clock_texts(&layout(viewport, 300., 0.37, Style::Morph), 1.);
        assert!(time.text_px > 3. * label.text_px, "{} px", time.text_px);
        assert!(label.alpha > 0.4 && time.alpha > 0.4);
    }

    #[test]
    fn the_crossfade_draws_each_clock_text_where_it_rests() {
        let viewport = size(px(1920.), px(1080.));
        let (label, time) = clock_texts(&layout(viewport, 300., 0.5, Style::Crossfade), 0.72);
        assert_eq!(label.bounds, shell_layout(viewport, 300.).clock);
        assert_eq!((label.text_px, label.alpha, time.alpha), (11., 0.5, 0.5));
        assert_eq!(time.bounds, place_dial(&dial_layout(viewport), 0.72).clock);
        assert_eq!(time.text_px, dial_layout(viewport).clock_px * 0.72);
    }

    #[test]
    fn the_pill_grows_with_the_dots_and_follows_the_shake() {
        let place = place_panel(size(px(1920.), px(1080.)));
        let pill = |dots, offset| Pill {
            dots,
            tone: Tone::Normal,
            offset_x: px(offset),
            dashed: false,
            countdown_minutes: None,
        };
        let short = pill_bounds(&place, &pill(3, 0.));
        assert_eq!(short.size.width, px(220.));
        assert_eq!(short.center().x, px(960.));
        let long = pill_bounds(&place, &pill(40, 0.));
        assert_eq!(long.size.width, px(40. * 17. + 56.));
        let huge = pill_bounds(&place, &pill(400, 0.));
        assert_eq!(huge.size.width, px(1920. * 0.6));
        assert_eq!(
            pill_bounds(&place, &pill(3, 7.)).origin.x,
            short.origin.x + px(7.)
        );
    }

    #[test]
    fn the_panel_sits_below_the_resting_dial() {
        let viewport = size(px(1920.), px(1080.));
        let disc = dial_layout(viewport).opening;
        let place = place_panel(viewport);
        assert!(place.pill_centre_y - place.pill_height / 2. > f32::from(disc.bottom()));
        assert!(place.line_centre_y > place.pill_centre_y + place.pill_height / 2.);
    }

    #[test]
    fn the_closing_covers_the_workspace_only_after_the_opening_has_started_to_shrink() {
        let viewport = size(px(1920.), px(1080.));
        let at = |t: f32| Over::at(t, &layout(viewport, 300., t, Style::Morph));
        assert_eq!(
            at(0.),
            Over {
                workspace: 0.,
                frame: 0.
            }
        );
        assert_eq!(at(1.), Over::CLOSED);
        let shell = shell_layout(viewport, 300.);
        let mut painted_wallpaper = false;
        for step in 0..=100 {
            let t = step as f32 / 100.;
            let Pose::Moving(now) = layout(viewport, 300., t, Style::Morph) else {
                panic!("morph is always moving");
            };
            let over = at(t);
            // The overlay may paint the wallpaper only over a covered opening.
            if now.wallpaper > 0. {
                painted_wallpaper = true;
                assert_eq!(over.workspace, 1., "t = {t}");
            }
            // The shell's frame is whole before any of it moves.
            if now.opening != shell.opening {
                assert_eq!(over.frame, 1., "t = {t}");
            }
            // While the opening is still wide the workspace is still seen.
            if now.opening.size.width > shell.opening.size.width * 0.75 {
                assert!(over.workspace < 0.1, "t = {t}: {over:?}");
            }
        }
        assert!(painted_wallpaper);
    }

    #[test]
    fn the_crossfade_covers_the_opening_before_it_paints_a_wallpaper() {
        let viewport = size(px(1920.), px(1080.));
        for step in 0..=100 {
            let t = step as f32 / 100.;
            let pose = layout(viewport, 300., t, Style::Crossfade);
            let Pose::Fading { mix, .. } = pose else {
                continue;
            };
            if mix > 0. && Over::at(t, &pose).workspace < 1. {
                assert!(mix < 0.1, "t = {t}: mix {mix}");
            }
        }
    }
}
