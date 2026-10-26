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
    /// Topographic contour lines over shaded relief, with finer terrain on a finer monitor.
    Contours,
    /// Low-poly mountain ridges under a moon, or a sun in a light theme.
    Sunset,
}

impl Style {
    pub const ALL: [Style; 2] = [Style::Contours, Style::Sunset];

    pub fn name(self) -> &'static str {
        match self {
            Self::Contours => "contours",
            Self::Sunset => "sunset",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Contours => "Contours",
            Self::Sunset => "Sunset",
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
    /// The palette is a light theme's. The pictures stay on a dark ground either way; this only
    /// tells a scene that has a sun or a moon which to draw.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub light: bool,
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
            light: false,
        }
    }
}

impl Palette {
    /// Colours borrowed from a theme, so the wallpaper belongs with the rest of the desktop. The
    /// background is the theme's darker of background and foreground (a light theme's text). The
    /// accents are chosen by role, not by brightness, so the picture wears the theme's own
    /// identity: a deep shade of its primary colour, the primary itself, and its secondary. A
    /// theme's other colours (a cyan "info", a green "success") are left out.
    pub fn from_theme(theme: &crate::theme::SemanticPalette) -> anyhow::Result<Self> {
        let colour = |role: &str| -> anyhow::Result<Rgb> {
            theme
                .color(role)
                .parse()
                .with_context(|| format!("the theme's {role} colour"))
        };
        let (background, foreground) = (colour("background")?, colour("foreground")?);
        let ground = if luminance(linear(background)) <= luminance(linear(foreground)) {
            background
        } else {
            foreground
        };
        let (primary, secondary) = (linear(colour("primary")?), linear(colour("secondary")?));
        let deep = mix(linear(ground), primary, 0.45);
        let mut palette = Self::finish(ground, [deep, primary, secondary]);
        palette.light = theme.appearance == crate::theme::Appearance::Light;
        Ok(palette)
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
        Ok(Self::finish(background, three))
    }

