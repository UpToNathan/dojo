//! Procedural wallpapers: a picture described by a recipe and drawn at whatever resolution is asked.
//!
//! A recipe is a style, a seed and a palette. The picture is a continuous function of position
//! over the whole desktop (the *canvas*, one unit tall however wide), so it is evaluated at each
//! monitor's own pixels and never resampled. That is what keeps it sharp on a rotated or scaled
//! monitor, and what makes it continuous across monitors when it spans them: a monitor only
//! chooses which part of the canvas it samples.
//!
//! The same recipe, view and size always give the same pixels, and a view that splits at a pixel
//! boundary gives the same pixels as the unsplit one.

use std::fmt;

use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::theme::wallpaper_layout::Region;

pub mod desktop;

/// The recipe format this build reads and writes.
pub const VERSION: u32 = 1;

/// Where a theme generation keeps its recipe, beside the retained `wallpaper` picture. The
/// wallpaper surface reads this one small file rather than the whole theme.
pub const FILE_NAME: &str = "wallpaper-recipe.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Style {
    /// Soft luminous bands drifting through a dark field.
    Aurora,
    /// Topographic contour lines.
    Contours,
    /// Layered mountain ridges fading into haze, under a low sun.
    Ridges,
}

impl Style {
    pub const ALL: [Style; 3] = [Style::Aurora, Style::Contours, Style::Ridges];

    pub fn name(self) -> &'static str {
        match self {
            Self::Aurora => "aurora",
            Self::Contours => "contours",
            Self::Ridges => "ridges",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Aurora => "Aurora",
            Self::Contours => "Contours",
            Self::Ridges => "Ridges",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|style| style.name() == text.trim())
    }
}

/// An sRGB colour, written as `#rrggbb`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub [u8; 3]);

impl fmt::Display for Rgb {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "#{:02x}{:02x}{:02x}",
            self.0[0], self.0[1], self.0[2]
        )
    }
}

impl std::str::FromStr for Rgb {
    type Err = anyhow::Error;

    fn from_str(text: &str) -> anyhow::Result<Self> {
        let digits = text.strip_prefix('#').context("a colour starts with #")?;
        anyhow::ensure!(
            digits.len() == 6 && digits.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "a colour is #rrggbb, not {text:?}"
        );
        let channel = |at: usize| u8::from_str_radix(&digits[at..at + 2], 16);
        Ok(Self([channel(0)?, channel(2)?, channel(4)?]))
    }
}

impl Serialize for Rgb {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Rgb {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// A dark background and three accents from dim to bright.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Palette {
    pub background: Rgb,
    pub accents: [Rgb; 3],
}

impl Palette {
    /// A dark, harmonious palette chosen by `seed`.
    pub fn from_seed(seed: u64) -> Self {
        let mut random = Random::new(seed ^ 0x70a1_e77e);
        let hue = random.unit() * 360.;
        let offsets: [f64; 3] = match random.below(3) {
            0 => [0., 28., 58.],
            1 => [0., 40., 170.],
            _ => [-30., 0., 45.],
        };
        let saturation = 0.5 + random.unit() * 0.25;
        let background = hsl(
            hue + random.unit() * 20. - 10.,
            0.32 + random.unit() * 0.12,
            0.065,
        );
        let lightness = [0.34, 0.5, 0.68];
        let accents = std::array::from_fn(|index| {
            hsl(
                hue + offsets[index],
                (saturation + 0.05 * index as f64).min(0.85),
                lightness[index],
            )
        });
        Self {
            background,
            accents,
        }
    }
}

impl Palette {
    /// Colours borrowed from a theme, so the wallpaper belongs with the rest of the desktop. The
    /// background is the theme's darker of background and foreground (a light theme's text), and
    /// the accents are its primary, secondary and info colours.
    pub fn from_theme(theme: &crate::theme::SemanticPalette) -> anyhow::Result<Self> {
        let colour = |role: &str| -> anyhow::Result<Rgb> {
            theme
                .color(role)
                .parse()
                .with_context(|| format!("the theme's {role} colour"))
        };
        let (background, foreground) = (colour("background")?, colour("foreground")?);
        let darker = if luminance(linear(background)) <= luminance(linear(foreground)) {
            background
        } else {
            foreground
        };
        let accents = ["primary", "secondary", "info"]
            .into_iter()
            .map(colour)
            .collect::<anyhow::Result<Vec<_>>>()?;
        Self::from_parts(darker, &accents)
    }

