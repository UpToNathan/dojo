//! The lock screen's pre-blurred, contrast-aware wallpaper.
//!
//! Rendered once when a theme generation is applied so the locker only decodes a
//! small PNG and never blurs on the GPU or CPU while locked.
//!
//! The scrim over the blurred picture is chosen from the text drawn on it, not
//! from a constant. The text is the palette's weakest foreground (the muted
//! colour used by the hint line, which sits straight on the wallpaper). The scrim
//! is the opposite tone of that text: black under light text, white under dark
//! text. Its strength is the smallest in `MIN_SCRIM..=MAX_SCRIM` for which the
//! text reaches `TARGET_CONTRAST` against the worst-case backdrop.
//!
//! The worst case is a percentile of the post-scrim luminance of the whole blurred
//! image, not the mean and not the hint's own region. One file serves every output
//! and aspect ratio, so the hint can land on any part of it. The 99.9th percentile
//! (the 0.1th under dark text) drops only the last 0.1% of pixels, about 130 of the
//! 480x270 render, which the blur has already diluted, so any patch big enough to
//! sit behind the hint line counts. The mean would let a bright patch behind the
//! text through, and so did a first attempt at the 99th percentile: it skipped the
//! core of a hint-sized bright blob.

use anyhow::Context as _;
use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat, RgbaImage};

use crate::extraction::Rgb;

/// Name of the artifact in each activation directory (`current/` next to `palette.json`).
pub const FILE_NAME: &str = "lock-wallpaper.png";
/// Output width in pixels. Smaller sources are not upscaled.
pub const WIDTH: u32 = 480;
/// Box-blur half-width in output pixels. Three passes approximate a Gaussian of
/// about `sqrt(3 * ((2r + 1)^2 - 1) / 12)` pixels, around 2% of the width.
pub const BLUR_RADIUS: usize = 10;
/// Contrast the text must reach. The hint is large text (3:1 would pass WCAG);
/// 4.5:1 clears the normal-text bar, and the extra 0.1 absorbs 8-bit rounding and
/// the texture filtering the locker applies when it scales the image.
pub const TARGET_CONTRAST: f64 = 4.6;
/// Weakest scrim. A wallpaper that already has the contrast keeps this much, so a
/// dark picture is not crushed and the lock screen still reads as dimmed.
pub const MIN_SCRIM: f32 = 0.30;
/// Strongest scrim. At least a tenth of the picture survives; where the target
/// needs more, the contrast falls short rather than the picture vanishing.
pub const MAX_SCRIM: f32 = 0.90;
/// Fraction of pixels whose post-scrim luminance must be at or beyond the worst
/// case: all but the brightest 0.1% under light text, all but the darkest 0.1%
/// under dark text.
pub const WORST_CASE_PERCENTILE: f64 = 0.999;
/// Version of this scrim rule, stored in the PNG. Bump it when the rule or its
/// constants change so already-applied generations are rendered again.
pub const RULE_VERSION: u32 = 2;

const MAX_DIMENSION: u32 = 16_384;
const MAX_ALLOC: u64 = 256 * 1024 * 1024;
const BINS: usize = 4096;
const KEYWORD: &str = "dojo-lock-wallpaper";
const SEARCH_STEPS: u32 = 10;

/// Which way the scrim pushes the picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    /// Black scrim under light text.
    Dark,
    /// White scrim under dark text.
    Light,
}

impl Tone {
    /// The tone that can give `text_luminance` the most contrast: black wins
    /// exactly when the text is brighter than the geometric midpoint of black and white.
    fn opposite_to(text_luminance: f64) -> Tone {
        if text_luminance > 0.179 {
            Tone::Dark
        } else {
            Tone::Light
        }
    }

    fn anchor(self) -> f32 {
        match self {
            Tone::Dark => 0.,
            Tone::Light => 255.,
        }
    }
}

/// The scrim chosen for one wallpaper and one text colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scrim {
    pub tone: Tone,
    /// Weight of the scrim colour in each pixel, `MIN_SCRIM..=MAX_SCRIM`.
    pub strength: f32,
    /// Contrast of the text against the worst-case backdrop at `strength`.
    /// Below `TARGET_CONTRAST` only when even `MAX_SCRIM` could not reach it.
    pub contrast: f64,
}

pub struct Rendered {
    pub image: RgbaImage,
    pub scrim: Scrim,
}

