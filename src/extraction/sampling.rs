use std::path::Path;

use anyhow::Context as _;
use image::{ImageReader, RgbaImage};

use super::Rgb;

pub(super) fn load(path: &Path) -> anyhow::Result<Vec<Rgb>> {
    let mut reader = ImageReader::open(path)
        .with_context(|| format!("opening wallpaper {}", path.display()))?
        .with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    let image = reader
        .decode()
        .with_context(|| format!("decoding wallpaper {}", path.display()))?;
    anyhow::ensure!(
        u64::from(image.width()) * u64::from(image.height()) <= crate::theme::MAX_WALLPAPER_PIXELS,
        "wallpaper exceeds the 64 million pixel limit"
    );
    Ok(sample(&image.into_rgba8()))
}

pub(super) fn load_bytes(bytes: &[u8]) -> anyhow::Result<Vec<Rgb>> {
    let mut reader = ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode()?;
    anyhow::ensure!(
        u64::from(image.width()) * u64::from(image.height()) <= crate::theme::MAX_WALLPAPER_PIXELS,
        "wallpaper exceeds the 64 million pixel limit"
    );
    Ok(sample(&image.into_rgba8()))
}

fn kernel(distance: f64) -> f64 {
    if distance < 1. {
        (1.5 * distance - 2.5) * distance * distance + 1.
    } else {
        ((-0.5 * distance + 2.5) * distance - 4.) * distance + 2.
    }
}

struct Contributions {
    pixels: Vec<(u32, f64)>,
    inverse_weight: f64,
}

fn contributions(destination: u32, source: u32) -> Vec<Contributions> {
    let scale = source as f64 / destination as f64;
    let support = 2. * scale.max(1.);
    let argument_scale = 1. / scale.max(1.);
    (0..destination)
        .map(|index| {
            let center = (index as f64 + 0.5) * scale - 0.5;
            let start = (center - support).floor().max(0.) as u32;
            let end = (center + support).ceil().min(source as f64) as u32;
            let mut total = 0.;
            let mut pixels = Vec::new();
            for index in start..end {
                let distance = ((center - index as f64) * argument_scale).abs();
                if distance >= 2. {
                    continue;
                }
                let weight = kernel(distance);
                if weight == 0. {
                    continue;
                }
                total += weight;
                pixels.push((index, weight));
            }
            Contributions {
                pixels,
                inverse_weight: 1. / total,
            }
        })
        .collect()
}

// Match x/image/draw v0.41.0's kernel support, premultiplied-alpha arithmetic and
// 16-bit destination rounding. Generic Catmull-Rom resizers differ at these steps.
// See licenses/go-image-BSD.txt for the source attribution.
fn resize(image: &RgbaImage, width: u32, height: u32) -> RgbaImage {
    let horizontal = contributions(width, image.width());
    let vertical = contributions(height, image.height());
    let mut intermediate = vec![[0.; 4]; width as usize * image.height() as usize];
    for y in 0..image.height() {
        for (x, contribution) in horizontal.iter().enumerate() {
            let mut sum = [0.; 4];
            for (source_x, weight) in &contribution.pixels {
                let pixel = image.get_pixel(*source_x, y).0;
                let alpha = pixel[3] as u32 * 257;
                for channel in 0..3 {
                    sum[channel] += (pixel[channel] as u32 * alpha / 255) as f64 * weight;
                }
                sum[3] += alpha as f64 * weight;
            }
            intermediate[y as usize * width as usize + x] =
                sum.map(|v| v * (contribution.inverse_weight / 65535.));
        }
    }
    let mut result = RgbaImage::new(width, height);
    for x in 0..width {
        for (y, contribution) in vertical.iter().enumerate() {
            let mut sum = [0.; 4];
            for (source_y, weight) in &contribution.pixels {
                let pixel = intermediate[*source_y as usize * width as usize + x as usize];
                for channel in 0..4 {
                    sum[channel] += pixel[channel] * weight;
                }
            }
            let alpha =
                (sum[3] * contribution.inverse_weight * 65535. + 0.5).clamp(0., 65535.) as u32;
            let mut pixel = [0; 4];
            pixel[3] = (alpha >> 8) as u8;
            if alpha > 0 {
                for channel in 0..3 {
                    let premultiplied =
                        (sum[channel].min(sum[3]) * contribution.inverse_weight * 65535. + 0.5)
                            .clamp(0., 65535.) as u32;
                    pixel[channel] = ((premultiplied * 65535 / alpha) >> 8) as u8;
                }
            }
            result.put_pixel(x, y as u32, image::Rgba(pixel));
        }
    }
    result
}

fn sample(image: &RgbaImage) -> Vec<Rgb> {
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 {
        return Vec::new();
    }
    let (width, height) = if width >= height {
        (
            400,
            (height as f64 * 400. / width as f64).round().max(1.) as u32,
        )
    } else {
        (
            (width as f64 * 400. / height as f64).round().max(1.) as u32,
            400,
        )
    };
    let scaled = resize(image, width, height);
    let step = ((width * height) / 50_000).max(1) as usize;
    let mut colors = Vec::new();
    for y in (0..height).step_by(step) {
        for x in (0..width).step_by(step) {
            let pixel = scaled.get_pixel(x, y).0;
            if pixel[3] >= 128 {
                colors.push(Rgb(pixel[0], pixel[1], pixel[2]));
            }
        }
    }
    colors
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampling_and_alpha_handling_match_go() -> anyhow::Result<()> {
        let image =
            image::load_from_memory(include_bytes!("../../tests/fixtures/aether-sampling.png"))?
                .into_rgba8();
        let actual: Vec<u8> = sample(&image)
            .into_iter()
            .flat_map(|c| [c.0, c.1, c.2])
            .collect();
        let expected = include_bytes!("../../tests/fixtures/aether-sampling.rgb");
        assert_eq!(actual.len(), expected.len());
        let mismatches: Vec<_> = actual
            .iter()
            .zip(expected)
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .take(20)
            .collect();
        assert!(mismatches.is_empty(), "sample differences: {mismatches:?}");
        Ok(())
    }

    #[test]
    fn sampling_preserves_aspect_ratio_and_ignores_transparency() {
        let opaque = RgbaImage::from_pixel(512, 256, image::Rgba([20, 30, 40, 255]));
        assert_eq!(sample(&opaque).len(), 400 * 200);
        let transparent = RgbaImage::from_pixel(512, 256, image::Rgba([255, 0, 0, 0]));
        assert!(sample(&transparent).is_empty());
    }
}
