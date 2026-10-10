//! Wallpaper palette extraction ported from Aether, revision
//! 29d07b1a8f474c77bf02aece948373f40abed105. See licenses/aether-MIT.txt.

mod color;
mod palette;
mod quantize;
mod sampling;

use std::path::Path;
use std::str::FromStr;

use anyhow::{Context as _, bail};
use serde::{Deserialize, Serialize};

pub use color::Rgb;

pub const UPSTREAM_REVISION: &str = "29d07b1a8f474c77bf02aece948373f40abed105";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    Normal,
    Monochromatic,
    Analogous,
    Pastel,
    Material,
    Colorful,
    Muted,
    Bright,
    Complementary,
    Triadic,
    SplitComplementary,
    Tetradic,
    Fire,
    Ocean,
    Forest,
    Earthtone,
    Neon,
    Sunset,
    Vaporwave,
    Midnight,
    Aurora,
    HighContrast,
    Duotone,
}

impl Mode {
    pub const ALL: [Self; 23] = [
        Self::Normal,
        Self::Monochromatic,
        Self::Analogous,
        Self::Pastel,
        Self::Material,
        Self::Colorful,
        Self::Muted,
        Self::Bright,
        Self::Complementary,
        Self::Triadic,
        Self::SplitComplementary,
        Self::Tetradic,
        Self::Fire,
        Self::Ocean,
        Self::Forest,
        Self::Earthtone,
        Self::Neon,
        Self::Sunset,
        Self::Vaporwave,
        Self::Midnight,
        Self::Aurora,
        Self::HighContrast,
        Self::Duotone,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Monochromatic => "monochromatic",
            Self::Analogous => "analogous",
            Self::Pastel => "pastel",
            Self::Material => "material",
            Self::Colorful => "colorful",
            Self::Muted => "muted",
            Self::Bright => "bright",
            Self::Complementary => "complementary",
            Self::Triadic => "triadic",
            Self::SplitComplementary => "split-complementary",
            Self::Tetradic => "tetradic",
            Self::Fire => "fire",
            Self::Ocean => "ocean",
            Self::Forest => "forest",
            Self::Earthtone => "earthtone",
            Self::Neon => "neon",
            Self::Sunset => "sunset",
            Self::Vaporwave => "vaporwave",
            Self::Midnight => "midnight",
            Self::Aurora => "aurora",
            Self::HighContrast => "high-contrast",
            Self::Duotone => "duotone",
        }
    }
}

impl FromStr for Mode {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|mode| mode.name() == value)
            .with_context(|| format!("unknown extraction mode {value:?}"))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Palette {
    /// ANSI slots 0..15. Slot 0 is the background, 7 the foreground, 8 comments.
    pub colors: [Rgb; 16],
}

impl Palette {
    pub fn hex_colors(&self) -> [String; 16] {
        self.colors.map(Rgb::hex)
    }
}

/// Decode, sample, quantize and derive a palette without modifying desktop state.
pub fn extract(path: &Path, light: bool, mode: Mode) -> anyhow::Result<Palette> {
    let pixels = sampling::load(path)?;
    extract_pixels(&pixels, light, mode)
}

/// Derive both appearances from the same decoded and quantized image snapshot.
pub fn extract_pair(path: &Path, mode: Mode) -> anyhow::Result<(Palette, Palette)> {
    let pixels = sampling::load(path)?;
    anyhow::ensure!(
        pixels.len() >= 100,
        "not enough opaque pixels to extract colors"
    );
    let (colors, counts) = quantize::dominant(&pixels, 48);
    Ok((
        derive(&colors, &counts, false, mode)?,
        derive(&colors, &counts, true, mode)?,
    ))
}

pub fn extract_pair_bytes(bytes: &[u8], mode: Mode) -> anyhow::Result<(Palette, Palette)> {
    let pixels = sampling::load_bytes(bytes)?;
    anyhow::ensure!(
        pixels.len() >= 100,
        "not enough opaque pixels to extract colors"
    );
    let (colors, counts) = quantize::dominant(&pixels, 48);
    Ok((
        derive(&colors, &counts, false, mode)?,
        derive(&colors, &counts, true, mode)?,
    ))
}

/// Reuse one image analysis when comparing every mode without applying any theme.
pub fn extract_all_pairs(bytes: &[u8]) -> anyhow::Result<Vec<(Mode, Palette, Palette)>> {
    let pixels = sampling::load_bytes(bytes)?;
    anyhow::ensure!(
        pixels.len() >= 100,
        "not enough opaque pixels to extract colors"
    );
    let (colors, counts) = quantize::dominant(&pixels, 48);
    Mode::ALL
        .into_iter()
        .map(|mode| {
            Ok((
                mode,
                derive(&colors, &counts, false, mode)?,
                derive(&colors, &counts, true, mode)?,
            ))
        })
        .collect()
}

/// The same extraction pipeline for already-sampled, straight-alpha RGB pixels.
pub fn extract_pixels(pixels: &[Rgb], light: bool, mode: Mode) -> anyhow::Result<Palette> {
    if pixels.len() < 100 {
        bail!("not enough opaque pixels to extract colors (need at least 100)");
    }
    let (colors, counts) = quantize::dominant(pixels, 48);
    derive(&colors, &counts, light, mode)
}