/// The downscaled, blurred wallpaper before any scrim, as 8-bit sRGB.
pub struct Blurred {
    width: u32,
    height: u32,
    rgb: Vec<u8>,
}

/// Downscales and blurs `source`. Transparent pixels are composited over black first.
pub fn blur(source: &DynamicImage) -> Blurred {
    if source.width() == 0 || source.height() == 0 {
        return Blurred {
            width: 0,
            height: 0,
            rgb: Vec::new(),
        };
    }
    let width = source.width().min(WIDTH);
    let height = (u64::from(source.height()) * u64::from(width))
        .div_ceil(u64::from(source.width()))
        .max(1) as u32;
    let small = source
        .resize_exact(width, height, FilterType::Triangle)
        .to_rgba8();
    let (w, h) = (width as usize, height as usize);
    let mut buffer: Vec<f32> = small
        .pixels()
        .flat_map(|p| {
            let alpha = f32::from(p.0[3]) / 255.;
            [p.0[0], p.0[1], p.0[2]].map(|c| f32::from(c) * alpha)
        })
        .collect();
    for _ in 0..3 {
        box_pass(&mut buffer, w, h, BLUR_RADIUS, true);
        box_pass(&mut buffer, w, h, BLUR_RADIUS, false);
    }
    Blurred {
        width,
        height,
        rgb: buffer.iter().map(|c| c.round() as u8).collect(),
    }
}

impl Blurred {
    /// The picture under the weakest scrim that gives `text` the target contrast.
    /// The result is fully opaque.
    pub fn scrimmed(&self, text: Rgb) -> Rendered {
        let tone = Tone::opposite_to(luminance_of(text));
        let search = Search::new(self, text, tone);
        let (mut strength, mut contrast) = (MIN_SCRIM, search.contrast(MIN_SCRIM));
        if contrast < TARGET_CONTRAST {
            (strength, contrast) = (MAX_SCRIM, search.contrast(MAX_SCRIM));
            if contrast >= TARGET_CONTRAST {
                let (mut fails, mut passes) = (MIN_SCRIM, MAX_SCRIM);
                for _ in 0..SEARCH_STEPS {
                    let middle = (fails + passes) / 2.;
                    let measured = search.contrast(middle);
                    if measured >= TARGET_CONTRAST {
                        (passes, contrast) = (middle, measured);
                    } else {
                        fails = middle;
                    }
                }
                strength = passes;
            }
        }
        Rendered {
            image: self.composite(tone, strength),
            scrim: Scrim {
                tone,
                strength,
                contrast,
            },
        }
    }

    fn composite(&self, tone: Tone, strength: f32) -> RgbaImage {
        let table = blend_table(tone, strength);
        let mut output = RgbaImage::new(self.width, self.height);
        for (pixel, rgb) in output.pixels_mut().zip(self.rgb.chunks_exact(3)) {
            pixel.0 = [
                table[usize::from(rgb[0])],
                table[usize::from(rgb[1])],
                table[usize::from(rgb[2])],
                255,
            ];
        }
        output
    }
}

/// Each channel value after the scrim at `strength`: a straight blend in sRGB
/// toward black or white.
fn blend_table(tone: Tone, strength: f32) -> [u8; 256] {
    let (keep, anchor) = (1. - strength, tone.anchor() * strength);
    std::array::from_fn(|c| (c as f32 * keep + anchor).round() as u8)
}

/// Contrast of one text colour against the worst-case backdrop at any strength.
struct Search<'a> {
    picture: &'a Blurred,
    tone: Tone,
    text: f64,
    /// Linear light of an 8-bit channel, premultiplied by its luminance weight.
    weighted: [[f32; 256]; 3],
}

impl<'a> Search<'a> {
    fn new(picture: &'a Blurred, text: Rgb, tone: Tone) -> Self {
        let weighted = [0.2126, 0.7152, 0.0722]
            .map(|weight| std::array::from_fn(|c| (weight * linearize(c as f64 / 255.)) as f32));
        Self {
            picture,
            tone,
            text: luminance_of(text),
            weighted,
        }
    }

    fn contrast(&self, strength: f32) -> f64 {
        let backdrop = self.worst_luminance(strength);
        match self.tone {
            Tone::Dark => (self.text + 0.05) / (backdrop + 0.05),
            Tone::Light => (backdrop + 0.05) / (self.text + 0.05),
        }
    }

