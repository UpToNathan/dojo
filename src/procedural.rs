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
    /// Curtains of light over a dark sky and stars.
    Aurora,
    /// Soft, flowing blooms of colour.
    #[serde(alias = "mesh")]
    Ambient,
    /// Leaded glass lit from behind: translucent panes divided by dark lead.
    StainedGlass,
    /// Layered, soft dunes, each with a lit crest.
    Dunes,
}

impl Style {
    pub const ALL: [Style; 6] = [Style::Contours, Style::Sunset, Style::Aurora, Style::Ambient, Style::StainedGlass, Style::Dunes];

    pub fn name(self) -> &'static str {
        match self {
            Self::Contours => "contours",
            Self::Sunset => "sunset",
            Self::Aurora => "aurora",
            Self::Ambient => "ambient",
            Self::StainedGlass => "stained-glass",
            Self::Dunes => "dunes",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Contours => "Contours",
            Self::Sunset => "Sunset",
            Self::Aurora => "Aurora",
            Self::Ambient => "Ambient",
            Self::StainedGlass => "Stained glass",
            Self::Dunes => "Dunes",
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
    /// Colours borrowed from a theme, so the wallpaper belongs with the rest of the desktop,
    /// used as the theme states them: the ground is its `background`, the shell's own colour.
    /// The accents are chosen by role, not by brightness, so the picture wears the theme's own
    /// identity: a shade between the ground and its primary, the primary itself, and its
    /// secondary. A theme's other colours (a cyan "info", a green "success") are left out.
    pub fn from_theme(theme: &crate::theme::SemanticPalette) -> anyhow::Result<Self> {
        let colour = |role: &str| -> anyhow::Result<Rgb> {
            theme
                .color(role)
                .parse()
                .with_context(|| format!("the theme's {role} colour"))
        };
        let (ground, primary, secondary) = (
            colour("background")?,
            colour("primary")?,
            colour("secondary")?,
        );
        let deep = srgb(mix(linear(ground), linear(primary), 0.45));
        Ok(Self {
            background: ground,
            accents: [deep, primary, secondary],
            light: theme.appearance == crate::theme::Appearance::Light,
        })
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
    /// two, ordered from dim to bright. Nothing is darkened or limited: the colours are the ones
    /// given.
    fn from_parts(background: Rgb, accents: &[Rgb]) -> anyhow::Result<Self> {
        anyhow::ensure!(!accents.is_empty(), "a palette needs an accent colour");
        let mut accents = accents.to_vec();
        accents.sort_by(|a, b| luminance(linear(*a)).total_cmp(&luminance(linear(*b))));
        accents.dedup();
        let three = match accents.len() {
            1 => [
                srgb(mix(linear(background), linear(accents[0]), 0.45)),
                accents[0],
                srgb(mix(linear(accents[0]), [1.; 3], 0.35)),
            ],
            2 => [
                accents[0],
                srgb(mix(linear(accents[0]), linear(accents[1]), 0.5)),
                accents[1],
            ],
            count => [accents[0], accents[count / 2], accents[count - 1]],
        };
        Ok(Self {
            background,
            accents: three,
            light: false,
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

// ---- illustrated scenes ---------------------------------------------------------------------
//
// Aurora, Mesh, Stained glass and Dunes are drawn the way an illustrator blends colour: on the
// encoded (gamma) values, so a dark ground and a bright accent make a dark, muted middle. They
// convert to linear light on the way out. Each is a function of canvas position, the recipe's
// noise and its per-seed offsets, and nothing else, so a monitor draws only the part it shows.

/// A colour as encoded (gamma) values in `[0, 1]`.
type Encoded = [f64; 3];

fn encoded_mix(from: Encoded, to: Encoded, amount: f64) -> Encoded {
    let amount = amount.clamp(0., 1.);
    std::array::from_fn(|channel| from[channel] + (to[channel] - from[channel]) * amount)
}

/// `base` plus `amount` of `other`, which is how light adds.
fn encoded_add(base: Encoded, other: Encoded, amount: f64) -> Encoded {
    std::array::from_fn(|channel| base[channel] + other[channel] * amount)
}

fn to_linear(colour: Encoded) -> Colour {
    colour.map(|channel| decode_unit(channel.clamp(0., 1.)))
}

/// The recipe's palette, as encoded values.
struct Ground {
    background: Encoded,
    accents: [Encoded; 3],
}

impl Ground {
    /// The ground, then the three accents, as one ramp over `[0, 1]`.
    fn ramp(&self, position: f64) -> Encoded {
        let position = position.clamp(0., 1.) * 3.;
        let stops = [
            self.background,
            self.accents[0],
            self.accents[1],
            self.accents[2],
        ];
        let index = (position.floor() as usize).min(2);
        encoded_mix(stops[index], stops[index + 1], position - index as f64)
    }
}

impl Scene {
    /// An aurora over a night landscape: layered ribbons of crisp, vertical rays above faceted ridges
    /// and pines, in a field of stars. Where the ribbons hang, how they lean and ripple, and the lay of
    /// the land come from the seed.
    fn aurora(&self, x: f64, y: f64, pixel: f64) -> Colour {
        let (ground, noise, o) = (&self.ground, &self.noise, &self.offsets);
        let shift = (o[0] - 0.5) * 0.12;
        let lean = (o[1] - 0.5) * 0.3;
        let phase = o[2] * std::f64::consts::TAU;
        let across = x / self.aspect - 0.5;
        let mut colour = encoded_mix(ground.background, ground.accents[0], 0.22 * smoothstep(0.15, 0.8, y));
        // Stars: many, small and crisp, a few larger. None among the ridges.
        for (scale, keep, radius, strength) in [(210., 0.955, 0.34, 0.55), (120., 0.975, 0.30, 0.8), (62., 0.992, 0.26, 1.0)] {
            let (cell_x, cell_y) = ((x * scale).floor() as i64, (y * scale).floor() as i64);
            let roll = self.hash(7 + scale as i64, cell_x, cell_y);
            if roll > keep {
                let (dx, dy) = (x * scale - cell_x as f64 - 0.5, y * scale - cell_y as f64 - 0.5);
                let distance = (dx * dx + dy * dy).sqrt();
                let size = radius * (0.5 + 0.5 * self.hash(8, cell_x, cell_y));
                let edge = (size - distance) / (pixel * scale) + 0.5;
                colour = encoded_mix(colour, [0.94, 0.97, 1.], edge.clamp(0., 1.) * strength * (1. - smoothstep(0.5, 0.78, y)));
            }
        }
        // The ribbons, farthest and highest first. Each is a band of vertical rays standing on a bright,
        // sharp lower edge and fading upward; every ray has its own length and brightness.
        for ribbon in 0..5usize {
            let index = ribbon as f64;
            let edge = 0.50
                + shift
                + 0.045 * index
                + 0.065 * (x * (1.7 + 0.35 * index) + index * 1.9 + phase).sin()
                + 0.06 * noise.fbm(x * 1.2 + index * 5., index * 3.7, 3)
                + lean * across;
            let width = 0.0042 + 0.0011 * index;
            let ray = (x / width).floor() as i64;
            let tone = self.hash(100 + ribbon as i64, ray, 0);
            let reach = (0.20 + 0.16 * noise.fbm(x * 2.4 + index * 9., 2. + index, 3).abs() * 2.2) * (0.55 + 0.7 * self.hash(110 + ribbon as i64, ray, 1));
            let height = (edge - y) / reach;
            if height < 0. || height > 1. {
                continue;
            }
            // Crisp at the foot of each ray, fading to nothing at its tip, and a thin dark gap between rays.
            let foot = ((edge - y) / pixel + 0.5).clamp(0., 1.);
            let inside = (x / width).fract();
            let gap = smoothstep(0.0, 0.16, inside) * smoothstep(1.0, 0.84, inside);
            let strength = foot * gap * (1. - height).powf(1.5) * (0.45 + 0.55 * tone);
            let hue = ground.ramp(0.45 + 0.55 * (0.5 * index / 4. + 0.5 * height));
            colour = encoded_add(colour, hue, strength * 0.62);
            // A bright core along each ray's foot.
            let core = (1. - smoothstep(0., 0.05, height)) * foot * gap;
            colour = encoded_add(colour, encoded_mix(hue, [1.; 3], 0.55), core * 0.28 * (0.5 + 0.5 * tone));
        }
        // The land: three ridges of flat, faceted ground, the nearest with pines, darkest in front.
        for layer in 0..3usize {
            let depth = layer as f64 / 2.;
            let base = 0.78 + 0.075 * layer as f64;
            let amplitude = 0.10 - 0.02 * layer as f64;
            let spacing = 0.10 - 0.025 * layer as f64;
            let trees = if layer >= 1 { self.pines(layer + 3, x, 0.3 + 0.7 * depth) } else { 0. };
            if y < base - amplitude * 1.3 - trees {
                continue;
            }
            let (_, left, right) = self.segment(layer + 3, x, spacing, base, amplitude);
            let along = ((x - left.0) / (right.0 - left.0)).clamp(0., 1.);
            let ridge = left.1 + (right.1 - left.1) * along;
            let coverage = ((y - (ridge - trees)) / pixel + 0.5).clamp(0., 1.);
            if coverage == 0. {
                continue;
            }
            let lean = if layer % 2 == 0 { 0.7 } else { -0.7 };
            let (k, from, to) = self.segment(layer + 3, x + (y - ridge) * lean, spacing, base, amplitude);
            let slope = (to.1 - from.1) / (to.0 - from.0);
            let facing = (slope / (1. + slope * slope).sqrt()).clamp(-1., 1.);
            let facet = (((y - ridge) / 0.035 + 0.6 * (x - from.0) / (to.0 - from.0) * lean.signum()).floor()) as i64;
            let jitter = self.hash(layer as i64 + 41, k, facet) - 0.5;
            let body = encoded_mix(ground.background, ground.accents[0], 0.55 * (1. - depth));
            let lit = encoded_mix(body, ground.accents[1], 0.10 * (1. - 0.4 * depth));
            let dark = encoded_mix(body, ground.background, 0.62);
            let surface = if y < ridge {
                dark
            } else {
                encoded_mix(dark, lit, (0.5 + 0.5 * facing + 0.2 * jitter).clamp(0., 1.))
            };
            colour = encoded_mix(colour, surface, coverage);
        }
        to_linear(colour)
    }

    /// Soft blooms of the accents drifting over the ground, with the field warped so none is round.
    fn ambient(&self, x: f64, y: f64, _pixel: f64) -> Colour {
        let (ground, noise, o) = (&self.ground, &self.noise, &self.offsets);
        let (warped_x, warped_y) = (
            x + noise.fbm(x * 0.9, y * 0.9, 3) * 0.35,
            y + noise.fbm(x * 0.9 + 5., y * 0.9 + 3., 3) * 0.35,
        );
        let place = |fraction_x: f64, fraction_y: f64, jitter: usize, other: usize| {
            (
                self.aspect * (fraction_x + (o[jitter] - 0.5) * 0.2),
                fraction_y + (o[other] - 0.5) * 0.24,
            )
        };
        let blooms = [
            (place(0.17, 0.30, 0, 1), 0.55, ground.accents[1]),
            (place(0.76, 0.72, 2, 3), 0.60, ground.accents[2]),
            (place(0.48, 0.90, 4, 5), 0.50, ground.accents[0]),
            (place(0.90, 0.15, 1, 4), 0.45, ground.accents[0]),
        ];
        let mut colour = ground.background;
        for ((bloom_x, bloom_y), radius, accent) in blooms {
            let distance = ((warped_x - bloom_x).powi(2) + (warped_y - bloom_y).powi(2)) / (radius * radius);
            colour = encoded_add(colour, accent, (-distance * 1.6).exp() * 0.8);
        }
        to_linear(colour)
    }

    /// Stained glass lit from behind: a close mosaic of panes of many sizes, each flat colour cut into
    /// two lit and shaded facets, divided by clean dark lead. Each seed cuts the glass differently and
    /// puts the light somewhere else.
    fn stained_glass(&self, x: f64, y: f64, pixel: f64) -> Colour {
        let (ground, noise, o) = (&self.ground, &self.noise, &self.offsets);
        let size = 0.082;
        let (u, v) = (x / size, y / size);
        let (cell_x, cell_y) = (u.floor() as i64, v.floor() as i64);
        // Each cell holds one pane's centre and a weight; a bigger weight makes a bigger pane, and the
        // lead stays straight (a power diagram).
        let site = |gx: i64, gy: i64| {
            (
                gx as f64 + 0.12 + 0.76 * self.hash(21, gx, gy),
                gy as f64 + 0.12 + 0.76 * self.hash(22, gx, gy),
                0.30 * self.hash(25, gx, gy),
            )
        };
        let power = |site: (f64, f64, f64)| (u - site.0).powi(2) + (v - site.1).powi(2) - site.2;
        let (mut nearest, mut pane) = (f64::MAX, (0, 0));
        for dy in -2..=2 {
            for dx in -2..=2 {
                let distance = power(site(cell_x + dx, cell_y + dy));
                if distance < nearest {
                    nearest = distance;
                    pane = (cell_x + dx, cell_y + dy);
                }
            }
        }
        let own = site(pane.0, pane.1);
        // How far the pixel is from the nearest lead: from each bisector of the power diagram.
        let mut border = f64::MAX;
        for dy in -3..=3 {
            for dx in -3..=3 {
                let other = (pane.0 + dx, pane.1 + dy);
                if other == pane {
                    continue;
                }
                let site_other = site(other.0, other.1);
                let (nx, ny) = (site_other.0 - own.0, site_other.1 - own.1);
                let length = (nx * nx + ny * ny).sqrt();
                let along = (site_other.0.powi(2) + site_other.1.powi(2) - own.0.powi(2) - own.1.powi(2) - (site_other.2 - own.2)) / (2. * length);
                border = border.min(along - (u * nx + v * ny) / length);
            }
        }
        // Each pane's colour is one of the accents' tones, with its own lightness; the glass nearer the
        // light shines more.
        let tone = self.hash(23, pane.0, pane.1);
        let family = (tone * 5.).floor() / 5. + 0.1 * self.hash(26, pane.0, pane.1);
        let base = ground.ramp(0.26 + 0.72 * family.clamp(0., 1.));
        let (light_x, light_y) = (self.aspect * (0.35 + 0.3 * o[3]), 0.10 + 0.25 * o[4]);
        let (to_x, to_y) = (x - light_x, y - light_y);
        let glow = (-(to_x * to_x + to_y * to_y) * 0.9).exp();
        // Two facets: a line through the pane's centre at its own angle splits the lit half from the shaded.
        let angle = self.hash(24, pane.0, pane.1) * std::f64::consts::PI;
        let side = ((u - own.0) * angle.cos() + (v - own.1) * angle.sin()).signum();
        let facet = 1. + 0.16 * side;
        let streak = noise.fbm(x * 8. + own.0, y * 70. + own.1, 2) * 0.5 + 0.5;
        let mut light = (0.66 + 0.5 * glow + 0.06 * (streak - 0.5)) * facet * (0.82 + 0.18 * self.hash(27, pane.0, pane.1));
        // A thin, brighter rim just inside the lead, like a bevelled edge.
        let rim = (-((border - 0.10) / 0.035).powi(2)).exp();
        light += 0.12 * rim;
        let mut colour = base.map(|channel| channel * light);
        colour = encoded_add(colour, encoded_mix(base, [1.; 3], 0.55), (light - 0.95).max(0.) * 0.8);
        // The lead: flat near black, evenly thick, crisp.
        let width = 0.045 + pixel / size;
        let lead = 1. - smoothstep(width - pixel / size, width + pixel / size, border);
        to_linear(encoded_mix(colour, encoded_mix(ground.background, [0.01, 0.012, 0.018], 0.85), lead))
    }

    /// Thirteen layers of dunes in flat, faceted colour: crests with a gentle windward face and a
    /// steep, shaded lee, ripples across the lit faces, haze in the distance, a moon and stars. The
    /// nearest dunes are the strongest accent.
    fn dunes(&self, x: f64, y: f64, pixel: f64) -> Colour {
        const LAYERS: usize = 13;
        let (ground, noise, o) = (&self.ground, &self.noise, &self.offsets);
        let sun = (self.aspect * (0.15 + 0.7 * o[1]), 0.12 + 0.12 * o[2]);
        let crest = |layer: usize, along: f64| {
            let depth = layer as f64 / (LAYERS - 1) as f64;
            let phase = layer as f64 * 7.3 + o[0] * 60.;
            let base = 0.34 + 0.62 * depth.powf(0.85);
            let amplitude = 0.035 + 0.075 * depth;
            let frequency = 2.1 + 1.3 * (1. - depth);
            // A smooth, asymmetric wave: a long gentle rise to a crest, then a shorter, steeper fall.
            // The spacing is warped by noise and every crest has its own height, so none repeats.
            let u = along * frequency + phase + 0.9 * noise.fbm(along * 0.8 + phase, 5. + phase * 0.1, 2);
            let (index, cycle) = (u.floor(), u - u.floor());
            let wave = smoothstep(0., 0.62, cycle) * (1. - smoothstep(0.62, 0.90, cycle));
            let height = 0.45 + 1.0 * self.hash(80 + layer as i64, index as i64, 0);
            base - amplitude * (0.62 * height * wave + 0.5 * noise.fbm(along * 1.4 + phase, phase * 0.3, 3))
        };
        let haze = |depth: f64| (1. - depth).powf(1.4);
        let mut colour = encoded_mix(ground.background, ground.accents[0], 0.20 * smoothstep(0.1, 0.7, y));
        // The moon and stars, in the sky.
        let moon = 1. - smoothstep(0.060 - pixel, 0.060 + pixel, ((x - sun.0).powi(2) + (y - sun.1).powi(2)).sqrt());
        colour = encoded_mix(colour, ground.accents[2], moon);
        let (cell_x, cell_y) = ((x * 150.).floor() as i64, (y * 150.).floor() as i64);
        if self.hash(51, cell_x, cell_y) > 0.986 && y < 0.5 {
            let (dx, dy) = (x * 150. - cell_x as f64 - 0.5, y * 150. - cell_y as f64 - 0.5);
            let edge = (0.3 - (dx * dx + dy * dy).sqrt()) / (pixel * 150.) + 0.5;
            colour = encoded_mix(colour, [0.92, 0.95, 1.], edge.clamp(0., 1.) * 0.6);
        }
        // The layers are opaque, so the nearest one whose crest is above the pixel is all there is to see.
        for layer in (0..LAYERS).rev() {
            let top = crest(layer, x);
            if y < top {
                continue;
            }
            let depth = layer as f64 / (LAYERS - 1) as f64;
            let slope = (crest(layer, x + 0.003) - crest(layer, x - 0.003)) / 0.006;
            // Lit on the windward face, which rises to the crest; the shorter lee falls away in shade.
            let facing = (-slope * 1.8).clamp(-1., 1.);
            // Flat bands of tone running down the face, leaning with the slope, so the face reads as cut.
            let band = ((y - top) / 0.03 + 0.5 * slope).floor() as i64;
            let jitter = self.hash(layer as i64 + 71, band, 0) - 0.5;
            let hue = ground.ramp(0.30 + 0.70 * depth.powf(0.8));
            let far = encoded_mix(ground.background, ground.accents[0], 0.55);
            let body = encoded_mix(encoded_mix(far, hue, 1. - haze(depth)), ground.background, 0.18 * (1. - depth));
            let lit = encoded_mix(body, ground.accents[2], 0.18 * (1. - 0.4 * depth));
            let shade = encoded_mix(body, ground.background, 0.52);
            // The face is flat bands of tone, a little brighter where it rises to the sun, with variation
            // from band to band down it.
            let tone = (0.62 + 0.26 * facing.max(0.) + 0.14 * jitter - 0.010 * band as f64).clamp(0., 1.);
            let mut surface = encoded_mix(shade, lit, tone);
            // The lee: a wedge of shadow just under the crest, deepest where the fall is steepest and
            // gone where it flattens, with a crisp edge.
            let reach = 0.055 * (-facing).max(0.);
            let in_shadow = (((reach - (y - top)) / pixel) * 0.5 + 0.5).clamp(0., 1.);
            surface = encoded_mix(surface, shade, in_shadow * 0.9);
            // Wind ripples: thin lines following the crest, strongest just below it on the lit face.
            let ripple = (((y - top) / 0.011).fract() - 0.5).abs();
            let line = (1. - smoothstep(0.03, 0.10, ripple)) * smoothstep(0.0, 0.006, y - top) * (1. - smoothstep(0.0, 0.14, y - top)) * tone;
            surface = encoded_add(surface, ground.accents[2], line * 0.10);
            // A crisp lit line along the crest.
            let rim = 1. - smoothstep(0., 0.0035 + pixel, y - top);
            surface = encoded_mix(surface, encoded_mix(lit, [1.; 3], 0.25), rim * 0.55 * (0.4 + 0.6 * tone));
            colour = surface;
            break;
        }
        to_linear(colour)
    }
}

// ---- colour ---------------------------------------------------------------------------------

type Colour = [f64; 3];

fn decode(value: u8) -> f64 {
    decode_unit(f64::from(value) / 255.)
}

fn decode_unit(value: f64) -> f64 {
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

/// [`mix`] done on the encoded (gamma) values, as a designer's colour picker does it. Blending in
/// linear light makes the middle of every blend glow; this keeps a blend of a dark ground and a
/// bright accent as dark and muted as it looks in a flat illustration.
fn blend(from: Colour, to: Colour, amount: f64) -> Colour {
    let amount = amount.clamp(0., 1.);
    std::array::from_fn(|channel| {
        let (a, b) = (encode(from[channel]), encode(to[channel]));
        decode_unit(a + (b - a) * amount)
    })
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

/// How many ridges a sunset has.
const SUNSET_LAYERS: usize = 8;

fn luminance(colour: Colour) -> f64 {
    0.2126 * colour[0] + 0.7152 * colour[1] + 0.0722 * colour[2]
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
    /// The same colours as encoded values, for the scenes that blend the way an illustrator does.
    ground: Ground,
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
            ground: Ground {
                background: recipe.palette.background.0.map(|channel| f64::from(channel) / 255.),
                accents: recipe.palette.accents.map(|accent| accent.0.map(|channel| f64::from(channel) / 255.)),
            },
            light: recipe.palette.light,
            offsets: std::array::from_fn(|_| random.unit()),
        }
    }

    /// The linear colour at canvas position `(x, y)`, with `pixel` the canvas extent of a pixel.
    fn colour(&self, x: f64, y: f64, pixel: f64) -> Colour {
        match self.style {
            Style::Contours => self.contours(x, y, pixel),
            Style::Sunset => self.sunset(x, y, pixel),
            Style::Aurora => self.aurora(x, y, pixel),
            Style::Ambient => self.ambient(x, y, pixel),
            Style::StainedGlass => self.stained_glass(x, y, pixel),
            Style::Dunes => self.dunes(x, y, pixel),
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
            shade: 0.07,
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
        // The ridges dip toward the sun, more the further away they are, so it sets in a valley.
        let depth = layer as f64 / (SUNSET_LAYERS - 1) as f64;
        let valley = 0.09 * (1. - 0.8 * depth) * (-((along - self.sun_x()) / 0.55).powi(2)).exp();
        (along, base - amplitude * (0.7 * broad + 0.5 * jag) + valley)
    }

    /// Where along the canvas the sun sets.
    fn sun_x(&self) -> f64 {
        self.aspect * (0.2 + 0.6 * self.offsets[0])
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

    /// A flat, low-poly valley at sunset: many layered faceted ridges, lighter and hazier in the
    /// distance and darker in front, dipping toward a plain sun, with pines along the front
    /// ridges and flat clouds across the sky. It is drawn in the theme's background, primary and
    /// secondary, blended in gamma space. The sky is the background itself, with a gradient no
    /// stronger than ten percent laid over it.
    fn sunset(&self, x: f64, y: f64, pixel: f64) -> Colour {
        let (ground, primary, secondary) = (self.background, self.accents[1], self.accents[2]);
        let glow = blend(primary, secondary, 0.5);
        // The sky is flat. The only gradient is a faint warmth toward the horizon, at most 10%.
        let mut colour = blend(ground, glow, 0.1 * smoothstep(0.15, 0.62, y));

        let (sun_x, sun_y) = (self.sun_x(), 0.5);
        let (dx, dy) = (x - sun_x, y - sun_y);
        let distance = (dx * dx + dy * dy).sqrt();
        // A flat disc, with no glow around it.
        let disc = 1. - smoothstep(0.075 - pixel, 0.075 + pixel, distance);
        colour = mix(colour, secondary, disc);

        if !self.light {
            let star = self.stars(x, y, pixel, sun_x, sun_y);
            colour = mix(colour, secondary, 0.5 * star);
        }
        // Flat clouds, as faint as the sky's gradient.
        colour = mix(colour, blend(colour, glow, 0.1), self.clouds(x, y, pixel));

        for layer in 0..SUNSET_LAYERS {
            let depth = layer as f64 / (SUNSET_LAYERS - 1) as f64;
            let base = 0.5 + 0.06 * layer as f64;
            let amplitude = 0.14 - 0.07 * depth;
            let spacing = 0.12 - 0.05 * depth;
            let trees = if layer >= SUNSET_LAYERS - 3 {
                self.pines(layer, x, depth)
            } else {
                0.
            };
            if y < base - amplitude * 1.3 - trees {
                continue;
            }
            let (_, left, right) = self.segment(layer, x, spacing, base, amplitude);
            let along = ((x - left.0) / (right.0 - left.0)).clamp(0., 1.);
            let ridge = left.1 + (right.1 - left.1) * along;
            let top = ridge - trees;
            let coverage = ((y - top) / pixel + 0.5).clamp(0., 1.);
            if coverage == 0. {
                continue;
            }

            // The facets are cut along lines that lean from the crests, so they read as triangles
            // and strips rather than as columns: shade by the segment found along the lean.
            let lean = if layer % 2 == 0 { 0.7 } else { -0.7 };
            let (k, from, to) =
                self.segment(layer, x + (y - ridge) * lean, spacing, base, amplitude);
            let slope = (to.1 - from.1) / (to.0 - from.0);
            // The surface faces the sun when it falls away from it. Straight below the sun the
            // facets are lit evenly, so the shading eases through zero there.
            let toward = ((sun_x - x) / 0.35).clamp(-1., 1.);
            let facing = (slope * toward / (1. + slope * slope).sqrt()).clamp(-1., 1.);
            let band_height = 0.03;
            let into =
                (y - ridge) / band_height + 0.6 * (x - from.0) / (to.0 - from.0) * lean.signum();
            let facet = into.floor() as i64;
            let jitter = self.hash(layer as i64 + 31, k, facet) - 0.5;

            // Far ridges are the secondary, nearer ones the primary. On a dark ground each ridge
            // is darker than the one behind it, ending in the ground itself; on a light ground
            // each is stronger, ending in the primary, so the front still stands out.
            let hue = blend(secondary, primary, (depth * 1.4).min(1.));
            let (lit, dark) = if self.light {
                let body = blend(ground, hue, 0.2 + 0.7 * depth);
                (blend(body, ground, 0.3), blend(body, hue, 0.3))
            } else {
                let body = blend(ground, hue, 0.5 * (1. - depth).powf(0.9));
                (
                    blend(body, hue, 0.2 * (1. - 0.5 * depth)),
                    blend(body, ground, 0.4),
                )
            };
            let surface = if y < ridge {
                // Above the ridge line the silhouette is a tree: one flat, dark shape.
                dark
            } else {
                let shade =
                    (0.5 + 0.5 * facing + 0.22 * jitter - 0.025 * facet as f64).clamp(0., 1.);
                mix(dark, lit, shade)
            };
            colour = mix(colour, surface, coverage);
        }
        colour
    }

    /// The height of the pines standing on ridge `layer` at `x`, in canvas units: stepped
    /// triangles, bigger the nearer the ridge.
    fn pines(&self, layer: usize, x: f64, depth: f64) -> f64 {
        let width = 0.03 + 0.04 * depth;
        let cell = (x / width).floor() as i64;
        let mut tallest: f64 = 0.;
        for c in cell - 1..=cell + 1 {
            let id = layer as i64 + 60;
            if self.hash(id, c, 0) > 0.72 {
                continue;
            }
            let centre = (c as f64 + 0.2 + 0.6 * self.hash(id, c, 1)) * width;
            let half = width * (0.45 + 0.3 * self.hash(id, c, 2));
            let height = width * (0.9 + 0.8 * self.hash(id, c, 3));
            let across = (x - centre).abs() / half;
            if across >= 1. {
                continue;
            }
            // Three tiers, each a shorter slope than the cone around it.
            let tiers = 3. * (1. - across);
            let profile = (tiers.floor() + tiers.fract() * 0.5) / 3.;
            tallest = tallest.max(height * profile);
        }
        tallest
    }

    /// How much of the point `(x, y)` is covered by a flat cloud, in `[0, 1]`: hexagonal strips,
    /// each with a shorter one stepped below it. The clouds keep to the upper sky.
    fn clouds(&self, x: f64, y: f64, pixel: f64) -> f64 {
        const CELL_W: f64 = 0.55;
        const CELL_H: f64 = 0.15;
        if y > 0.47 {
            return 0.;
        }
        let (column, row) = ((x / CELL_W).floor() as i64, (y / CELL_H).floor() as i64);
        let mut covered: f64 = 0.;
        for c in column - 1..=column + 1 {
            for r in row - 1..=row + 1 {
                if self.hash(c, r, 70) > 0.5 {
                    continue;
                }
                let cx = (c as f64 + 0.15 + 0.7 * self.hash(c, r, 71)) * CELL_W;
                let cy = (r as f64 + 0.25 + 0.5 * self.hash(c, r, 72)) * CELL_H;
                let width = 0.07 + 0.12 * self.hash(c, r, 73);
                let height = 0.011 + 0.01 * self.hash(c, r, 74);
                // The main strip and one shorter strip, stepped down and to the left.
                for (strip_x, strip_y, strip_w) in [
                    (cx, cy, width),
                    (cx - 0.35 * width, cy + 1.7 * height, 0.6 * width),
                ] {
                    let (ox, oy) = ((x - strip_x).abs(), (y - strip_y).abs());
                    let edge = (height - oy).min(strip_w * (1. - 0.5 * oy / height) - ox);
                    covered = covered.max((edge / pixel + 0.5).clamp(0., 1.));
                }
            }
        }
        covered
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

        // Higher ground drifts from the primary toward the secondary, in the ground, the relief
        // light and the lines, so both of the theme's colours are seen across the picture.
        let tone = tone(here, x, y).clamp(0., 1.);
        let high = smoothstep(0.35, 0.95, tone);
        // The ground is the theme's background, drawn toward the accents by a few percent.
        let mut ground = mix(
            self.background,
            mix(self.accents[0], self.accents[2], high * 0.6),
            0.03 * tone * look.tint,
        );
        if look.shade > 0. {
            // A light from the upper left: surfaces facing it brighten, those facing away dim.
            let (nx, ny) = (-slope_x * look.relief, -slope_y * look.relief);
            let length = (nx * nx + ny * ny + 1.).sqrt();
            let lit = (nx * -0.45 + ny * -0.55 + 0.70) / length - 0.70;
            ground = if lit >= 0. {
                add(
                    ground,
                    scale(
                        mix(self.accents[1], self.accents[2], 0.5 + 0.4 * high),
                        lit * look.shade,
                    ),
                )
            } else {
                // Shadow darkens without shifting the hue, which subtracting a colour would.
                scale(ground, (1. + lit * look.shade * 5.).max(0.45))
            };
        }
        let line = mix(self.accents[1], self.accents[2], high * 0.9);
        let (ink, strength) = match tier {
            0 => (mix(line, self.background, 0.35), 0.5),
            1 => (line, 0.78),
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
    fn the_moon_and_the_sun_are_the_themes_secondary_as_it_is() {
        // The brightest, most saturated thing in a sunset is its moon or sun, and it is the
        // palette's secondary colour itself, not a dimmed or lightened version of it.
        for light in [false, true] {
            let mut palette = Palette::from_colors(&[
                Rgb([0x1a, 0x1b, 0x26]),
                Rgb([0x7a, 0xa2, 0xf7]),
                Rgb([0xbb, 0x9a, 0xf7]),
            ])
            .unwrap();
            palette.light = light;
            let secondary = palette.accents[2];
            let image =
                Recipe::with_palette(Style::Sunset, 3, palette).render(View::whole(1.8), 360, 200);
            let found = image.pixels().any(|p| {
                p.0[..3]
                    .iter()
                    .zip(secondary.0)
                    .all(|(have, want)| have.abs_diff(want) <= 2)
            });
            assert!(found, "no pixel is the secondary colour (light: {light})");
        }
    }

    #[test]
    fn the_ground_stays_close_to_the_themes_background() {
        // The shell is drawn in the theme's background, so the wallpaper behind it should be that
        // colour with the picture as a departure from it, not a brighter wash. The median pixel
        // is the ground: lines cover a few percent.
        let palette = Palette::from_colors(&[
            Rgb([0x1a, 0x1b, 0x26]),
            Rgb([0x7a, 0xa2, 0xf7]),
            Rgb([0xbb, 0x9a, 0xf7]),
        ])
        .unwrap();
        let wanted = luminance(linear(palette.background));
        for (style, allowed) in [(Style::Contours, 1.8)] {
            let image =
                Recipe::with_palette(style, 21, palette.clone()).render(View::whole(1.9), 760, 400);
            let mut lums: Vec<f64> = image
                .pixels()
                .map(|p| luminance([decode(p[0]), decode(p[1]), decode(p[2])]))
                .collect();
            lums.sort_by(f64::total_cmp);
            let ground = lums[lums.len() / 2];
            assert!(
                ground <= wanted * allowed,
                "{style:?} ground is {:.1}x the background",
                ground / wanted
            );
        }
    }

    #[test]
    fn a_dark_sunset_shows_the_primary_and_the_secondary_not_grey() {
        // Dimming a colour by mixing it with the near-grey background leaves grey. The ridges
        // are blends of the theme's own blue and purple, so across a few pictures a good part
        // must be tinted well beyond the background's own faint blue, and both the blue and the
        // purple must be there.
        let palette = Palette::from_colors(&[
            Rgb([0x1a, 0x1b, 0x26]),
            Rgb([0x7a, 0xa2, 0xf7]),
            Rgb([0xbb, 0x9a, 0xf7]),
        ])
        .unwrap();
        let (mut tinted, mut blue, mut purple, mut total) = (0., 0., 0., 0.);
        for seed in [21, 7, 3] {
            let image = Recipe::with_palette(Style::Sunset, seed, palette.clone()).render(
                View::whole(1.9),
                380,
                200,
            );
            for p in image.pixels() {
                let (r, g, b) = (i32::from(p[0]), i32::from(p[1]), i32::from(p[2]));
                total += 1.;
                // The background itself is (26, 27, 38): blue exceeds green by 11.
                tinted += f64::from(u8::from(b - g >= 18));
                blue += f64::from(u8::from(b - g >= 22 && r <= g));
                purple += f64::from(u8::from(r - g >= 8 && b - r >= 10));
            }
        }
        let (tinted, blue, purple) = (tinted / total, blue / total, purple / total);
        assert!(tinted > 0.3, "only {:.0}% is tinted", tinted * 100.);
        assert!(blue > 0.03, "only {:.0}% leans blue", blue * 100.);
        assert!(purple > 0.05, "only {:.0}% leans purple", purple * 100.);
    }

    #[test]
    fn a_themes_own_background_is_the_ground_exactly() -> anyhow::Result<()> {
        use crate::extraction::{Palette as Swatches, Rgb as Swatch};
        use crate::theme::{Appearance, SemanticPalette};
        // Tokyo Night Storm and Tokyo Night differ only in their backgrounds, so a wallpaper that
        // flattened the background toward one deep tone would look the same in both.
        let theme = |background: Swatch| {
            let mut colors = [Swatch(192, 202, 245); 16];
            colors[0] = background;
            colors[4] = Swatch(122, 162, 247);
            colors[5] = Swatch(187, 154, 247);
            SemanticPalette::derive(&Swatches { colors }, Appearance::Dark)
        };
        let night = Palette::from_theme(&theme(Swatch(26, 27, 38)))?;
        let storm = Palette::from_theme(&theme(Swatch(36, 40, 59)))?;
        // No limit touches the ground: it is the theme's background as it is.
        assert_eq!(night.background, Rgb([26, 27, 38]));
        assert_eq!(storm.background, Rgb([36, 40, 59]));
        // Their primary and secondary are the same, so the accents that name them are too.
        assert_eq!(night.accents[1..], storm.accents[1..]);
        let view = View::whole(1.9);
        for style in Style::ALL {
            assert_ne!(
                Recipe::with_palette(style, 5, night.clone()).render(view, 96, 50),
                Recipe::with_palette(style, 5, storm.clone()).render(view, 96, 50),
                "{style:?}"
            );
        }
        Ok(())
    }

    #[test]
    fn the_sunset_sky_is_the_flat_background_with_at_most_a_ten_percent_overlay() {
        let ground = Rgb([0x1a, 0x1b, 0x26]);
        let palette =
            Palette::from_colors(&[ground, Rgb([0x7a, 0xa2, 0xf7]), Rgb([0xbb, 0x9a, 0xf7])])
                .unwrap();
        // A light palette has no stars, which are the one thing in a dark sky that is not the
        // background or a faint overlay; the sky and clouds are the same either way.
        let mut palette = palette;
        palette.light = true;
        let image =
            Recipe::with_palette(Style::Sunset, 7, palette).render(View::whole(1.9), 760, 400);
        // The top fifth of the canvas is open sky: no ridge reaches it. Apart from the odd star
        // and the faint clouds, every pixel is the background itself.
        let total = (image.width() * 80) as f64;
        let flat = (0..80)
            .flat_map(|y| (0..image.width()).map(move |x| (x, y)))
            .filter(|&(x, y)| {
                image.get_pixel(x, y).0[..3]
                    .iter()
                    .zip(ground.0)
                    .all(|(have, want)| have.abs_diff(want) <= 1)
            })
            .count() as f64;
        assert!(
            flat / total > 0.9,
            "only {:.0}% of the open sky is flat",
            flat / total * 100.
        );
        // Nothing in the sky strays from the background by more than ten percent of the way to
        // the accents: no pixel in the sky is brighter than that blend.
        let limit =
            |want: u8, accent: u8| f64::from(want) + 0.11 * (f64::from(accent) - f64::from(want));
        for y in 0..80 {
            for x in 0..image.width() {
                let pixel = image.get_pixel(x, y).0;
                // Stars are the secondary itself; skip pixels that are not near the background.
                if pixel[2] > 150 {
                    continue;
                }
                assert!(
                    f64::from(pixel[2]) <= limit(0x26, 0xf7),
                    "{x},{y}: {pixel:?}"
                );
            }
        }
    }

    #[test]
    fn a_sunset_has_many_layers_so_a_portrait_monitor_shows_a_scene() {
        // A tall, narrow slice of a wide canvas, as one rotated monitor of four shows, must hold
        // plenty of separate shapes, not two or three big ones.
        let palette = Palette::from_colors(&[
            Rgb([0x1a, 0x1b, 0x26]),
            Rgb([0x7a, 0xa2, 0xf7]),
            Rgb([0xbb, 0x9a, 0xf7]),
        ])
        .unwrap();
        let slice = View {
            aspect: 1.89,
            region: Region {
                left: 0.,
                top: 0.,
                width: 1080. / 4080.,
                height: 1.,
            },
        };
        let image = Recipe::with_palette(Style::Sunset, 7, palette).render(slice, 270, 500);
        let mut colours = std::collections::HashSet::new();
        for pixel in image.pixels() {
            colours.insert(pixel.0);
        }
        assert!(
            colours.len() > 80,
            "only {} distinct colours",
            colours.len()
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
    fn a_pasted_palette_is_used_exactly_as_given() -> anyhow::Result<()> {
        // The darkest colour is the ground and the rest are the accents, none of them changed.
        let palette = Palette::from_colors(&[
            Rgb([250, 250, 250]),
            Rgb([20, 20, 40]),
            Rgb([200, 60, 60]),
            Rgb([60, 60, 200]),
        ])?;
        assert_eq!(palette.background, Rgb([20, 20, 40]));
        assert_eq!(palette.accents[2], Rgb([250, 250, 250]));
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
                palette.background == theme.color("background").parse::<Rgb>().unwrap(),
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
    fn the_illustrated_styles_keep_their_dark_ground_in_a_light_theme() {
        // A light theme changes the accents, not the ground: the picture stays dark behind the windows.
        let mut palette = Palette::from_seed(5);
        palette.light = true;
        for style in [Style::Aurora, Style::Ambient, Style::StainedGlass, Style::Dunes] {
            let image = Recipe::with_palette(style, 5, palette.clone()).render(View::whole(1.8), 96, 54);
            let mean = image
                .pixels()
                .map(|pixel| (f64::from(pixel[0]) + f64::from(pixel[1]) + f64::from(pixel[2])) / 765.)
                .sum::<f64>()
                / (96. * 54.);
            assert!(mean < 0.4, "{style:?} is too bright on a light theme: {mean}");
        }
    }

    #[test]
    fn a_recipe_saved_as_mesh_still_reads_as_ambient() {
        let recipe = Recipe::new(Style::Ambient, 3);
        let saved = String::from_utf8(recipe.to_json()).unwrap().replace("\"ambient\"", "\"mesh\"");
        assert_eq!(Recipe::from_json(saved.as_bytes()).unwrap(), recipe);
        assert_eq!(Style::Ambient.label(), "Ambient");
    }

    #[test]
    fn the_new_styles_are_named_as_their_buttons_say() {
        assert_eq!(Style::parse("stained-glass"), Some(Style::StainedGlass));
        assert_eq!(Style::StainedGlass.label(), "Stained glass");
        assert_eq!(Style::ALL.len(), 6);
        for style in Style::ALL {
            let json = serde_json::to_string(&style).unwrap();
            assert_eq!(json, format!("\"{}\"", style.name()), "a recipe stores the name the command line uses");
        }
    }

    #[test]
    fn styles_are_named_for_the_command_line() {
        for style in Style::ALL {
            assert_eq!(Style::parse(style.name()), Some(style));
        }
        assert_eq!(Style::parse("plaid"), None);
    }
}