    /// A palette from colours the user chose: the darkest is the background, the rest are the
    /// accents. Needs at least two.
    pub fn from_colors(colors: &[Rgb]) -> anyhow::Result<Self> {
        let mut colors = colors.to_vec();
        colors.dedup();
        anyhow::ensure!(
            colors.len() >= 2,
            "a palette needs at least two colours, such as #0b0b14 #5b6ee1"
        );
        colors.sort_by(|a, b| luminance(linear(*a)).total_cmp(&luminance(linear(*b))));
        let background = colors.remove(0);
        Self::from_parts(background, &colors)
    }

    /// `accents` may be any number of colours: three are chosen from them, or made up from one or
    /// two, ordered from dim to bright. The result stays dark enough for a wallpaper: the
    /// background is darkened to a deep tone and the dimmest accent is held back, so a bright
    /// palette still gives a picture that sits behind windows.
    fn from_parts(background: Rgb, accents: &[Rgb]) -> anyhow::Result<Self> {
        anyhow::ensure!(!accents.is_empty(), "a palette needs an accent colour");
        let mut accents = accents.to_vec();
        accents.sort_by(|a, b| luminance(linear(*a)).total_cmp(&luminance(linear(*b))));
        accents.dedup();
        let ground = limit_luminance(linear(background), 0.012);
        let colours: Vec<Colour> = accents.iter().map(|accent| linear(*accent)).collect();
        let three: [Colour; 3] = match colours.len() {
            1 => [
                mix(ground, colours[0], 0.45),
                colours[0],
                mix(colours[0], [1.; 3], 0.35),
            ],
            2 => [colours[0], mix(colours[0], colours[1], 0.5), colours[1]],
            count => [colours[0], colours[count / 2], colours[count - 1]],
        };
        let mut three = three;
        three[0] = limit_luminance(three[0], 0.1);
        Ok(Self {
            background: srgb(ground),
            accents: three.map(srgb),
        })
    }
}

/// The colours written as `#rrggbb` (or `#rgb`) anywhere in `text`, in order. Pasting a palette
/// from a stylesheet, a swatch list or a chat message all work.
pub fn parse_colors(text: &str) -> Vec<Rgb> {
    let mut found = Vec::new();
    for word in text.split(|c: char| !(c == '#' || c.is_ascii_hexdigit())) {
        let Some(digits) = word.strip_prefix('#') else {
            continue;
        };
        let expanded = match digits.len() {
            6 => digits.to_string(),
            3 => digits.chars().flat_map(|c| [c, c]).collect(),
            _ => continue,
        };
        if let Ok(colour) = format!("#{expanded}").parse() {
            found.push(colour);
        }
    }
    found
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    pub version: u32,
    pub style: Style,
    pub seed: u64,
    pub palette: Palette,
}

impl Recipe {
    /// The recipe for `style` and `seed`, with the palette `seed` chooses.
    pub fn new(style: Style, seed: u64) -> Self {
        Self::with_palette(style, seed, Palette::from_seed(seed))
    }

    pub fn with_palette(style: Style, seed: u64, palette: Palette) -> Self {
        Self {
            version: VERSION,
            style,
            seed,
            palette,
        }
    }

    pub fn to_json(&self) -> Vec<u8> {
        serde_json::to_vec_pretty(self).expect("a recipe is always serializable")
    }

    pub fn from_json(bytes: &[u8]) -> anyhow::Result<Self> {
        let recipe: Self = serde_json::from_slice(bytes).context("reading a wallpaper recipe")?;
        anyhow::ensure!(
            recipe.version == VERSION,
            "wallpaper recipe version {} is not supported",
            recipe.version
        );
        Ok(recipe)
    }

    /// Names this exact recipe, so a change of any part of it is seen as a new wallpaper.
    pub fn digest(&self) -> [u8; 32] {
        Sha256::digest(self.to_json()).into()
    }

