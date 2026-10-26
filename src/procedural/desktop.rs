//! Where each monitor sits on a procedural wallpaper's canvas.
//!
//! The canvas is the bounding box of the monitors in the compositor's logical coordinates, which
//! already account for scale and rotation. Spanned, each monitor samples the rectangle it
//! occupies, so a gap between monitors skips the picture behind it. Filled ("each monitor"), each
//! monitor draws a whole picture in its own shape, at its own pixels, rather than a crop of a
//! canvas shaped for the whole desktop.
//!
//! Unlike a photograph, nothing here decides how much to enlarge: the picture is drawn at each
//! monitor's own pixels, so the monitors' sizes only choose what is in view.

use anyhow::Context as _;

use crate::procedural::Frame;
use crate::theme::wallpaper_layout::{Monitor, Region, WallpaperLayout};

/// What a spanned wallpaper shows on each monitor, in the order given, with the canvas's width
/// over its height. `None` for no monitors or a monitor without area.
pub fn spanned(monitors: &[Monitor]) -> Option<(f64, Vec<Region>)> {
    if monitors.is_empty()
        || monitors
            .iter()
            .any(|monitor| monitor.width <= 0. || monitor.height <= 0.)
    {
        return None;
    }
    let left = monitors.iter().map(|m| m.x).fold(f64::INFINITY, f64::min);
    let top = monitors.iter().map(|m| m.y).fold(f64::INFINITY, f64::min);
    let right = monitors
        .iter()
        .map(|m| m.x + m.width)
        .fold(f64::NEG_INFINITY, f64::max);
    let bottom = monitors
        .iter()
        .map(|m| m.y + m.height)
        .fold(f64::NEG_INFINITY, f64::max);
    let (width, height) = (right - left, bottom - top);
    let regions = monitors
        .iter()
        .map(|monitor| Region {
            left: (monitor.x - left) / width,
            top: (monitor.y - top) / height,
            width: monitor.width / width,
            height: monitor.height / height,
        })
        .collect();
    Some((width / height, regions))
}

/// What one monitor draws of a procedural wallpaper: the shape of the canvas it samples (its width
/// over its height) and the part of that canvas it shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub aspect: f64,
    /// How tall that canvas is in the scene's unit (see [`crate::procedural::View`]).
    pub height: f64,
    pub region: Region,
    /// Where the scene is composed on the canvas; `None` for the canvas itself.
    pub frame: Option<Frame>,
}

/// The height of the shortest monitor: the scene's unit, so that on a monitor that tall the scene is
/// exactly the size it is on a desktop of one monitor, and monitors of other heights show more of it
/// rather than a bigger one.
fn shortest_height(monitors: &[Monitor]) -> f64 {
    monitors.iter().map(|m| m.height).fold(f64::INFINITY, f64::min)
}

pub fn canvas_height(monitors: &[Monitor]) -> Option<f64> {
    let top = monitors.iter().map(|m| m.y).fold(f64::INFINITY, f64::min);
    let bottom = monitors.iter().map(|m| m.y + m.height).fold(f64::NEG_INFINITY, f64::max);
    let shortest = shortest_height(monitors);
    (shortest > 0. && bottom > top).then(|| (bottom - top) / shortest)
}

/// Where a scene is composed on a canvas across `monitors`. Across, it spans the shortest monitors. Down,
/// its horizon sits halfway down the tallest monitor, so a portrait monitor shows sky over its upper half
/// and land below, and a landscape one beside it shows the part of the same picture at its place. The scene
/// is exactly the size it is on one monitor; the sky continues above it and the land below.
fn frame_of(monitors: &[Monitor]) -> Option<Frame> {
    let unit = shortest_height(monitors);
    let tolerance = 0.005 * unit;
    let shortest: Vec<&Monitor> = monitors.iter().filter(|m| (m.height - unit).abs() <= tolerance).collect();
    let left = shortest.iter().map(|m| m.x).fold(f64::INFINITY, f64::min);
    let right = shortest.iter().map(|m| m.x + m.width).fold(f64::NEG_INFINITY, f64::max);
    let canvas_left = monitors.iter().map(|m| m.x).fold(f64::INFINITY, f64::min);
    let canvas_top = monitors.iter().map(|m| m.y).fold(f64::INFINITY, f64::min);
    // The tallest monitor, the topmost when several are as tall.
    let tallest = monitors.iter().fold(None::<&Monitor>, |best, m| match best {
        Some(b) if b.height > m.height + tolerance || ((b.height - m.height).abs() <= tolerance && b.y <= m.y) => Some(b),
        _ => Some(m),
    })?;
    let horizon = tallest.y + 0.5 * tallest.height;
    (unit > 0. && right > left).then(|| Frame {
        left: (left - canvas_left) / unit,
        top: (horizon - canvas_top) / unit - crate::procedural::HORIZON,
        width: (right - left) / unit,
    })
}