    /// `accents` as given, with the background darkened to a deep tone and the dimmest accent held
    /// back, so a bright palette still gives a picture that sits behind windows.
    fn finish(background: Rgb, mut accents: [Colour; 3]) -> Self {
        let ground = limit_luminance(linear(background), 0.012);
        accents[0] = limit_luminance(accents[0], 0.1);
        Self {
            background: srgb(ground),
            accents: accents.map(srgb),
            light: false,
        }
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
    fn fbm(&self, x: f64, y: f64, octaves: u32) -> f64 {
        self.fbm_gain(x, y, octaves, 0.5)
    }

    /// [`fbm`](Self::fbm) with each octave `gain` times as strong as the last: nearer 1 is rougher.
    fn fbm_gain(&self, mut x: f64, mut y: f64, octaves: u32, gain: f64) -> f64 {
        let (mut total, mut amplitude, mut weight) = (0., 1., 0.);
        for octave in 0..octaves {
            total +=
                amplitude * self.perlin(x + f64::from(octave) * 17.3, y - f64::from(octave) * 9.1);
            weight += amplitude;
            amplitude *= gain;
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
    light: bool,
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
            light: recipe.palette.light,
            offsets: std::array::from_fn(|_| random.unit()),
        }
    }

    /// The linear colour at canvas position `(x, y)`, with `pixel` the canvas extent of a pixel.
    fn colour(&self, x: f64, y: f64, pixel: f64) -> Colour {
        match self.style {
            Style::Contours => self.contours(x, y, pixel),
            Style::Sunset => self.sunset(x, y, pixel),
        }
    }

    /// How many octaves of noise that starts at `base` cycles per canvas unit still have features
    /// a few pixels wide. A finer monitor gets finer detail rather than the same picture larger.
    fn octaves(base: f64, pixel: f64, cap: u32) -> u32 {
        let finest = 1. / (4. * pixel);
        let count = (finest / base).max(1.).log(2.02).floor() as u32 + 1;
        count.clamp(3, cap)
    }

    fn contours(&self, x: f64, y: f64, pixel: f64) -> Colour {
        let noise = &self.noise;
        let (shift_x, shift_y) = (self.offsets[0] * 40., self.offsets[1] * 40.);
        let octaves = Self::octaves(4.4, pixel, 8);
        let height = |x: f64, y: f64| {
            let (x, y) = (x * 1.1 + shift_x, y * 1.1 + shift_y);
            let warp = noise.fbm(x * 0.7 + 4.4, y * 0.7, 2);
            let broad = noise.fbm(x + 0.5 * warp, y - 0.5 * warp, 5);
            // Folding a second field at zero cuts ridge lines and gullies into the broad relief.
            let gully = 1. - 2. * noise.fbm(x * 1.7 + 11., y * 1.7, 3).abs();
            // The finest terrain is whatever the monitor has pixels to show.
            let detail = noise.fbm_gain(x * 4. + 3.3, y * 4., octaves, 0.55);
            // Squashed rather than clamped, so no plateau has an edge for the shading to catch.
            0.5 + 0.5 * ((broad * 1.55 + 0.3 * gully) * 0.95 + 0.05 * detail).tanh()
        };
        let look = Look {
            levels: 48.,
            relief: 0.3,
            shade: 0.16,
            tint: 1.,
            widths: [0.0007, 0.0011, 0.0017],
        };
        self.lines(&height, &|value, _, _| value, x, y, pixel, &look)
    }

    /// A number in `[0, 1)` fixed by the recipe and three integers.
    fn hash(&self, a: i64, b: i64, c: i64) -> f64 {
        let hash = mix64(
            self.noise.seed
                ^ (a as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
                ^ (b as u64).wrapping_mul(0xc2b2_ae3d_27d4_eb4f)
                ^ (c as u64).wrapping_mul(0x1656_67b1_9e37_79f9),
        );
        (hash >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Vertex `k` of a ridge: where it sits along the canvas and how high its crest is. Heights
    /// mix smooth noise with a random jag per vertex, so the crests come out sharp.
    fn vertex(&self, layer: usize, k: i64, spacing: f64, base: f64, amplitude: f64) -> (f64, f64) {
        let along = (k as f64 + (self.hash(layer as i64, k, 0) - 0.5) * 0.55) * spacing;
        let broad = self
            .noise
            .fbm(along * 1.3 + layer as f64 * 13.7, layer as f64 * 7.1, 3);
        let jag = self.hash(layer as i64, k, 1) * 2. - 1.;
        (along, base - amplitude * (0.7 * broad + 0.5 * jag))
    }

    /// The segment of ridge `layer` under `x`: its two vertices.
    fn segment(
        &self,
        layer: usize,
        x: f64,
        spacing: f64,
        base: f64,
        amplitude: f64,
    ) -> (i64, (f64, f64), (f64, f64)) {
        let guess = (x / spacing).floor() as i64;
        for k in guess - 1..=guess + 1 {
            let left = self.vertex(layer, k, spacing, base, amplitude);
            let right = self.vertex(layer, k + 1, spacing, base, amplitude);
            if left.0 <= x && x < right.0 {
                return (k, left, right);
            }
        }
        let k = guess;
        (
            k,
            self.vertex(layer, k, spacing, base, amplitude),
            self.vertex(layer, k + 1, spacing, base, amplitude),
        )
    }

    fn sunset(&self, x: f64, y: f64, pixel: f64) -> Colour {
        let white = [1.; 3];
        // The sky climbs from the background to a glow at the horizon, in the palette's own
        // colours. A light theme gets a bright sky and sun, a dark one a dim moon and stars.
        let lift = |colour: Colour| mix(colour, white, 0.5);
        let (top, horizon) = if self.light {
            (
                lift(self.accents[0]),
                lift(mix(self.accents[1], self.accents[2], 0.5)),
            )
        } else {
            (
                self.background,
                mix(self.background, scale(self.accents[1], 0.6), 0.55),
            )
        };
        let mut colour = mix(top, horizon, smoothstep(0.0, 0.62, y).powf(1.5));

        let (sun_x, sun_y) = (self.aspect * (0.2 + 0.6 * self.offsets[0]), 0.4);
        let (dx, dy) = (x - sun_x, y - sun_y);
        let distance = (dx * dx + dy * dy).sqrt();
        let (radius, halo, body) = if self.light {
            (0.07, 0.42, mix(self.accents[2], white, 0.55))
        } else {
            // Dim enough to sit behind a terminal: the brightest accent is no limit on the moon.
            (
                0.045,
                0.10,
                limit_luminance(mix(self.accents[2], self.accents[1], 0.35), 0.2),
            )
        };
        colour = add(
            colour,
            scale(body, halo * (-distance * distance / 0.045).exp()),
        );
        let disc = 1. - smoothstep(radius - pixel, radius + pixel, distance);
        colour = mix(colour, body, disc * if self.light { 0.95 } else { 0.8 });

        if !self.light {
            colour = add(
                colour,
                scale(self.accents[2], 0.5 * self.stars(x, y, pixel, sun_x, sun_y)),
            );
        }

        const LAYERS: usize = 5;
        for layer in 0..LAYERS {
            let depth = layer as f64 / (LAYERS - 1) as f64;
            let base = 0.56 + 0.09 * layer as f64;
            let amplitude = 0.17 - 0.07 * depth;
            let spacing = 0.2 - 0.07 * depth;
            if y < base - amplitude * 1.3 {
                continue;
            }
            let (_, left, right) = self.segment(layer, x, spacing, base, amplitude);
            let along = ((x - left.0) / (right.0 - left.0)).clamp(0., 1.);
            let top = left.1 + (right.1 - left.1) * along;
            let coverage = ((y - top) / pixel + 0.5).clamp(0., 1.);
            if coverage == 0. {
                continue;
            }

            // The facets are cut along lines that lean from the crests, so they read as triangles
            // and strips rather than as columns: shade by the segment found along the lean.
            let lean = if layer % 2 == 0 { 0.7 } else { -0.7 };
            let (k, from, to) = self.segment(layer, x + (y - top) * lean, spacing, base, amplitude);
            let slope = (to.1 - from.1) / (to.0 - from.0);
            // The surface faces the light when it falls away from it: down to the right of a
            // sun that is further right, say. Straight below the light the facets are lit
            // evenly, so the shading eases through zero there instead of flipping at a seam.
            let toward = ((sun_x - x) / 0.35).clamp(-1., 1.);
            let facing = (slope * toward / (1. + slope * slope).sqrt()).clamp(-1., 1.);
            let band_height = 0.045;
            let into =
                (y - top) / band_height + 0.6 * (x - from.0) / (to.0 - from.0) * lean.signum();
            let facet = into.floor() as i64;
            let jitter = self.hash(layer as i64 + 31, k, facet) - 0.5;

            let fog = mix(horizon, self.background, 0.15 + 0.5 * depth);
            let body = mix(fog, scale(self.background, 1.1), depth.powf(0.8));
            let body = add(body, scale(self.accents[0], 0.05 * (1. - depth)));
            let lit = mix(body, self.accents[1], 0.34 * (1. - 0.5 * depth));
            let dark = scale(body, 0.6);
            let shade = (0.5 + 0.5 * facing + 0.22 * jitter - 0.025 * facet as f64).clamp(0., 1.);
            colour = mix(colour, mix(dark, lit, shade), coverage);
        }
        colour
    }

    /// Sparse points of light in a dark sky, none near the moon or the horizon.
    fn stars(&self, x: f64, y: f64, pixel: f64, moon_x: f64, moon_y: f64) -> f64 {
        const CELL: f64 = 0.03;
        let (cx, cy) = ((x / CELL).floor() as i64, (y / CELL).floor() as i64);
        if self.hash(cx, cy, 90) > 0.16 {
            return 0.;
        }
        let star_x = (cx as f64 + 0.15 + 0.7 * self.hash(cx, cy, 91)) * CELL;
        let star_y = (cy as f64 + 0.15 + 0.7 * self.hash(cx, cy, 92)) * CELL;
        let radius = (pixel * 0.85).max(0.0011 * (0.6 + self.hash(cx, cy, 93)));
        let reach = (((x - star_x).powi(2) + (y - star_y).powi(2)).sqrt() / radius).min(1.);
        let sky = 1. - smoothstep(0.1, 0.5, y);
        let clear = smoothstep(
            0.1,
            0.22,
            ((star_x - moon_x).powi(2) + (star_y - moon_y).powi(2)).sqrt(),
        );
        (1. - reach).powi(2) * (0.25 + 0.6 * self.hash(cx, cy, 94)) * sky * clear
    }

    /// Level lines of `field` over a ground tinted by `tone` and shaded as relief, drawn
    /// anti-aliased at a thickness set in canvas units, so it is the same at any resolution.
    /// Every fifth line is stronger and every tenth stronger again.
    #[allow(clippy::too_many_arguments)]
    fn lines(
        &self,
        field: &dyn Fn(f64, f64) -> f64,
        tone: &dyn Fn(f64, f64, f64) -> f64,
        x: f64,
        y: f64,
        pixel: f64,
        look: &Look,
    ) -> Colour {
        let here = field(x, y);
        let step = (pixel * 0.75).max(1e-6);
        let slope_x = (field(x + step, y) - field(x - step, y)) / (2. * step);
        let slope_y = (field(x, y + step) - field(x, y - step)) / (2. * step);
        let slope = (slope_x * slope_x + slope_y * slope_y).sqrt();

        let level = here * look.levels;
        let nearest = level.round();
        // Distance to the nearest line in pixels: levels away, over levels per pixel.
        let distance = (level - nearest).abs() / (slope * look.levels * pixel).max(1e-9);
        let index = nearest as i64;
        let tier = if index.rem_euclid(10) == 0 {
            2
        } else if index.rem_euclid(5) == 0 {
            1
        } else {
            0
        };
        // Never thinner than about a pixel, or a line would fade out on a coarse monitor.
        let half_width = (look.widths[tier] / pixel / 2.).max(0.45);
        let coverage = (half_width - distance + 0.5).clamp(0., 1.);

        let mut ground = mix(
            self.background,
            scale(self.accents[0], 0.17),
            tone(here, x, y).clamp(0., 1.) * look.tint,
        );
        if look.shade > 0. {
            // A light from the upper left: surfaces facing it brighten, those facing away dim.
            let (nx, ny) = (-slope_x * look.relief, -slope_y * look.relief);
            let length = (nx * nx + ny * ny + 1.).sqrt();
            let lit = (nx * -0.45 + ny * -0.55 + 0.70) / length - 0.70;
            ground = if lit >= 0. {
                add(ground, scale(self.accents[1], lit * look.shade))
            } else {
                // Shadow darkens without shifting the hue, which subtracting a colour would.
                scale(ground, (1. + lit * look.shade * 5.).max(0.45))
            };
        }
        let (ink, strength) = match tier {
            0 => (mix(self.accents[1], self.background, 0.35), 0.5),
            1 => (self.accents[1], 0.78),
            _ => (self.accents[2], 1.),
        };
        mix(ground, ink, coverage * strength)
    }
}

/// How [`Scene::lines`] draws a field.
struct Look {
    /// Lines per unit of the field.
    levels: f64,
    /// How steeply relief shading reads the field's slope.
    relief: f64,
    /// How strongly relief shading lightens and darkens the ground.
    shade: f64,
    /// How far the ground follows the tone.
    tint: f64,
    /// Thickness of the thin, fifth and tenth lines, in canvas units.
    widths: [f64; 3],
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
        let base = Recipe::new(Style::Sunset, 5);
        let mut other_seed = base.clone();
        other_seed.seed = 6;
        let mut other_colour = base.clone();
        other_colour.palette.background = Rgb([1, 2, 3]);
        let mut other_style = base.clone();
        other_style.style = Style::Contours;
        for other in [other_seed, other_colour, other_style] {
            assert_ne!(base.digest(), other.digest());
        }
        assert_eq!(base.digest(), base.clone().digest());
    }

    #[test]
    fn the_reference_picture_has_the_canvas_shape() -> anyhow::Result<()> {
        let png = recipe(Style::Contours).reference_png(2., 400)?;
        let image = image::load_from_memory(&png)?;
        assert_eq!((image.width(), image.height()), (400, 200));
        let tall = image::load_from_memory(&recipe(Style::Contours).reference_png(0.5, 400)?)?;
        assert_eq!((tall.width(), tall.height()), (200, 400));
        Ok(())
    }

    #[test]
    fn the_moon_of_a_dark_theme_is_dim_and_the_sun_of_a_light_one_is_not() {
        let brightest = |light: bool| {
            let mut palette = Palette::from_colors(&[
                Rgb([10, 10, 20]),
                Rgb([90, 110, 200]),
                Rgb([230, 235, 255]),
            ])
            .unwrap();
            palette.light = light;
            let image =
                Recipe::with_palette(Style::Sunset, 3, palette).render(View::whole(1.8), 180, 100);
            image
                .pixels()
                .map(|p| (f64::from(p[0]) + f64::from(p[1]) + f64::from(p[2])) / 765.)
                .fold(0., f64::max)
        };
        let (dark, light) = (brightest(false), brightest(true));
        assert!(dark < 0.75, "the dark moon blinds: {dark}");
        assert!(
            light > dark + 0.15,
            "the light sun is no brighter: {light} vs {dark}"
        );
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
            Recipe::with_palette(Style::Sunset, 1, three).render(View::whole(1.5), 32, 20);
        assert_ne!(
            picture,
            Recipe::new(Style::Sunset, 1).render(View::whole(1.5), 32, 20)
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
            // The accents are the theme's primary and secondary, by role. Its cyan "info" colour,
            // which a sort by brightness would have put in the middle, is not used at all.
            let near = |colour: Rgb, role: &str| {
                let wanted: Rgb = theme.color(role).parse().unwrap();
                colour
                    .0
                    .iter()
                    .zip(wanted.0)
                    .all(|(have, want)| have.abs_diff(want) <= 2)
            };
            assert!(near(palette.accents[1], "primary"), "{appearance:?}");
            assert!(near(palette.accents[2], "secondary"), "{appearance:?}");
            assert!(
                palette.accents.iter().all(|accent| !near(*accent, "info")),
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