    /// The pixels `view` of the canvas has when drawn `width` by `height`. Channels are RGBA.
    pub fn render(&self, view: View, width: u32, height: u32) -> image::RgbaImage {
        let (width, height) = (width.max(1) as usize, height.max(1) as usize);
        let scene = Scene::new(self, view.aspect);
        let mut pixels = vec![0u8; width * height * 4];
        let threads = std::thread::available_parallelism()
            .map_or(1, usize::from)
            .min(height);
        let rows = height.div_ceil(threads);
        std::thread::scope(|scope| {
            for (chunk, block) in pixels.chunks_mut(rows * width * 4).enumerate() {
                let scene = &scene;
                scope.spawn(move || {
                    for (offset, row) in block.chunks_mut(width * 4).enumerate() {
                        let y = chunk * rows + offset;
                        draw_row(scene, view, width, height, y, row);
                    }
                });
            }
        });
        image::RgbaImage::from_raw(width as u32, height as u32, pixels)
            .expect("the buffer holds exactly width * height pixels")
    }

    /// The whole canvas as PNG, `edge` pixels along its longer side: what theme derivation, the
    /// lock screen and the login screen read in place of a photograph.
    pub fn reference_png(&self, aspect: f64, edge: u32) -> anyhow::Result<Vec<u8>> {
        let aspect = aspect.clamp(0.4, 4.);
        let (width, height) = if aspect >= 1. {
            (edge, (f64::from(edge) / aspect).round() as u32)
        } else {
            ((f64::from(edge) * aspect).round() as u32, edge)
        };
        let image = self.render(View::whole(aspect), width, height);
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(image).write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )?;
        Ok(bytes)
    }
}

/// Which part of the canvas to draw, and how wide the whole canvas is relative to its height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub aspect: f64,
    pub region: Region,
}

impl View {
    pub fn whole(aspect: f64) -> Self {
        Self {
            aspect,
            region: Region {
                left: 0.,
                top: 0.,
                width: 1.,
                height: 1.,
            },
        }
    }
}

fn draw_row(scene: &Scene, view: View, width: usize, height: usize, y: usize, row: &mut [u8]) {
    let region = view.region;
    let fy = region.top + (y as f64 + 0.5) / height as f64 * region.height;
    // One pixel's extent in canvas units, which is what keeps drawn lines a fixed thickness.
    let pixel = region.height / height as f64;
    for (x, out) in row.chunks_exact_mut(4).enumerate() {
        let fx = region.left + (x as f64 + 0.5) / width as f64 * region.width;
        let colour = scene.colour(fx * view.aspect, fy, pixel);
        let noise = dither(fx, fy);
        for channel in 0..3 {
            let encoded = encode(colour[channel]) * 255. + noise;
            out[channel] = encoded.round().clamp(0., 255.) as u8;
        }
        out[3] = 255;
    }
}

/// Triangular noise of about one level, a function of the position alone so that two views of the
/// same canvas agree. It breaks up the banding a dark smooth gradient would otherwise show.
fn dither(fx: f64, fy: f64) -> f64 {
    let (x, y) = (
        (fx * 1_048_576.) as i64 as u64,
        (fy * 1_048_576.) as i64 as u64,
    );
    let hash = mix64(x.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ y.wrapping_mul(0xc2b2_ae3d_27d4_eb4f));
    let first = (hash & 0xffff) as f64 / 65535.;
    let second = ((hash >> 16) & 0xffff) as f64 / 65535.;
    first + second - 1.
}

// ---- colour ---------------------------------------------------------------------------------

type Colour = [f64; 3];