/// What each monitor draws under `layout`, in the order given, with the whole desktop's aspect.
/// Spanned, every monitor samples the rectangle it occupies of one canvas across the desktop.
/// Filled, each draws a whole canvas in its own shape, so a portrait monitor gets a portrait picture
/// and nothing is cropped away or stretched to fit.
pub fn placements(monitors: &[Monitor], layout: WallpaperLayout) -> Option<(f64, Vec<Placement>)> {
    let (aspect, regions) = spanned(monitors)?;
    let height = canvas_height(monitors)?;
    let frame = frame_of(monitors)?;
    let placements = monitors
        .iter()
        .zip(regions)
        .map(|(monitor, spanned)| match layout {
            WallpaperLayout::Span => Placement { aspect, height, region: spanned, frame: Some(frame) },
            WallpaperLayout::Fill => Placement {
                aspect: monitor.width / monitor.height,
                height: 1.,
                region: whole(),
                frame: None,
            },
        })
        .collect();
    Some((aspect, placements))
}

pub fn whole() -> Region {
    Region {
        left: 0.,
        top: 0.,
        width: 1.,
        height: 1.,
    }
}

/// An output Niri has laid out: where it is in logical coordinates and how many pixels each logical
/// unit is.
#[derive(Clone, Debug)]
pub struct Output {
    pub name: String,
    pub monitor: Monitor,
    pub scale: f64,
}

/// The compositor's outputs with a logical position, by name.
pub fn niri_monitors() -> anyhow::Result<Vec<(String, Monitor)>> {
    Ok(niri_outputs()?
        .into_iter()
        .map(|output| (output.name, output.monitor))
        .collect())
}

/// The compositor's outputs with a logical position, sorted by name, each with its scale.
pub fn niri_outputs() -> anyhow::Result<Vec<Output>> {
    let mut socket = niri_ipc::socket::Socket::connect().context("connecting to Niri")?;
    socket.set_read_timeout(Some(std::time::Duration::from_secs(2)))?;
    socket.set_write_timeout(Some(std::time::Duration::from_secs(2)))?;
    let response = socket
        .send(niri_ipc::Request::Outputs)?
        .map_err(anyhow::Error::msg)?;
    let niri_ipc::Response::Outputs(outputs) = response else {
        anyhow::bail!("Niri returned an unexpected response to Outputs");
    };
    let mut monitors: Vec<_> = outputs
        .into_values()
        .filter_map(|output| {
            let logical = output.logical?;
            Some(Output {
                name: output.name,
                monitor: Monitor {
                    x: f64::from(logical.x),
                    y: f64::from(logical.y),
                    width: f64::from(logical.width),
                    height: f64::from(logical.height),
                },
                scale: logical.scale,
            })
        })
        .collect();
    monitors.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(monitors)
}

/// The canvas's width over its height for the monitors Niri has now, or 16:9 without Niri.
pub fn current_aspect() -> f64 {
    niri_monitors()
        .ok()
        .and_then(|monitors| {
            let monitors: Vec<_> = monitors.into_iter().map(|(_, monitor)| monitor).collect();
            spanned(&monitors)
        })
        .map_or(16. / 9., |(aspect, _)| aspect)
}

/// The shortest long edge of the picture retained for the lock and login screens and palette
/// derivation, and the longest.
const RETAINED_EDGES: (u32, u32) = (1920, 3840);

/// The shape, scene height and long edge of the picture retained for a generated wallpaper.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Retained {
    pub aspect: f64,
    pub height: f64,
    pub frame: Option<Frame>,
    pub edge: u32,
}