    /// The post-scrim luminance that `WORST_CASE_PERCENTILE` of the picture does not
    /// get past in the harmful direction, rounded outward to a histogram bin edge.
    fn worst_luminance(&self, strength: f32) -> f64 {
        let pixels = self.picture.rgb.len() / 3;
        if pixels == 0 {
            return 0.;
        }
        let table = blend_table(self.tone, strength);
        let [red, green, blue] = &self.weighted;
        let mut histogram = [0u32; BINS];
        for rgb in self.picture.rgb.chunks_exact(3) {
            let luminance = red[usize::from(table[usize::from(rgb[0])])]
                + green[usize::from(table[usize::from(rgb[1])])]
                + blue[usize::from(table[usize::from(rgb[2])])];
            histogram[((luminance * (BINS - 1) as f32) as usize).min(BINS - 1)] += 1;
        }
        let beyond = ((1. - WORST_CASE_PERCENTILE) * pixels as f64).floor() as u32;
        let mut seen = 0;
        match self.tone {
            // Skip the brightest `beyond` pixels; the next one is the worst case.
            Tone::Dark => {
                for (bin, count) in histogram.iter().enumerate().rev() {
                    seen += count;
                    if seen > beyond {
                        return (bin + 1) as f64 / (BINS - 1) as f64;
                    }
                }
                1.
            }
            Tone::Light => {
                for (bin, count) in histogram.iter().enumerate() {
                    seen += count;
                    if seen > beyond {
                        return bin as f64 / (BINS - 1) as f64;
                    }
                }
                0.
            }
        }
    }
}

fn linearize(channel: f64) -> f64 {
    if channel <= 0.04045 {
        channel / 12.92
    } else {
        ((channel + 0.055) / 1.055).powf(2.4)
    }
}

fn luminance_of(color: Rgb) -> f64 {
    0.2126 * linearize(f64::from(color.0) / 255.)
        + 0.7152 * linearize(f64::from(color.1) / 255.)
        + 0.0722 * linearize(f64::from(color.2) / 255.)
}

/// Decodes a retained wallpaper snapshot within the size limits.
pub fn decode(wallpaper: &[u8]) -> anyhow::Result<DynamicImage> {
    let mut reader =
        image::ImageReader::new(std::io::Cursor::new(wallpaper)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(MAX_ALLOC);
    reader.limits(limits);
    reader.decode().context("decoding wallpaper")
}

/// Encodes `rendered` as a PNG whose first chunk after the header records the rule.
pub fn encode(rendered: &Rendered) -> anyhow::Result<Vec<u8>> {
    let mut png = std::io::Cursor::new(Vec::new());
    rendered
        .image
        .write_to(&mut png, ImageFormat::Png)
        .context("encoding lock wallpaper")?;
    let mut png = png.into_inner();
    let scrim = &rendered.scrim;
    let text = format!(
        "{KEYWORD}\0rule={RULE_VERSION};scrim={:?}:{:.3};contrast={:.2}",
        scrim.tone, scrim.strength, scrim.contrast
    );
    let chunk = png_chunk(b"tEXt", text.as_bytes());
    anyhow::ensure!(
        png.len() >= HEADER_END && png.starts_with(&PNG_SIGNATURE),
        "the PNG encoder produced no header"
    );
    png.splice(HEADER_END..HEADER_END, chunk);
    Ok(png)
}

/// Decodes a retained wallpaper snapshot, renders it for `text` and encodes the PNG.
pub fn render_png(wallpaper: &[u8], text: Rgb) -> anyhow::Result<Vec<u8>> {
    encode(&blur(&decode(wallpaper)?).scrimmed(text))
}

const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
/// Signature, then the 13-byte IHDR chunk with its length, type and CRC.
const HEADER_END: usize = 8 + 4 + 4 + 13 + 4;
/// Bytes of a file that `rule_of` needs.
pub const MARKER_PREFIX: usize = 128;

fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut chunk = Vec::with_capacity(data.len() + 12);
    chunk.extend_from_slice(&(data.len() as u32).to_be_bytes());
    chunk.extend_from_slice(kind);
    chunk.extend_from_slice(data);
    let mut crc = !0u32;
    for byte in kind.iter().chain(data) {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & (crc & 1).wrapping_neg());
        }
    }
    chunk.extend_from_slice(&(!crc).to_be_bytes());
    chunk
}