fn derive(colors: &[Rgb], counts: &[usize], light: bool, mode: Mode) -> anyhow::Result<Palette> {
    if colors.len() < 8 || colors.len() != counts.len() {
        bail!("need at least eight aligned dominant colors and coverage counts");
    }
    let total: usize = counts.iter().sum();
    if total == 0 {
        bail!("dominant color coverage cannot be empty");
    }
    let weights: Vec<f64> = counts
        .iter()
        .map(|count| *count as f64 / total as f64)
        .collect();
    let mut colors = palette::generate(colors, &weights, light, mode);
    palette::normalize(&mut colors);
    Ok(Palette { colors })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize)]
    struct Input {
        name: String,
        colors: Vec<String>,
        counts: Vec<usize>,
    }
    #[derive(Deserialize)]
    struct Expected {
        mode: String,
        light: bool,
        colors: [String; 16],
    }
    #[derive(Deserialize)]
    struct Fixture {
        input: Input,
        expected: Vec<Expected>,
    }

    #[test]
    fn all_modes_match_the_pinned_go_palette_generators() -> anyhow::Result<()> {
        let fixtures: Vec<Fixture> =
            serde_json::from_str(include_str!("../tests/fixtures/aether-palettes.json"))?;
        let mut failures = Vec::new();
        for fixture in fixtures {
            assert_eq!(fixture.expected.len(), Mode::ALL.len() * 2);
            let colors: Vec<Rgb> = fixture
                .input
                .colors
                .iter()
                .map(|c| Rgb::parse(c))
                .collect::<anyhow::Result<_>>()?;
            for expected in fixture.expected {
                let palette = derive(
                    &colors,
                    &fixture.input.counts,
                    expected.light,
                    expected.mode.parse()?,
                )?;
                for (slot, (actual, hex)) in
                    palette.colors.into_iter().zip(expected.colors).enumerate()
                {
                    let upstream = Rgb::parse(&hex)?;
                    if actual != upstream {
                        failures.push(format!(
                            "{} {} light={} slot {slot}: {} != {}",
                            fixture.input.name,
                            expected.mode,
                            expected.light,
                            actual.hex(),
                            upstream.hex()
                        ));
                    }
                }
            }
        }
        assert!(
            failures.is_empty(),
            "{} mismatches:\n{}",
            failures.len(),
            failures.join("\n")
        );
        Ok(())
    }

    #[test]
    fn rejects_unknown_modes_and_insufficient_input() {
        assert!("not-a-mode".parse::<Mode>().is_err());
        assert!(extract_pixels(&[], false, Mode::Normal).is_err());
        assert!(extract_pixels(&[Rgb(30, 30, 30); 100], false, Mode::Normal).is_err());
    }

    #[test]
    fn quantization_matches_go_on_identical_wallpaper_samples() -> anyhow::Result<()> {
        let samples = include_bytes!("../tests/fixtures/astro-go-samples.rgb");
        let pixels: Vec<Rgb> = samples
            .chunks_exact(3)
            .map(|p| Rgb(p[0], p[1], p[2]))
            .collect();
        let (colors, counts) = quantize::dominant(&pixels, 48);
        let fixtures: Vec<Fixture> =
            serde_json::from_str(include_str!("../tests/fixtures/aether-palettes.json"))?;
        let upstream = fixtures
            .into_iter()
            .find(|f| f.input.name == "astro")
            .context("missing astro fixture")?;
        let mut actual: Vec<_> = colors
            .iter()
            .zip(&counts)
            .map(|(c, n)| (c.hex().to_uppercase(), *n))
            .collect();
        let mut expected: Vec<_> = upstream
            .input
            .colors
            .into_iter()
            .zip(upstream.input.counts)
            .collect();
        actual.sort();
        expected.sort();
        // Go uses an unstable count sort; ties can reorder identical-coverage buckets.
        assert_eq!(actual, expected);
        for expected in upstream.expected {
            let result = derive(&colors, &counts, expected.light, expected.mode.parse()?)?;
            assert_eq!(
                result.hex_colors().map(|s| s.to_uppercase()),
                expected.colors.map(|s| s.to_uppercase()),
                "{} light={}",
                expected.mode,
                expected.light
            );
        }
        Ok(())
    }

    #[test]
    fn current_wallpaper_generates_every_mode_in_both_appearances() -> anyhow::Result<()> {
        let pixels = sampling::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("astro.jpg"))?;
        let (colors, counts) = quantize::dominant(&pixels, 48);
        for mode in Mode::ALL {
            for light in [false, true] {
                let palette = derive(&colors, &counts, light, mode)?;
                assert!(palette.hex_colors().iter().all(|color| color.len() == 7));
                assert!(
                    palette.colors[0].contrast(palette.colors[7]) > 1.,
                    "{} light={light}",
                    mode.name()
                );
            }
        }
        Ok(())
    }
}