fn decode(value: u8) -> f64 {
    let value = f64::from(value) / 255.;
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn encode(value: f64) -> f64 {
    let value = value.clamp(0., 1.);
    if value <= 0.003_130_8 {
        value * 12.92
    } else {
        1.055 * value.powf(1. / 2.4) - 0.055
    }
}

fn linear(colour: Rgb) -> Colour {
    colour.0.map(decode)
}

fn mix(from: Colour, to: Colour, amount: f64) -> Colour {
    let amount = amount.clamp(0., 1.);
    std::array::from_fn(|channel| from[channel] + (to[channel] - from[channel]) * amount)
}

fn scale(colour: Colour, factor: f64) -> Colour {
    colour.map(|channel| channel * factor)
}

fn add(left: Colour, right: Colour) -> Colour {
    std::array::from_fn(|channel| left[channel] + right[channel])
}

fn smoothstep(low: f64, high: f64, value: f64) -> f64 {
    let amount = ((value - low) / (high - low)).clamp(0., 1.);
    amount * amount * (3. - 2. * amount)
}

fn luminance(colour: Colour) -> f64 {
    0.2126 * colour[0] + 0.7152 * colour[1] + 0.0722 * colour[2]
}

/// `colour` darkened, hue kept, until its luminance is at most `ceiling`.
fn limit_luminance(colour: Colour, ceiling: f64) -> Colour {
    let current = luminance(colour);
    if current > ceiling {
        scale(colour, ceiling / current)
    } else {
        colour
    }
}

fn srgb(colour: Colour) -> Rgb {
    Rgb(colour.map(|channel| (encode(channel) * 255.).round() as u8))
}

fn hsl(hue: f64, saturation: f64, lightness: f64) -> Rgb {
    let hue = hue.rem_euclid(360.) / 60.;
    let chroma = (1. - (2. * lightness - 1.).abs()) * saturation;
    let second = chroma * (1. - (hue % 2. - 1.).abs());
    let (red, green, blue) = match hue as u32 {
        0 => (chroma, second, 0.),
        1 => (second, chroma, 0.),
        2 => (0., chroma, second),
        3 => (0., second, chroma),
        4 => (second, 0., chroma),
        _ => (chroma, 0., second),
    };
    let floor = lightness - chroma / 2.;
    Rgb([red, green, blue].map(|channel| ((channel + floor) * 255.).round().clamp(0., 255.) as u8))
}

// ---- randomness and noise -------------------------------------------------------------------

fn mix64(mut value: u64) -> u64 {
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

struct Random(u64);

impl Random {
    fn new(seed: u64) -> Self {
        Self(mix64(seed))
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        mix64(self.0)
    }

    /// In `[0, 1)`.
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }
}

const GRADIENTS: [(f64, f64); 8] = [
    (1., 0.),
    (-1., 0.),
    (0., 1.),
    (0., -1.),
    (0.707_106_8, 0.707_106_8),
    (-0.707_106_8, 0.707_106_8),
    (0.707_106_8, -0.707_106_8),
    (-0.707_106_8, -0.707_106_8),
];

#[derive(Clone, Copy)]
struct Noise {
    seed: u64,
}

impl Noise {
    fn gradient(&self, x: i64, y: i64) -> (f64, f64) {
        let hash = mix64(
            self.seed
                ^ (x as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
                ^ (y as u64).wrapping_mul(0xc2b2_ae3d_27d4_eb4f),
        );
        GRADIENTS[(hash >> 61) as usize]
    }

    /// Smooth gradient noise, roughly in `[-1, 1]`.
    fn perlin(&self, x: f64, y: f64) -> f64 {
        let (cell_x, cell_y) = (x.floor(), y.floor());
        let (fraction_x, fraction_y) = (x - cell_x, y - cell_y);
        let (cell_x, cell_y) = (cell_x as i64, cell_y as i64);
        let corner = |dx: i64, dy: i64| {
            let (gx, gy) = self.gradient(cell_x + dx, cell_y + dy);
            gx * (fraction_x - dx as f64) + gy * (fraction_y - dy as f64)
        };
        let fade = |value: f64| value * value * value * (value * (value * 6. - 15.) + 10.);
        let (u, v) = (fade(fraction_x), fade(fraction_y));
        let top = corner(0, 0) + (corner(1, 0) - corner(0, 0)) * u;
        let bottom = corner(0, 1) + (corner(1, 1) - corner(0, 1)) * u;
        (top + (bottom - top) * v) * 1.4
    }

    /// Several octaves of [`perlin`](Self::perlin), roughly in `[-1, 1]`. Each octave is turned a
    /// little so the lattice does not show as straight creases.
    fn fbm(&self, mut x: f64, mut y: f64, octaves: u32) -> f64 {
        let (mut total, mut amplitude, mut weight) = (0., 1., 0.);
        for octave in 0..octaves {
            total +=
                amplitude * self.perlin(x + f64::from(octave) * 17.3, y - f64::from(octave) * 9.1);
            weight += amplitude;
            amplitude *= 0.5;
            (x, y) = (2.02 * (x * 0.8 - y * 0.6), 2.02 * (x * 0.6 + y * 0.8));
        }
        (total / weight).clamp(-1., 1.)
    }
}

// ---- the scenes -----------------------------------------------------------------------------

struct Scene {
    style: Style,
    noise: Noise,
    aspect: f64,
    background: Colour,
    accents: [Colour; 3],
    /// Per-recipe choices that make one seed's picture differ from another's beyond its noise.
    offsets: [f64; 6],
}

impl Scene {
    fn new(recipe: &Recipe, aspect: f64) -> Self {
        let mut random = Random::new(recipe.seed);
        Self {
            style: recipe.style,
            noise: Noise {
                seed: mix64(recipe.seed ^ 0x6e6f_6973_65),
            },
            aspect,
            background: linear(recipe.palette.background),
            accents: recipe.palette.accents.map(linear),
            offsets: std::array::from_fn(|_| random.unit()),
        }
    }

    /// The linear colour at canvas position `(x, y)`, with `pixel` the canvas extent of a pixel.
    fn colour(&self, x: f64, y: f64, pixel: f64) -> Colour {
        match self.style {
            Style::Aurora => self.aurora(x, y),
            Style::Contours => self.contours(x, y, pixel),
            Style::Ridges => self.ridges(x, y, pixel),
        }
    }

    fn aurora(&self, x: f64, y: f64) -> Colour {
        let noise = &self.noise;
        let (shift_x, shift_y) = (self.offsets[0] * 40., self.offsets[1] * 40.);
        let (x, y) = (x * 0.9 + shift_x, y * 0.9 + shift_y);
        let warp_x = noise.fbm(x * 0.8, y * 0.8, 3);
        let warp_y = noise.fbm(x * 0.8 + 7.3, y * 0.8 - 2.1, 3);
        let (px, py) = (x + 0.7 * warp_x, y + 0.7 * warp_y);
        let body = noise.fbm(px * 1.2 + 3.1, py * 1.2, 5);
        // Filaments sit where a second field crosses zero.
        let filament = 1. - noise.fbm(px * 2.2, py * 2.2 + 9.0, 4).abs();
        let energy = smoothstep(-0.25, 0.85, body * 0.85 + filament.powf(3.) * 0.35 - 0.1);
        let glow = energy.powf(1.7);
        let ramp = glow * 3.;
        let accent = |index: usize| scale(self.accents[index], 0.9);
        let colour = if ramp < 1. {
            mix(self.background, accent(0), ramp)
        } else if ramp < 2. {
            mix(accent(0), accent(1), ramp - 1.)
        } else {
            mix(accent(1), accent(2), ramp - 2.)
        };
        // A little of the second accent lifts the thin, bright cores.
        add(
            colour,
            scale(self.accents[2], 0.18 * filament.powf(8.) * energy),
        )
    }

    fn contours(&self, x: f64, y: f64, pixel: f64) -> Colour {
        const LEVELS: f64 = 24.;
        let noise = &self.noise;
        let (shift_x, shift_y) = (self.offsets[0] * 40., self.offsets[1] * 40.);
        let height = |x: f64, y: f64| {
            let (x, y) = (x * 1.1 + shift_x, y * 1.1 + shift_y);
            let warp = noise.fbm(x * 0.7 + 4.4, y * 0.7, 2);
            (noise.fbm(x + 0.5 * warp, y - 0.5 * warp, 5) * 1.7 * 0.5 + 0.5).clamp(0., 1.)
        };
        let here = height(x, y);
        let step = (pixel * 0.75).max(1e-6);
        let slope_x = height(x + step, y) - height(x - step, y);
        let slope_y = height(x, y + step) - height(x, y - step);
        let slope = (slope_x * slope_x + slope_y * slope_y).sqrt() / (2. * step);
        let level = here * LEVELS;
        let fraction = level - level.floor();
        let between = fraction.min(1. - fraction);
        // Distance to the nearest line in pixels: levels away, over levels per pixel.
        let distance = between / (slope * LEVELS * pixel).max(1e-9);
        let major = (level.floor() as i64).rem_euclid(5) == 0;
        let half_width = if major { 0.0021 } else { 0.0012 } / pixel / 2.;
        let line = (half_width - distance + 0.5).clamp(0., 1.);
        let sheen = smoothstep(0.1, 0.9, here);
        let ground = mix(
            self.background,
            scale(self.accents[0], 0.14),
            sheen * 0.8 + y * 0.2,
        );
        let ink = if major {
            self.accents[2]
        } else {
            self.accents[1]
        };
        mix(ground, scale(ink, if major { 0.85 } else { 0.5 }), line)
    }

    fn ridges(&self, x: f64, y: f64, pixel: f64) -> Colour {
        let noise = &self.noise;
        let horizon = mix(self.background, scale(self.accents[1], 0.5), 0.42);
        let high = mix(self.background, scale(self.accents[0], 0.3), 0.3);
        let mut colour = mix(high, horizon, smoothstep(0.0, 0.7, y).powf(1.4));
        // A low sun, placed once for the whole canvas so a spanned picture has a single sun.
        let (sun_x, sun_y) = (self.aspect * (0.2 + 0.6 * self.offsets[0]), 0.42);
        let (dx, dy) = (x - sun_x, y - sun_y);
        let distance = (dx * dx + dy * dy).sqrt();
        let halo = (-distance * distance / 0.05).exp();
        let disc = 1. - smoothstep(0.07 - pixel, 0.07 + pixel, distance);
        colour = add(colour, scale(self.accents[2], 0.42 * halo));
        colour = mix(colour, mix(self.accents[2], [1.; 3], 0.35), disc * 0.9);

        const LAYERS: usize = 6;
        for layer in 0..LAYERS {
            let depth = layer as f64 / (LAYERS - 1) as f64;
            let base = 0.5 + 0.085 * layer as f64;
            let amplitude = 0.19 - 0.12 * depth;
            let frequency = 0.9 + 1.5 * depth;
            let origin = self.offsets[1 + layer % 5] * 100. + layer as f64 * 13.7;
            let smooth = noise.fbm(x * frequency + origin, layer as f64 * 7.7 + 0.5, 4);
            // Peaks come from folding a second field at zero, which leaves a sharp crest.
            let crest = 1.
                - 2. * noise
                    .fbm(x * frequency * 1.3 + origin, layer as f64 * 5.1 + 40., 4)
                    .abs();
            let wave = 0.4 * smooth + 0.6 * crest * (1. - 0.5 * depth);
            let top = base - amplitude * wave;
            let coverage = ((y - top) / pixel + 0.5).clamp(0., 1.);
            if coverage == 0. {
                continue;
            }
            let fog = mix(horizon, self.accents[1], 0.25);
            let body = mix(fog, self.background, 0.38 + 0.62 * depth);
            let lit = mix(body, self.accents[2], 0.3 * (1. - depth));
            let below = y - top;
            let surface = mix(lit, body, 1. - (-below * 7.).exp());
            colour = mix(colour, surface, coverage);
        }
        colour
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recipe(style: Style) -> Recipe {
        Recipe::new(style, 7)
    }

    #[test]
    fn the_same_recipe_draws_the_same_pixels() {
        for style in Style::ALL {
            let view = View::whole(16. / 9.);
            assert_eq!(
                recipe(style).render(view, 64, 36),
                recipe(style).render(view, 64, 36),
                "{style:?}"
            );
        }
    }

    #[test]
    fn a_seed_changes_the_picture_and_the_palette() {
        for style in Style::ALL {
            let view = View::whole(1.6);
            let first = Recipe::new(style, 1);
            let second = Recipe::new(style, 2);
            assert_ne!(first.palette, second.palette);
            assert_ne!(
                first.render(view, 48, 30),
                second.render(view, 48, 30),
                "{style:?}"
            );
        }
    }

    #[test]
    fn two_halves_drawn_apart_are_the_whole_drawn_at_once() {
        // The seam between two spanned monitors: no pixel may differ from the unsplit picture.
        let aspect = 3.;
        for style in Style::ALL {
            let recipe = recipe(style);
            let whole = recipe.render(View::whole(aspect), 120, 40);
            let half = |left: f64| View {
                aspect,
                region: Region {
                    left,
                    top: 0.,
                    width: 0.5,
                    height: 1.,
                },
            };
            let left = recipe.render(half(0.), 60, 40);
            let right = recipe.render(half(0.5), 60, 40);
            for y in 0..40 {
                for x in 0..60 {
                    assert_eq!(
                        whole.get_pixel(x, y),
                        left.get_pixel(x, y),
                        "{style:?} {x},{y}"
                    );
                    assert_eq!(
                        whole.get_pixel(x + 60, y),
                        right.get_pixel(x, y),
                        "{style:?} {}",
                        x + 60
                    );
                }
            }
        }
    }

    #[test]
    fn resolution_adds_detail_instead_of_blurring() {
        // The scene is a function of position, so a coarse drawing is the fine drawing sampled
        // at fewer places: neighbouring fine pixels agree closely with the coarse one. Contour
        // lines are thinner than a coarse pixel, so only the smooth styles can be compared.
        for style in [Style::Aurora, Style::Ridges] {
            let recipe = recipe(style);
            let view = View::whole(1.5);
            let coarse = recipe.render(view, 30, 20);
            let fine = recipe.render(view, 120, 80);
            let mut worst = 0u32;
            for y in 0..20 {
                for x in 0..30 {
                    let reference = coarse.get_pixel(x, y);
                    let sample = fine.get_pixel(x * 4 + 1, y * 4 + 1);
                    for channel in 0..3 {
                        let difference = u32::from(reference[channel].abs_diff(sample[channel]));
                        worst = worst.max(difference);
                    }
                }
            }
            assert!(worst < 90, "{style:?} differs by {worst}");
        }
    }

    #[test]
    fn pictures_stay_dark_and_use_the_whole_tonal_range() {
        for style in Style::ALL {
            let image = recipe(style).render(View::whole(16. / 9.), 96, 54);
            let lightness: Vec<f64> = image
                .pixels()
                .map(|pixel| {
                    (f64::from(pixel[0]) + f64::from(pixel[1]) + f64::from(pixel[2])) / 765.
                })
                .collect();
            let mean = lightness.iter().sum::<f64>() / lightness.len() as f64;
            let brightest = lightness.iter().copied().fold(0., f64::max);
            let darkest = lightness.iter().copied().fold(1., f64::min);
            assert!(mean < 0.4, "{style:?} is too bright: {mean}");
            assert!(brightest - darkest > 0.12, "{style:?} is flat");
        }
    }

    #[test]
    fn a_recipe_round_trips_and_rejects_other_versions() -> anyhow::Result<()> {
        let recipe = Recipe::new(Style::Contours, 99);
        assert_eq!(Recipe::from_json(&recipe.to_json())?, recipe);
        let text = String::from_utf8(recipe.to_json())?.replace("\"version\": 1", "\"version\": 9");
        assert!(Recipe::from_json(text.as_bytes()).is_err());
        assert!(Recipe::from_json(b"{}").is_err());
        assert!("#12345".parse::<Rgb>().is_err());
        assert_eq!("#0a0b0c".parse::<Rgb>()?, Rgb([10, 11, 12]));
        Ok(())
    }

    #[test]
    fn the_digest_follows_every_part_of_the_recipe() {
        let base = Recipe::new(Style::Aurora, 5);
        let mut other_seed = base.clone();
        other_seed.seed = 6;
        let mut other_colour = base.clone();
        other_colour.palette.background = Rgb([1, 2, 3]);
        let mut other_style = base.clone();
        other_style.style = Style::Ridges;
        for other in [other_seed, other_colour, other_style] {
            assert_ne!(base.digest(), other.digest());
        }
        assert_eq!(base.digest(), base.clone().digest());
    }

    #[test]
    fn the_reference_picture_has_the_canvas_shape() -> anyhow::Result<()> {
        let png = recipe(Style::Ridges).reference_png(2., 400)?;
        let image = image::load_from_memory(&png)?;
        assert_eq!((image.width(), image.height()), (400, 200));
        let tall = image::load_from_memory(&recipe(Style::Ridges).reference_png(0.5, 400)?)?;
        assert_eq!((tall.width(), tall.height()), (200, 400));
        Ok(())
    }

    #[test]
    fn colours_are_found_in_any_pasted_text() {
        let found =
            parse_colors("bg: #0B0B14;  accent #5b6ee1, #fa0 and #12345 or #zzzzzz, #abcdef0");
        assert_eq!(
            found,
            [Rgb([11, 11, 20]), Rgb([91, 110, 225]), Rgb([255, 170, 0])]
        );
        assert!(parse_colors("no colours here").is_empty());
    }

    #[test]
    fn a_pasted_palette_keeps_its_hues_and_stays_dark_enough_for_a_wallpaper() -> anyhow::Result<()>
    {
        // A light palette: the lightest is not the background, and nothing is washed out.
        let palette = Palette::from_colors(&[
            Rgb([250, 250, 250]),
            Rgb([20, 20, 40]),
            Rgb([200, 60, 60]),
            Rgb([60, 60, 200]),
        ])?;
        assert!(luminance(linear(palette.background)) <= 0.0125);
        assert!(
            palette.background.0[2] > palette.background.0[0],
            "keeps its blue cast"
        );
        assert!(luminance(linear(palette.accents[0])) <= 0.101);
        // The ramp runs from dim to bright.
        let lum = |index: usize| luminance(linear(palette.accents[index]));
        assert!(lum(0) <= lum(1) && lum(1) <= lum(2));
        // One accent or two are enough; fewer are not.
        assert!(Palette::from_colors(&[Rgb([0, 0, 0]), Rgb([200, 100, 50])]).is_ok());
        assert!(Palette::from_colors(&[Rgb([0, 0, 0])]).is_err());
        assert!(Palette::from_colors(&[Rgb([9, 9, 9]), Rgb([9, 9, 9])]).is_err());
        let three = Palette::from_colors(&[
            Rgb([0, 0, 0]),
            Rgb([10, 90, 60]),
            Rgb([60, 160, 120]),
            Rgb([180, 240, 200]),
        ])?;
        let picture =
            Recipe::with_palette(Style::Aurora, 1, three).render(View::whole(1.5), 32, 20);
        assert_ne!(
            picture,
            Recipe::new(Style::Aurora, 1).render(View::whole(1.5), 32, 20)
        );
        Ok(())
    }

    #[test]
    fn a_theme_gives_a_palette_in_its_own_colours() -> anyhow::Result<()> {
        use crate::extraction::{Palette as Swatches, Rgb as Swatch};
        use crate::theme::{Appearance, SemanticPalette};
        let swatches = Swatches {
            colors: [
                Swatch(26, 27, 38),
                Swatch(247, 118, 142),
                Swatch(158, 206, 106),
                Swatch(224, 175, 104),
                Swatch(122, 162, 247),
                Swatch(187, 154, 247),
                Swatch(42, 195, 222),
                Swatch(192, 202, 245),
                Swatch(65, 72, 104),
                Swatch(247, 118, 142),
                Swatch(158, 206, 106),
                Swatch(224, 175, 104),
                Swatch(122, 162, 247),
                Swatch(187, 154, 247),
                Swatch(42, 195, 222),
                Swatch(192, 202, 245),
            ],
        };
        for appearance in [Appearance::Dark, Appearance::Light] {
            let theme = SemanticPalette::derive(&swatches, appearance);
            let palette = Palette::from_theme(&theme)?;
            // Dark either way: a light theme contributes its accents, not its pale background.
            assert!(
                luminance(linear(palette.background)) <= 0.0125,
                "{appearance:?}"
            );
            // The bluish theme gives a bluish wallpaper, not a random hue.
            assert!(
                palette.accents[2].0[2] > palette.accents[2].0[0],
                "{appearance:?}"
            );
        }
        Ok(())
    }

    #[test]
    fn styles_are_named_for_the_command_line() {
        for style in Style::ALL {
            assert_eq!(Style::parse(style.name()), Some(style));
        }
        assert_eq!(Style::parse("plaid"), None);
    }
}