/// The rule version recorded in the first bytes of a lock wallpaper PNG, if any.
/// A file from before the rule existed has no marker and reads as `None`.
pub fn rule_of(prefix: &[u8]) -> Option<u32> {
    if !prefix.starts_with(&PNG_SIGNATURE) || prefix.len() < HEADER_END + 8 {
        return None;
    }
    let chunk = &prefix[HEADER_END..];
    let length = u32::from_be_bytes(chunk[..4].try_into().ok()?) as usize;
    if &chunk[4..8] != b"tEXt" {
        return None;
    }
    let data = chunk.get(8..8 + length.min(chunk.len() - 8))?;
    let text = std::str::from_utf8(data).ok()?;
    let value = text.strip_prefix(KEYWORD)?.strip_prefix('\0')?;
    value.strip_prefix("rule=")?.split(';').next()?.parse().ok()
}

/// Whether the file at `path` was rendered by the current rule.
pub fn is_current(path: &std::path::Path) -> bool {
    use std::io::Read as _;
    let mut prefix = Vec::with_capacity(MARKER_PREFIX);
    std::fs::File::open(path)
        .and_then(|file| file.take(MARKER_PREFIX as u64).read_to_end(&mut prefix))
        .is_ok()
        && rule_of(&prefix) == Some(RULE_VERSION)
}

