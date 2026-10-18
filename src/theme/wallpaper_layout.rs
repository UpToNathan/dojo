//! How one wallpaper is laid across the monitors.
//!
//! The choice lives in its own small file beside the theme store, not in a theme generation: it
//! describes this machine's monitors rather than a palette, so it survives theme changes and
//! applies the moment it is written.

use std::fs;
use std::path::Path;

use anyhow::Context as _;

use super::store::write_atomic;

pub const FILE_NAME: &str = "wallpaper-layout";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WallpaperLayout {
    /// Every monitor shows the whole image, cropped to fill it.
    #[default]
    Fill,
    /// One image covers the combined desktop; each monitor shows its own part of it.
    Span,
}

impl WallpaperLayout {
    pub fn name(self) -> &'static str {
        match self {
            Self::Fill => "fill",
            Self::Span => "span",
        }
    }

    fn parse(text: &str) -> anyhow::Result<Self> {
        match text.trim() {
            "" | "fill" => Ok(Self::Fill),
            "span" => Ok(Self::Span),
            other => anyhow::bail!("unknown wallpaper layout {other:?}; expected fill or span"),
        }
    }

    /// The saved layout; [`Fill`](Self::Fill) when none was ever chosen.
    pub fn read(root: &Path) -> anyhow::Result<Self> {
        let path = root.join(FILE_NAME);
        match fs::read_to_string(&path) {
            Ok(text) => Self::parse(&text).with_context(|| format!("reading {}", path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::Fill),
            Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn write(self, root: &Path) -> anyhow::Result<()> {
        write_atomic(
            &root.join(FILE_NAME),
            format!("{}\n", self.name()).as_bytes(),
        )
    }
}

/// A monitor's rectangle in the compositor's logical coordinates, which already account for
/// scale and rotation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Monitor {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// The part of an image one monitor shows, as fractions of the image's width and height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Region {
    pub left: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
}

impl Region {
    /// The same part of an image that is `width` by `height` pixels, as a pixel rectangle that
    /// stays inside it and is at least one pixel across: `(x, y, width, height)`.
    pub fn pixels(&self, width: u32, height: u32) -> (u32, u32, u32, u32) {
        let edge = |fraction: f64, whole: u32| {
            (fraction * f64::from(whole))
                .round()
                .clamp(0., f64::from(whole))
        };
        let left = edge(self.left, width).min(f64::from(width.saturating_sub(1)));
        let top = edge(self.top, height).min(f64::from(height.saturating_sub(1)));
        let right = edge(self.left + self.width, width)
            .max(left + 1.)
            .min(f64::from(width));
        let bottom = edge(self.top + self.height, height)
            .max(top + 1.)
            .min(f64::from(height));
        (
            left as u32,
            top as u32,
            (right - left) as u32,
            (bottom - top) as u32,
        )
    }
}

/// Lays one image over the bounding box of all the monitors, scaled to cover it and centred, and
/// returns what each monitor shows, in the order given. Monitors with gaps between them skip the
/// image behind the gap, as if looking through windows at one picture.
///
/// Returns `None` for an empty or degenerate arrangement or an empty image.
pub fn span_regions(monitors: &[Monitor], image: (u32, u32)) -> Option<Vec<Region>> {
    let (image_width, image_height) = (f64::from(image.0), f64::from(image.1));
    if monitors.is_empty()
        || image_width <= 0.
        || image_height <= 0.
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
    let (span_width, span_height) = (right - left, bottom - top);
    let scale = (span_width / image_width).max(span_height / image_height);
    let (shown_width, shown_height) = (image_width * scale, image_height * scale);
    let (offset_x, offset_y) = (
        (shown_width - span_width) / 2.,
        (shown_height - span_height) / 2.,
    );
    Some(
        monitors
            .iter()
            .map(|monitor| Region {
                left: (monitor.x - left + offset_x) / shown_width,
                top: (monitor.y - top + offset_y) / shown_height,
                width: monitor.width / shown_width,
                height: monitor.height / shown_height,
            })
            .collect(),
    )
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
    fn a_missing_file_means_fill_and_the_choice_round_trips() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        assert_eq!(
            WallpaperLayout::read(directory.path())?,
            WallpaperLayout::Fill
        );
        WallpaperLayout::Span.write(directory.path())?;
        assert_eq!(
            WallpaperLayout::read(directory.path())?,
            WallpaperLayout::Span
        );
        WallpaperLayout::Fill.write(directory.path())?;
        assert_eq!(
            WallpaperLayout::read(directory.path())?,
            WallpaperLayout::Fill
        );
        Ok(())
    }

    #[test]
    fn an_unknown_layout_is_an_error_not_a_silent_fill() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        fs::write(directory.path().join(FILE_NAME), "tile\n")?;
        assert!(WallpaperLayout::read(directory.path()).is_err());
        Ok(())
    }

    #[test]
    fn side_by_side_monitors_split_a_matching_image_in_half() {
        let regions = span_regions(
            &[
                monitor(0., 0., 1920., 1080.),
                monitor(1920., 0., 1920., 1080.),
            ],
            (3840, 1080),
        )
        .unwrap();
        assert_eq!(
            regions[0],
            Region {
                left: 0.,
                top: 0.,
                width: 0.5,
                height: 1.
            }
        );
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
    fn a_wide_image_is_cropped_evenly_at_the_sides() {
        // Twice as wide as the desktop needs: the middle half is shown.
        let regions = span_regions(&[monitor(0., 0., 1000., 1000.)], (2000, 1000)).unwrap();
        assert!(close(regions[0].left, 0.25));
        assert!(close(regions[0].width, 0.5));
        assert!(close(regions[0].top, 0.));
        assert!(close(regions[0].height, 1.));
    }

    #[test]
    fn a_tall_image_is_cropped_evenly_top_and_bottom() {
        let regions = span_regions(&[monitor(0., 0., 1000., 500.)], (1000, 1000)).unwrap();
        assert!(close(regions[0].top, 0.25));
        assert!(close(regions[0].height, 0.5));
        assert!(close(regions[0].left, 0.));
        assert!(close(regions[0].width, 1.));
    }

    #[test]
    fn monitors_at_negative_and_mixed_positions_share_one_picture() {
        // A portrait monitor left of a landscape one, tops aligned; the origin is not at zero.
        let monitors = [
            monitor(-1080., -200., 1080., 1920.),
            monitor(0., -200., 1920., 1080.),
        ];
        let regions = span_regions(&monitors, (3000, 1920)).unwrap();
        // Same total width as the image: the shared edge lines up exactly.
        assert!(close(regions[0].left + regions[0].width, regions[1].left));
        assert!(close(regions[0].top, regions[1].top));
        assert!(close(regions[0].left, 0.));
        assert!(close(regions[1].left + regions[1].width, 1.));
    }

    #[test]
    fn a_gap_between_monitors_skips_that_part_of_the_picture() {
        let regions = span_regions(
            &[monitor(0., 0., 100., 100.), monitor(200., 0., 100., 100.)],
            (300, 100),
        )
        .unwrap();
        assert!(close(regions[0].left + regions[0].width, 1. / 3.));
        assert!(close(regions[1].left, 2. / 3.));
    }

    #[test]
    fn a_scaled_monitor_takes_its_share_by_logical_size() {
        // 4K at 2x is 1920x1080 logical, the same share as an unscaled 1080p screen.
        let regions = span_regions(
            &[
                monitor(0., 0., 1920., 1080.),
                monitor(1920., 0., 1920., 1080.),
            ],
            (7680, 2160),
        )
        .unwrap();
        assert!(close(regions[0].width, regions[1].width));
    }

    #[test]
    fn degenerate_input_has_no_regions() {
        assert!(span_regions(&[], (100, 100)).is_none());
        assert!(span_regions(&[monitor(0., 0., 100., 100.)], (0, 100)).is_none());
        assert!(span_regions(&[monitor(0., 0., 0., 100.)], (100, 100)).is_none());
    }

    #[test]
    fn pixel_rectangles_stay_inside_the_image() {
        let region = Region {
            left: 0.5,
            top: 0.,
            width: 0.5,
            height: 1.,
        };
        assert_eq!(region.pixels(3840, 1080), (1920, 0, 1920, 1080));
        let sliver = Region {
            left: 1.,
            top: 1.,
            width: 0.,
            height: 0.,
        };
        let (x, y, width, height) = sliver.pixels(10, 10);
        assert!(
            x < 10 && y < 10 && width >= 1 && height >= 1 && x + width <= 10 && y + height <= 10
        );
    }
}