/// The picture retained for a generated wallpaper under `layout`. Spanned it is the whole canvas;
/// for each monitor it is the first monitor's own picture. Either way its long edge covers the
/// largest monitor's own pixels, so the lock screen is not made from a small picture stretched up.
pub fn retained(outputs: &[Output], layout: WallpaperLayout) -> Retained {
    let monitors: Vec<Monitor> = outputs.iter().map(|output| output.monitor).collect();
    let Some((_, placements)) = placements(&monitors, layout) else {
        return Retained { aspect: 16. / 9., height: 1., frame: None, edge: RETAINED_EDGES.0 };
    };
    let widest = outputs
        .iter()
        .map(|output| output.monitor.width.max(output.monitor.height) * output.scale)
        .fold(0., f64::max);
    let shown = placements[0];
    // Spanned, the retained picture is the whole canvas, not one monitor's part of it.
    let aspect = match layout {
        WallpaperLayout::Span => spanned(&monitors).map_or(shown.aspect, |(aspect, _)| aspect),
        WallpaperLayout::Fill => shown.aspect,
    };
    Retained { aspect, height: shown.height, frame: shown.frame, edge: (widest.round() as u32).clamp(RETAINED_EDGES.0, RETAINED_EDGES.1) }
}

/// A miniature of the desktop as `layout` will draw it: every monitor's own picture at its place,
/// `edge` pixels along the desktop's longer side, with the ground showing in any gap. Each monitor is
/// drawn at the miniature's own pixels, not cut from one picture, so it shows what that monitor will.
pub fn preview_image(
    recipe: &crate::procedural::Recipe,
    monitors: &[Monitor],
    layout: WallpaperLayout,
    edge: u32,
) -> Option<image::RgbaImage> {
    let (_, placements) = placements(monitors, layout)?;
    let left = monitors.iter().map(|m| m.x).fold(f64::INFINITY, f64::min);
    let top = monitors.iter().map(|m| m.y).fold(f64::INFINITY, f64::min);
    let right = monitors.iter().map(|m| m.x + m.width).fold(f64::NEG_INFINITY, f64::max);
    let bottom = monitors.iter().map(|m| m.y + m.height).fold(f64::NEG_INFINITY, f64::max);
    let scale = f64::from(edge.max(1)) / (right - left).max(bottom - top);
    let size = (
        ((right - left) * scale).round().max(1.) as u32,
        ((bottom - top) * scale).round().max(1.) as u32,
    );
    let [red, green, blue] = recipe.palette.background.0;
    let mut desktop = image::RgbaImage::from_pixel(size.0, size.1, image::Rgba([red, green, blue, 255]));
    for (monitor, placement) in monitors.iter().zip(placements) {
        let (x, y) = (((monitor.x - left) * scale).round() as i64, ((monitor.y - top) * scale).round() as i64);
        let (width, height) = (
            ((monitor.width * scale).round().max(1.)) as u32,
            ((monitor.height * scale).round().max(1.)) as u32,
        );
        let tile = recipe.render(
            crate::procedural::View {
                aspect: placement.aspect,
                height: placement.height,
                region: placement.region,
                frame: placement.frame,
            },
            width,
            height,
        );
        image::imageops::replace(&mut desktop, &tile, x, y);
    }
    Some(desktop)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(x: f64, y: f64, width: f64, height: f64) -> Monitor {
        Monitor {
            x,
            y,
            width,
            height,
        }
    }

    fn close(left: f64, right: f64) -> bool {
        (left - right).abs() < 1e-9
    }

    #[test]
    fn side_by_side_monitors_take_halves_of_the_canvas() {
        let (aspect, regions) = spanned(&[
            monitor(0., 0., 1920., 1080.),
            monitor(1920., 0., 1920., 1080.),
        ])
        .unwrap();
        assert!(close(aspect, 3840. / 1080.));
        assert_eq!(
            regions[1],
            Region {
                left: 0.5,
                top: 0.,
                width: 0.5,
                height: 1.
            }
        );
    }

    #[test]
    fn a_gap_is_skipped_and_the_origin_need_not_be_zero() {
        // A portrait monitor left of a landscape one, tops 200 apart, in negative coordinates.
        let monitors = [
            monitor(-1080., -200., 1080., 1920.),
            monitor(100., 0., 1920., 1080.),
        ];
        let (aspect, regions) = spanned(&monitors).unwrap();
        assert!(close(aspect, 3100. / 1920.));
        assert!(close(regions[0].left, 0.));
        assert!(close(regions[0].height, 1.));
        assert!(close(regions[1].left, 1180. / 3100.));
        assert!(close(regions[1].top, 200. / 1920.));
        // The gap between them belongs to neither.
        assert!(regions[0].left + regions[0].width < regions[1].left);
    }

    #[test]
    fn degenerate_arrangements_have_no_regions() {
        assert!(spanned(&[]).is_none());
        assert!(spanned(&[monitor(0., 0., 0., 100.)]).is_none());
    }

    fn two_monitors() -> [Monitor; 2] {
        // A landscape monitor and a portrait one beside it.
        [monitor(0., 0., 1920., 1080.), monitor(1920., 0., 1080., 1920.)]
    }

    #[test]
    fn spanned_monitors_share_one_canvas_and_each_samples_its_own_rectangle() {
        let (aspect, placements) = placements(&two_monitors(), WallpaperLayout::Span).unwrap();
        assert!(close(aspect, 3000. / 1920.));
        for placement in &placements {
            assert!(close(placement.aspect, aspect), "one canvas for every monitor");
        }
        assert!(close(placements[0].region.left, 0.) && close(placements[0].region.width, 1920. / 3000.));
        assert!(close(placements[1].region.left, 1920. / 3000.) && close(placements[1].region.height, 1.));
    }

    #[test]
    fn each_monitor_draws_a_whole_picture_in_its_own_shape() {
        let (_, placements) = placements(&two_monitors(), WallpaperLayout::Fill).unwrap();
        assert!(close(placements[0].aspect, 1920. / 1080.));
        assert!(close(placements[1].aspect, 1080. / 1920.), "a portrait monitor gets a portrait picture");
        for placement in placements {
            assert_eq!(placement.region, whole(), "nothing is cropped away");
        }
        assert!(placements_empty_is_none());
    }

    fn placements_empty_is_none() -> bool {
        placements(&[], WallpaperLayout::Fill).is_none() && placements(&[monitor(0., 0., 0., 10.)], WallpaperLayout::Span).is_none()
    }

    fn output(name: &str, monitor: Monitor, scale: f64) -> Output {
        Output { name: name.into(), monitor, scale }
    }

    #[test]
    fn the_retained_picture_follows_the_layout_and_covers_the_largest_monitors_pixels() {
        // A 4K monitor at scale 2 (1920x1080 logical) beside a 1080p portrait one.
        let outputs = [
            output("A", monitor(0., 0., 1920., 1080.), 2.),
            output("B", monitor(1920., 0., 1080., 1920.), 1.),
        ];
        let spanned = retained(&outputs, WallpaperLayout::Span);
        assert!(close(spanned.aspect, 3000. / 1920.));
        assert_eq!(spanned.edge, 3840, "the 4K monitor's own pixels, capped");
        let filled = retained(&outputs, WallpaperLayout::Fill);
        assert!(close(filled.aspect, 1920. / 1080.), "the first monitor's own shape");
        assert!(close(filled.height, 1.));
        // One ordinary monitor: no larger than it needs to be, never smaller than the floor.
        let small = retained(&[output("A", monitor(0., 0., 1366., 768.), 1.)], WallpaperLayout::Span);
        assert_eq!(small.edge, 1920);
        assert_eq!(retained(&[], WallpaperLayout::Span), Retained { aspect: 16. / 9., height: 1., frame: None, edge: 1920 });
    }

    #[test]
    fn a_span_over_monitors_of_several_heights_is_taller_than_one_unit_and_never_scales_the_scene() {
        // A landscape monitor beside a portrait one: the canvas is as tall as the portrait monitor, which
        // is 1920/1080 of the shortest, so the scene is not enlarged to fit it.
        let monitors = two_monitors();
        assert!(close(canvas_height(&monitors).unwrap(), 1920. / 1080.));
        let (_, spanned_placements) = placements(&monitors, WallpaperLayout::Span).unwrap();
        for placement in &spanned_placements {
            assert!(close(placement.height, 1920. / 1080.));
        }
        // Alike monitors need no extra room, and each monitor of a fill is one unit tall.
        let alike = [monitor(0., 0., 1920., 1080.), monitor(1920., 0., 1920., 1080.)];
        assert!(close(canvas_height(&alike).unwrap(), 1.));
        let (_, filled) = placements(&monitors, WallpaperLayout::Fill).unwrap();
        assert!(filled.iter().all(|placement| close(placement.height, 1.)));
        // The shortest monitor shows the scene at one monitor's own scale: its region is one unit of the
        // canvas tall, which is its height over the canvas's.
        assert!(close(spanned_placements[1].region.height * spanned_placements[1].height, 1920. / 1080.));
        assert!(close(spanned_placements[0].region.height * spanned_placements[0].height, 1.));
        assert!(canvas_height(&[]).is_none());
    }

    #[test]
    fn a_span_has_one_frame_on_the_topmost_shortest_row_so_the_picture_is_continuous() {
        // Jupiter-like: two landscape monitors stacked, a portrait one each side.
        let monitors = [
            monitor(0., 291., 1080., 1920.),
            monitor(1080., -457., 1920., 1080.),
            monitor(3000., 291., 1080., 1920.),
            monitor(1080., 1071., 1920., 1080.),
        ];
        let (_, placed) = placements(&monitors, WallpaperLayout::Span).unwrap();
        // Every monitor shares the one frame: the upper landscape monitor, which is the canvas's top.
        let frame = placed[0].frame.unwrap();
        assert!(placed.iter().all(|placement| placement.frame == Some(frame)));
        // Across it is the landscape monitors; down, its horizon is halfway down the portrait ones, which
        // run from 748 to 2668 of the canvas's 2668 (in units of 1080): the middle is at 1708.
        let horizon = (291. + 960. + 457.) / 1080.;
        assert!(close(frame.left, 1.) && close(frame.width, 1920. / 1080.));
        assert!(close(frame.top, horizon - crate::procedural::HORIZON), "{}", frame.top);
        // A fill has no frame: each monitor's picture is composed for itself.
        let (_, filled) = placements(&monitors, WallpaperLayout::Fill).unwrap();
        assert!(filled.iter().all(|placement| placement.frame.is_none()));
    }

    #[test]
    fn alike_monitors_frame_the_scene_as_one_monitor_does_and_a_taller_one_lowers_it() {
        let row = [monitor(0., 0., 1920., 1080.), monitor(1920., 0., 1920., 1080.)];
        let frame = frame_of(&row).unwrap();
        assert!(close(frame.left, 0.) && close(frame.top, 0.) && close(frame.width, 3840. / 1080.));
        // A tall monitor beside a short one: the horizon is halfway down the tall one, and the sky above
        // the scene is more of the same picture, not a larger one.
        let mixed = [monitor(0., 0., 1080., 1920.), monitor(1080., 0., 1920., 1080.)];
        let frame = frame_of(&mixed).unwrap();
        assert!(close(frame.left, 1.) && close(frame.width, 1920. / 1080.));
        assert!(close(frame.top, 960. / 1080. - crate::procedural::HORIZON));
        assert!(frame.top > 0.);
    }

    #[test]
    fn the_preview_draws_each_monitor_for_the_layout_at_the_previews_own_pixels() {
        use crate::procedural::{Palette, Recipe, Style, View};
        let recipe = Recipe::with_palette(Style::Aurora, 3, Palette::from_seed(3));
        let monitors = two_monitors();
        let spanned = preview_image(&recipe, &monitors, WallpaperLayout::Span, 300).unwrap();
        let filled = preview_image(&recipe, &monitors, WallpaperLayout::Fill, 300).unwrap();
        // The desktop's own shape, long side `edge`, either way.
        assert_eq!(spanned.dimensions(), filled.dimensions());
        assert_eq!(spanned.width(), 300);
        // Spanned, the miniature is the one canvas drawn at its own size, up to the rounding of where
        // each monitor's tile falls: shown here with no gap between monitors, since a gap is skipped.
        let side_by_side = [monitor(0., 0., 1920., 1080.), monitor(1920., 0., 1920., 1080.)];
        let spanned = preview_image(&recipe, &side_by_side, WallpaperLayout::Span, 300).unwrap();
        let whole_canvas = recipe.render(View::whole(3840. / 1080.), spanned.width(), spanned.height());
        let difference: f64 = spanned
            .pixels()
            .zip(whole_canvas.pixels())
            .map(|(a, b)| (0..3).map(|c| (f64::from(a[c]) - f64::from(b[c])).abs()).sum::<f64>() / 3.)
            .sum::<f64>()
            / f64::from(spanned.width() * spanned.height());
        assert!(difference < 6., "the spanned miniature is the whole canvas: {difference}");
        // Each monitor draws its own whole picture instead of a part of that one.
        assert_ne!(spanned, filled);
        let landscape = recipe.render(View::whole(1920. / 1080.), 192, 108);
        assert_eq!(*filled.get_pixel(0, 0), *landscape.get_pixel(0, 0), "the landscape monitor's own picture starts at its corner");
    }
}