/// One separable box-blur pass over an interleaved RGB buffer with clamped edges.
fn box_pass(buffer: &mut [f32], width: usize, height: usize, radius: usize, horizontal: bool) {
    let (lines, length, line_stride, step) = if horizontal {
        (height, width, width * 3, 3)
    } else {
        (width, height, 3, width * 3)
    };
    let window = (2 * radius + 1) as f32;
    let mut line = vec![0.; length];
    for index in 0..lines {
        for channel in 0..3 {
            let start = index * line_stride + channel;
            for (i, value) in line.iter_mut().enumerate() {
                *value = buffer[start + i * step];
            }
            let at = |i: isize| line[i.clamp(0, length as isize - 1) as usize];
            let r = radius as isize;
            let mut sum: f32 = (-r..=r).map(at).sum();
            for i in 0..length {
                buffer[start + i * step] = sum / window;
                sum += at(i as isize + r + 1) - at(i as isize - r);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    /// The real muted text of a dark generation's palette, and of a light one.
    const LIGHT_TEXT: Rgb = Rgb(0xa9, 0xb1, 0xd6);
    const DARK_TEXT: Rgb = Rgb(0x59, 0x63, 0x66);

    fn solid(width: u32, height: u32, value: u8) -> DynamicImage {
        DynamicImage::ImageRgba8(RgbaImage::from_pixel(
            width,
            height,
            Rgba([value, value, value, 255]),
        ))
    }

    fn gradient(width: u32, height: u32) -> DynamicImage {
        DynamicImage::ImageRgba8(RgbaImage::from_fn(width, height, |x, y| {
            Rgba([
                (x * 255 / (width - 1)) as u8,
                (y * 255 / (height - 1)) as u8,
                128,
                255,
            ])
        }))
    }

    fn checkerboard(width: u32, height: u32, square: u32) -> DynamicImage {
        DynamicImage::ImageRgba8(RgbaImage::from_fn(width, height, |x, y| {
            if (x / square + y / square) % 2 == 0 {
                Rgba([255, 255, 255, 255])
            } else {
                Rgba([0, 0, 0, 255])
            }
        }))
    }

    /// A smooth backdrop from `top` to `bottom` with a soft-edged `blob` of
    /// the size of the hint line's neighbourhood, centred where the hint sits.
    fn photo_with_blob(top: u8, bottom: u8, blob: [u8; 3]) -> DynamicImage {
        let (width, height) = (960u32, 540u32);
        DynamicImage::ImageRgba8(RgbaImage::from_fn(width, height, |x, y| {
            let t = y as f32 / (height - 1) as f32;
            let base = f32::from(top) * (1. - t) + f32::from(bottom) * t;
            let (dx, dy) = (
                (x as f32 - 480.) / 140.,
                (y as f32 - 0.697 * height as f32) / 50.,
            );
            let weight = (1. - (dx * dx + dy * dy).min(1.)).powf(0.5);
            let channel = |c: u8| (base * (1. - weight) + f32::from(c) * weight).round() as u8;
            Rgba([channel(blob[0]), channel(blob[1]), channel(blob[2]), 255])
        }))
    }

    /// WCAG 2.x relative luminance, written out here and not borrowed from the module.
    fn wcag_luminance(pixel: [u8; 3]) -> f64 {
        let [r, g, b] = pixel.map(|c| {
            let c = f64::from(c) / 255.;
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        });
        0.2126 * r + 0.7152 * g + 0.0722 * b
    }

    fn wcag_contrast(a: f64, b: f64) -> f64 {
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    fn text_luminance(text: Rgb) -> f64 {
        wcag_luminance([text.0, text.1, text.2])
    }

    fn pixel_luminance(pixel: &Rgba<u8>) -> f64 {
        wcag_luminance([pixel.0[0], pixel.0[1], pixel.0[2]])
    }

    /// Contrast of `text` against the 99.9th-percentile backdrop that hurts it:
    /// the brightest 0.1% under light text, the darkest 0.1% under dark text.
    fn percentile_contrast(image: &RgbaImage, text: Rgb) -> f64 {
        let text = text_luminance(text);
        let mut all: Vec<f64> = image.pixels().map(pixel_luminance).collect();
        all.sort_by(f64::total_cmp);
        let index = (0.999 * all.len() as f64).ceil() as usize - 1;
        let backdrop = if text > 0.179 {
            all[index]
        } else {
            all[all.len() - 1 - index]
        };
        wcag_contrast(text, backdrop)
    }

    /// Contrast of `text` against the worst pixel in the box the hint line occupies
    /// on a 16:9 screen: 30% to 70% across, 62% to 76% down.
    fn hint_zone_contrast(image: &RgbaImage, text: Rgb) -> f64 {
        let text = text_luminance(text);
        let (w, h) = (image.width() as f64, image.height() as f64);
        let mut worst: Option<f64> = None;
        for (x, y, pixel) in image.enumerate_pixels() {
            let (fx, fy) = (f64::from(x) / w, f64::from(y) / h);
            if !(0.30..=0.70).contains(&fx) || !(0.62..=0.76).contains(&fy) {
                continue;
            }
            let contrast = wcag_contrast(text, pixel_luminance(pixel));
            worst = Some(worst.map_or(contrast, |w: f64| w.min(contrast)));
        }
        worst.expect("the zone has pixels")
    }

    fn wallpapers() -> Vec<(&'static str, DynamicImage)> {
        vec![
            ("near-white", solid(960, 540, 250)),
            ("near-black", solid(960, 540, 12)),
            ("mid-grey", solid(960, 540, 128)),
            ("checkerboard", checkerboard(960, 540, 120)),
            ("gradient", gradient(960, 540)),
            (
                "bright blob on a dark photo",
                photo_with_blob(70, 25, [255, 250, 235]),
            ),
            (
                "dark blob on a bright photo",
                photo_with_blob(240, 200, [8, 10, 14]),
            ),
        ]
    }

    #[test]
    fn the_text_reaches_the_target_on_every_wallpaper_under_both_tones() {
        for (name, source) in wallpapers() {
            let picture = blur(&source);
            for text in [LIGHT_TEXT, DARK_TEXT] {
                let rendered = picture.scrimmed(text);
                let contrast = percentile_contrast(&rendered.image, text);
                assert!(
                    contrast >= 4.5,
                    "{name} under {text:?}: {contrast:.2}:1 at strength {}",
                    rendered.scrim.strength
                );
                assert!(
                    (MIN_SCRIM..=MAX_SCRIM).contains(&rendered.scrim.strength),
                    "{name}: strength {}",
                    rendered.scrim.strength
                );
                let expected = if text == LIGHT_TEXT {
                    Tone::Dark
                } else {
                    Tone::Light
                };
                assert_eq!(rendered.scrim.tone, expected, "{name}");
                assert!(rendered.scrim.contrast >= TARGET_CONTRAST, "{name}");
            }
        }
    }

    #[test]
    fn a_bright_blob_behind_the_hint_is_covered_even_though_the_mean_is_dark() {
        let source = photo_with_blob(70, 25, [255, 250, 235]);
        let rendered = blur(&source).scrimmed(LIGHT_TEXT);
        assert!(rendered.scrim.strength > 0.5, "{:?}", rendered.scrim);
        let contrast = hint_zone_contrast(&rendered.image, LIGHT_TEXT);
        assert!(contrast >= 4.5, "hint zone {contrast:.2}:1");
        let source = photo_with_blob(240, 200, [8, 10, 14]);
        let rendered = blur(&source).scrimmed(DARK_TEXT);
        let contrast = hint_zone_contrast(&rendered.image, DARK_TEXT);
        assert!(contrast >= 4.5, "dark blob, hint zone {contrast:.2}:1");
    }

    #[test]
    fn a_wallpaper_that_already_has_contrast_keeps_the_minimum_scrim() {
        let rendered = blur(&solid(960, 540, 12)).scrimmed(LIGHT_TEXT);
        assert_eq!(rendered.scrim.strength, MIN_SCRIM);
        assert_eq!(rendered.scrim.tone, Tone::Dark);
        let kept = rendered.image.get_pixel(240, 135).0;
        assert_eq!(kept, [8, 8, 8, 255], "12 * (1 - 0.30) rounds to 8");
        let rendered = blur(&solid(960, 540, 250)).scrimmed(DARK_TEXT);
        assert_eq!(rendered.scrim.tone, Tone::Light);
        assert_eq!(rendered.scrim.strength, MIN_SCRIM, "dark text on white");
        let rendered = blur(&solid(960, 540, 12)).scrimmed(DARK_TEXT);
        assert!(rendered.scrim.strength > MIN_SCRIM, "dark text on black");
    }

    #[test]
    fn the_scrim_is_the_weakest_that_passes() {
        let picture = blur(&solid(960, 540, 128));
        let rendered = picture.scrimmed(LIGHT_TEXT);
        let search = Search::new(&picture, LIGHT_TEXT, Tone::Dark);
        let strength = rendered.scrim.strength;
        assert!(strength > MIN_SCRIM && strength < MAX_SCRIM);
        assert!(search.contrast(strength) >= TARGET_CONTRAST);
        assert!(search.contrast(strength - 0.01) < TARGET_CONTRAST);
        // A darker picture needs no more than a brighter one.
        let darker = blur(&solid(960, 540, 90)).scrimmed(LIGHT_TEXT);
        assert!(darker.scrim.strength <= strength);
    }

    #[test]
    fn a_target_beyond_the_strongest_scrim_is_reported_not_hidden() {
        // This muted green-grey needs a backdrop darker than the strongest scrim
        // leaves of a white picture.
        let weak = Rgb(0x73, 0x81, 0x7d);
        let rendered = blur(&solid(960, 540, 255)).scrimmed(weak);
        assert_eq!(rendered.scrim.strength, MAX_SCRIM);
        assert!(rendered.scrim.contrast < TARGET_CONTRAST);
        let measured = percentile_contrast(&rendered.image, weak);
        assert!(
            (measured - rendered.scrim.contrast).abs() < 0.15,
            "reported {:.2}, measured {measured:.2}",
            rendered.scrim.contrast
        );
    }

    #[test]
    fn a_text_colour_near_the_middle_picks_the_tone_with_more_headroom() {
        assert_eq!(Tone::opposite_to(0.178), Tone::Light);
        assert_eq!(Tone::opposite_to(0.18), Tone::Dark);
        assert_eq!(
            Tone::opposite_to(text_luminance(Rgb(255, 255, 255))),
            Tone::Dark
        );
        assert_eq!(Tone::opposite_to(text_luminance(Rgb(0, 0, 0))), Tone::Light);
    }

    #[test]
    fn output_is_480_wide_and_keeps_the_aspect_ratio() {
        for (w, h, expected_h) in [
            (1920, 1080, 270),
            (3840, 2160, 270),
            (1200, 1600, 640),
            (960, 540, 270),
        ] {
            let out = blur(&gradient(w, h)).scrimmed(LIGHT_TEXT).image;
            assert_eq!((out.width(), out.height()), (480, expected_h), "{w}x{h}");
        }
        let small = blur(&gradient(240, 100)).scrimmed(LIGHT_TEXT).image;
        assert_eq!((small.width(), small.height()), (240, 100), "no upscaling");
        let strip = blur(&gradient(4800, 2)).scrimmed(LIGHT_TEXT).image;
        assert_eq!((strip.width(), strip.height()), (480, 1));
    }

    #[test]
    fn a_hard_edge_becomes_a_smooth_ramp() {
        let mut edge = RgbaImage::from_pixel(960, 540, Rgba([0, 0, 0, 255]));
        for y in 0..540 {
            for x in 480..960 {
                edge.put_pixel(x, y, Rgba([255, 255, 255, 255]));
            }
        }
        let out = blur(&DynamicImage::ImageRgba8(edge)).composite(Tone::Dark, 0.45);
        let row: Vec<u8> = (0..out.width())
            .map(|x| out.get_pixel(x, 135).0[0])
            .collect();
        let largest_step = row.windows(2).map(|w| w[1].abs_diff(w[0])).max().unwrap();
        // The unblurred step would be 255 * (1 - 0.45) = 140 in one pixel.
        assert!(largest_step <= 12, "largest step {largest_step}");
        let ramp = row.iter().filter(|&&v| v > 5 && v < 135).count();
        assert!(ramp >= 20, "ramp is only {ramp} pixels wide");
        assert!(
            row.windows(2).all(|w| w[1] >= w[0]),
            "ramp must be monotonic"
        );
        assert_eq!(row[0], 0);
        assert_eq!(row[row.len() - 1], 140);
    }

    #[test]
    fn a_fine_checkerboard_flattens_to_its_scrimmed_mean() {
        let picture = blur(&checkerboard(960, 540, 8));
        for (tone, expected) in [(Tone::Dark, 127.5 * 0.7), (Tone::Light, 127.5 * 0.7 + 76.5)] {
            let out = picture.composite(tone, 0.30);
            // Edge clamping reaches 3 * BLUR_RADIUS pixels in, so judge the interior.
            let margin = (3 * BLUR_RADIUS) as u32;
            for (x, y, pixel) in out.enumerate_pixels() {
                if x < margin
                    || y < margin
                    || x + margin >= out.width()
                    || y + margin >= out.height()
                {
                    continue;
                }
                assert!(
                    (f64::from(pixel.0[0]) - expected).abs() <= 6.,
                    "{tone:?} ({x},{y}) is {}, not within 6 of {expected}",
                    pixel.0[0]
                );
            }
        }
    }

    #[test]
    fn output_is_opaque_and_transparency_composites_over_black() {
        let clear =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(64, 64, Rgba([255, 255, 255, 0])));
        let out = blur(&clear).scrimmed(LIGHT_TEXT).image;
        assert!(out.pixels().all(|p| p.0 == [0, 0, 0, 255]));
        let out = blur(&gradient(300, 200)).scrimmed(DARK_TEXT).image;
        assert!(out.pixels().all(|p| p.0[3] == 255));
    }

    #[test]
    fn an_empty_image_renders_nothing() {
        let rendered = blur(&DynamicImage::ImageRgba8(RgbaImage::new(0, 0))).scrimmed(LIGHT_TEXT);
        assert_eq!((rendered.image.width(), rendered.image.height()), (0, 0));
        assert_eq!(rendered.scrim.strength, MIN_SCRIM);
    }

    #[test]
    fn rendering_is_deterministic_and_the_png_carries_the_rule() -> anyhow::Result<()> {
        let source = checkerboard(800, 600, 50);
        let mut png = std::io::Cursor::new(Vec::new());
        source.write_to(&mut png, ImageFormat::Png)?;
        let first = render_png(png.get_ref(), LIGHT_TEXT)?;
        assert_eq!(first, render_png(png.get_ref(), LIGHT_TEXT)?);
        assert_ne!(first, render_png(png.get_ref(), DARK_TEXT)?);
        let decoded = image::load_from_memory(&first)?;
        assert_eq!((decoded.width(), decoded.height()), (480, 360));
        assert_eq!(rule_of(&first[..MARKER_PREFIX]), Some(RULE_VERSION));
        let text = String::from_utf8_lossy(&first[HEADER_END..MARKER_PREFIX]).into_owned();
        assert!(
            text.contains("scrim=Dark:") && text.contains("contrast=4."),
            "{text}"
        );
        Ok(())
    }

    #[test]
    fn a_file_without_the_current_marker_is_not_current() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let mut plain = std::io::Cursor::new(Vec::new());
        solid(8, 8, 9).write_to(&mut plain, ImageFormat::Png)?;
        let old = directory.path().join("old.png");
        std::fs::write(&old, plain.get_ref())?;
        assert_eq!(rule_of(plain.get_ref()), None);
        assert!(!is_current(&old));
        let fresh = directory.path().join("fresh.png");
        std::fs::write(&fresh, render_png(plain.get_ref(), LIGHT_TEXT)?)?;
        assert!(is_current(&fresh));
        let mut bytes = std::fs::read(&fresh)?;
        let at = bytes.windows(6).position(|w| w == b"rule=2").unwrap();
        bytes[at + 5] = b'1';
        std::fs::write(&fresh, &bytes)?;
        assert_eq!(rule_of(&bytes[..MARKER_PREFIX]), Some(1));
        assert!(!is_current(&fresh));
        assert!(!is_current(&directory.path().join("missing.png")));
        assert_eq!(rule_of(b"not a png at all, but long enough to be inspected........................................................"), None);
        Ok(())
    }

    #[test]
    fn undecodable_bytes_are_an_error() {
        assert!(render_png(b"not an image", LIGHT_TEXT).is_err());
    }

    /// `cargo test --release --lib theme::lock_wallpaper::tests::measure -- --ignored --nocapture`
    #[test]
    #[ignore = "timing probe, not a correctness test"]
    fn measure_render_time_and_size() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let noisy = |w: u32, h: u32| {
            RgbaImage::from_fn(w, h, |x, y| {
                let n = x.wrapping_mul(2654435761) ^ y.wrapping_mul(40503);
                Rgba([
                    ((x * 255 / w) as u8).wrapping_add((n & 15) as u8),
                    ((y * 255 / h) as u8).wrapping_add(((n >> 4) & 15) as u8),
                    ((x + y) * 255 / (w + h)) as u8,
                    255,
                ])
            })
        };
        let jpeg = directory.path().join("wall.jpg");
        DynamicImage::ImageRgba8(noisy(3840, 2160))
            .to_rgb8()
            .save(&jpeg)?;
        let png = directory.path().join("wall.png");
        DynamicImage::ImageRgba8(noisy(1920, 1080)).save(&png)?;
        for path in [&jpeg, &png] {
            let bytes = std::fs::read(path)?;
            let start = std::time::Instant::now();
            let out = render_png(&bytes, LIGHT_TEXT)?;
            let one = start.elapsed();
            let start = std::time::Instant::now();
            let source = decode(&bytes)?;
            let decoded = start.elapsed();
            let start = std::time::Instant::now();
            let picture = blur(&source);
            let blurred = start.elapsed();
            let start = std::time::Instant::now();
            let rendered = picture.scrimmed(LIGHT_TEXT);
            let scrimmed = start.elapsed();
            let start = std::time::Instant::now();
            let _ = encode(&rendered)?;
            let encoded = start.elapsed();
            let start = std::time::Instant::now();
            let _ = picture.scrimmed(DARK_TEXT);
            let second = start.elapsed();
            println!(
                "{}: input {} bytes, one render {one:?} (decode {decoded:?}, resize+blur {blurred:?}, scrim search+composite {scrimmed:?}, encode {encoded:?}), a second palette adds {second:?}+encode, output {} bytes, scrim {:?}",
                path.display(),
                bytes.len(),
                out.len(),
                rendered.scrim
            );
        }
        Ok(())
    }

    /// Writes the before (constant 45% black dim, the old rule) and after renders
    /// of every image in `DOJO_S3_IN` for both palettes' muted text into `DOJO_S3_OUT`.
    #[test]
    #[ignore = "measurement fixture for .audit/s3, not a correctness test"]
    fn write_measurement_wallpapers() -> anyhow::Result<()> {
        let input = std::path::PathBuf::from(std::env::var("DOJO_S3_IN")?);
        let output = std::path::PathBuf::from(std::env::var("DOJO_S3_OUT")?);
        std::fs::create_dir_all(&output)?;
        for entry in std::fs::read_dir(input)? {
            let path = entry?.path();
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()).map(str::to_owned) else {
                continue;
            };
            let picture = blur(&decode(&std::fs::read(&path)?)?);
            picture
                .composite(Tone::Dark, 0.45)
                .save(output.join(format!("{stem}.before.png")))?;
            for (palette, text) in [("dark", LIGHT_TEXT), ("light", DARK_TEXT)] {
                let rendered = picture.scrimmed(text);
                println!("{stem} {palette}: {:?}", rendered.scrim);
                std::fs::write(
                    output.join(format!("{stem}.{palette}.after.png")),
                    encode(&rendered)?,
                )?;
            }
        }
        Ok(())
    }
}
