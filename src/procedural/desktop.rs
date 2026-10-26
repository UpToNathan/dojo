//! Where each monitor sits on a procedural wallpaper's canvas.
//!
//! The canvas is the bounding box of the monitors in the compositor's logical coordinates, which
//! already account for scale and rotation. Spanned, each monitor samples the rectangle it
//! occupies, so a gap between monitors skips the picture behind it. Filled, each monitor samples
//! the largest centred part of the canvas that has its own shape.
//!
//! Unlike a photograph, nothing here decides how much to enlarge: the picture is drawn at each
//! monitor's own pixels, so the monitors' sizes only choose what is in view.

use anyhow::Context as _;

use crate::theme::wallpaper_layout::{Monitor, Region};

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

/// What a filled wallpaper shows on a monitor `width` by `height`: the largest part of a canvas
/// of shape `aspect` that has the monitor's shape, centred.
pub fn filled(aspect: f64, width: f64, height: f64) -> Region {
    let monitor = width / height;
    if !(monitor.is_finite() && aspect.is_finite() && monitor > 0. && aspect > 0.) {
        return whole();
    }
    if monitor >= aspect {
        let shown = aspect / monitor;
        Region {
            left: 0.,
            top: (1. - shown) / 2.,
            width: 1.,
            height: shown,
        }
    } else {
        let shown = monitor / aspect;
        Region {
            left: (1. - shown) / 2.,
            top: 0.,
            width: shown,
            height: 1.,
        }
    }
}

pub fn whole() -> Region {
    Region {
        left: 0.,
        top: 0.,
        width: 1.,
        height: 1.,
    }
}

/// The compositor's outputs with a logical position, by name.
pub fn niri_monitors() -> anyhow::Result<Vec<(String, Monitor)>> {
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
            Some((
                output.name,
                Monitor {
                    x: f64::from(logical.x),
                    y: f64::from(logical.y),
                    width: f64::from(logical.width),
                    height: f64::from(logical.height),
                },
            ))
        })
        .collect();
    monitors.sort_by(|a, b| a.0.cmp(&b.0));
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

    #[test]
    fn a_filled_monitor_takes_the_centred_part_with_its_own_shape() {
        // A wide canvas on a portrait monitor: a narrow centred slice, full height.
        let slice = filled(3., 1080., 1920.);
        assert!(close(slice.height, 1.) && close(slice.top, 0.));
        assert!(close(slice.width * 3. / 1., 1080. / 1920.));
        assert!(close(slice.left, (1. - slice.width) / 2.));
        // A tall canvas on a landscape monitor: a short centred band, full width.
        let band = filled(0.5, 1920., 1080.);
        assert!(close(band.width, 1.) && close(band.left, 0.));
        assert!(close(band.top, (1. - band.height) / 2.));
        // Its own shape: the whole canvas.
        assert_eq!(filled(1.6, 1600., 1000.), whole());
        // Nonsense falls back to everything rather than to nothing.
        assert_eq!(filled(f64::NAN, 1., 1.), whole());
    }
}
