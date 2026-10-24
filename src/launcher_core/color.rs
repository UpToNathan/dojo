//! 10 Colour parsing, conversion and contrast. See .scratch/launcher-features/issues/10-colour-picker-and-conversion.md.
//!
//! [`evaluate`] recognises a typed colour and returns every format the launcher offers for it
//! plus its WCAG contrast against black and white. [`from_unit_rgb`] turns the 0..1 channels that
//! Niri's `PickColor` returns into the same [`Color`], and [`answer`] gives that colour the same
//! rows.
//!
//! Accepted input (case-insensitive):
//!
//! - `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`. A hex colour needs its `#`: a bare `bad` or `cafe`
//!   is a word, not a colour.
//! - `rgb()` and `rgba()` with commas or spaces, channels as numbers (0..255) or percentages, and
//!   alpha as a number (0..1) or percentage, after a comma or a `/`.
//! - `hsl()` and `hsla()` with the hue in degrees (or `deg`, `grad`, `rad`, `turn`).
//! - `oklch(L C H)` with L as 0..1 or a percentage and C as a number or a percentage of 0.4.
//!   Colours outside the sRGB gamut keep their lightness and hue and lose chroma until they fit.
//! - The 148 CSS named colours, spelled exactly (`red` is a colour, `redshift` is not). A name
//!   can also be an application's name (`orange`), so the launcher should rank this row below
//!   an exact application match.
//!
//! The oklab matrices are the ones in `src/theme/refined.rs` (Ottosson's), kept as `f64` and
//! applied without the 8-bit rounding the theme code needs. Contrast ignores alpha: it is the
//! ratio of the opaque colour.

/// An sRGB colour with 8-bit channels and an alpha from 0 (clear) to 255 (opaque).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

/// One way of writing a colour for the clipboard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColorFormat {
    /// `Hex`, `RGB`, `HSL`, `OKLCH` or `Name`.
    pub label: &'static str,
    /// What the row shows and Enter copies.
    pub text: String,
}

/// The WCAG 2 level a contrast ratio reaches for normal text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WcagGrade {
    /// At least 7:1.
    Aaa,
    /// At least 4.5:1.
    Aa,
    /// At least 3:1, enough for large text only.
    AaLarge,
    Fail,
}

impl WcagGrade {
    pub fn label(self) -> &'static str {
        match self {
            Self::Aaa => "AAA",
            Self::Aa => "AA",
            Self::AaLarge => "AA large text",
            Self::Fail => "Fail",
        }
    }
}

/// A contrast ratio and what it reaches.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Contrast {
    pub ratio: f64,
    pub grade: WcagGrade,
}

impl Contrast {
    fn of(ratio: f64) -> Self {
        let grade = if ratio >= 7.0 {
            WcagGrade::Aaa
        } else if ratio >= 4.5 {
            WcagGrade::Aa
        } else if ratio >= 3.0 {
            WcagGrade::AaLarge
        } else {
            WcagGrade::Fail
        };
        Self { ratio, grade }
    }

    /// `21:1`, `4.54:1`.
    pub fn ratio_text(&self) -> String {
        format!("{}:1", trimmed(self.ratio, 2))
    }

    /// `4.54:1 AA`.
    pub fn text(&self) -> String {
        format!("{} {}", self.ratio_text(), self.grade.label())
    }
}

/// Everything the launcher shows for a colour.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorAnswer {
    pub color: Color,
    /// Hex, RGB, HSL, OKLCH, then Name when the colour is exactly a CSS named colour.
    pub formats: Vec<ColorFormat>,
    /// Contrast of this colour against black (black text on this colour).
    pub on_black: Contrast,
    /// Contrast of this colour against white (white text on this colour).
    pub on_white: Contrast,
}

/// Recognises a typed colour. `None` for anything that is not entirely a colour.
pub fn evaluate(query: &str) -> Option<ColorAnswer> {
    parse(query).map(answer)
}

/// The rows for a colour that did not come from typing, such as one picked from the screen.
pub fn answer(color: Color) -> ColorAnswer {
    let mut formats = vec![
        ColorFormat {
            label: "Hex",
            text: color.to_hex(),
        },
        ColorFormat {
            label: "RGB",
            text: color.to_rgb_css(),
        },
        ColorFormat {
            label: "HSL",
            text: color.to_hsl_css(),
        },
        ColorFormat {
            label: "OKLCH",
            text: color.to_oklch_css(),
        },
    ];
    if let Some(name) = color.css_name() {
        formats.push(ColorFormat {
            label: "Name",
            text: name.to_owned(),
        });
    }
    ColorAnswer {
        color,
        formats,
        on_black: Contrast::of(color.contrast_with(Color::BLACK)),
        on_white: Contrast::of(color.contrast_with(Color::WHITE)),
    }
}

/// The colour for channels in 0..1, as Niri's `PickedColor` reports them (it holds `f64`s; cast
/// them down). Out-of-range values clamp and NaN becomes 0. The result is opaque.
pub fn from_unit_rgb(r: f32, g: f32, b: f32) -> Color {
    let channel = |value: f32| {
        if value.is_nan() {
            0
        } else {
            (f64::from(value).clamp(0.0, 1.0) * 255.0).round() as u8
        }
    };
    Color {
        r: channel(r),
        g: channel(g),
        b: channel(b),
        a: 255,
    }
}

// ---------------------------------------------------------------- Color

impl Color {
    pub const BLACK: Color = Color::opaque(0, 0, 0);
    pub const WHITE: Color = Color::opaque(255, 255, 255);

    pub const fn opaque(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// Alpha as a fraction, 0..=1.
    pub fn alpha(&self) -> f64 {
        f64::from(self.a) / 255.0
    }

    fn linear(&self) -> [f64; 3] {
        [self.r, self.g, self.b].map(|channel| srgb_to_linear(f64::from(channel) / 255.0))
    }

    /// WCAG relative luminance, 0 (black) to 1 (white).
    pub fn luminance(&self) -> f64 {
        let [r, g, b] = self.linear();
        0.2126 * r + 0.7152 * g + 0.0722 * b
    }

    /// WCAG contrast ratio between two colours, 1 to 21. Alpha is ignored.
    pub fn contrast_with(&self, other: Color) -> f64 {
        let (a, b) = (self.luminance(), other.luminance());
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    /// Hue in degrees, saturation and lightness as fractions. Hue is 0 for greys.
    pub fn to_hsl(&self) -> (f64, f64, f64) {
        let [r, g, b] = [self.r, self.g, self.b].map(|channel| f64::from(channel) / 255.0);
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let lightness = (max + min) / 2.0;
        let delta = max - min;
        if delta == 0.0 {
            return (0.0, 0.0, lightness);
        }
        let saturation = delta / (1.0 - (2.0 * lightness - 1.0).abs());
        let hue = if max == r {
            ((g - b) / delta).rem_euclid(6.0)
        } else if max == g {
            (b - r) / delta + 2.0
        } else {
            (r - g) / delta + 4.0
        } * 60.0;
        (hue.rem_euclid(360.0), saturation.clamp(0.0, 1.0), lightness)
    }

    /// Oklab lightness (0..1), chroma and hue in degrees. Hue is 0 for greys.
    pub fn to_oklch(&self) -> (f64, f64, f64) {
        let [r, g, b] = self.linear();
        let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
        let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
        let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
        let lightness = 0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s;
        let a = 1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s;
        let b = 0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s;
        let chroma = a.hypot(b);
        if chroma < GREY_CHROMA {
            return (lightness.clamp(0.0, 1.0), 0.0, 0.0);
        }
        (
            lightness.clamp(0.0, 1.0),
            chroma,
            b.atan2(a).to_degrees().rem_euclid(360.0),
        )
    }

    /// `#rrggbb`, or `#rrggbbaa` when not opaque.
    pub fn to_hex(&self) -> String {
        if self.a == 255 {
            format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
        } else {
            format!("#{:02x}{:02x}{:02x}{:02x}", self.r, self.g, self.b, self.a)
        }
    }

    /// `rgb(255, 136, 0)`, or `rgba(255, 136, 0, 0.5)` when not opaque.
    pub fn to_rgb_css(&self) -> String {
        if self.a == 255 {
            format!("rgb({}, {}, {})", self.r, self.g, self.b)
        } else {
            format!(
                "rgba({}, {}, {}, {})",
                self.r,
                self.g,
                self.b,
                trimmed(self.alpha(), 2)
            )
        }
    }

    /// `hsl(32, 100%, 50%)`, or `hsla(32, 100%, 50%, 0.5)` when not opaque. Whole numbers.
    pub fn to_hsl_css(&self) -> String {
        let (hue, saturation, lightness) = self.to_hsl();
        let hue = (hue.round() as i32).rem_euclid(360);
        let saturation = (saturation * 100.0).round();
        let lightness = (lightness * 100.0).round();
        if self.a == 255 {
            format!("hsl({hue}, {saturation}%, {lightness}%)")
        } else {
            format!(
                "hsla({hue}, {saturation}%, {lightness}%, {})",
                trimmed(self.alpha(), 2)
            )
        }
    }

    /// `oklch(0.628 0.258 29.2)`, with ` / 0.5` for alpha when not opaque.
    pub fn to_oklch_css(&self) -> String {
        let (lightness, chroma, hue) = self.to_oklch();
        let body = format!(
            "{} {} {}",
            trimmed(lightness, 3),
            trimmed(chroma, 3),
            trimmed(hue, 1)
        );
        if self.a == 255 {
            format!("oklch({body})")
        } else {
            format!("oklch({body} / {})", trimmed(self.alpha(), 2))
        }
    }

    /// The CSS name of this colour when it is exactly a named colour (and opaque). Where CSS has
    /// aliases the common spelling is used: `cyan`, `magenta` and `gray`.
    pub fn css_name(&self) -> Option<&'static str> {
        if self.a != 255 {
            return None;
        }
        let packed = u32::from(self.r) << 16 | u32::from(self.g) << 8 | u32::from(self.b);
        NAMED_COLORS
            .iter()
            .find(|(name, value)| {
                *value == packed && !name.contains("grey") && !matches!(*name, "aqua" | "fuchsia")
            })
            .map(|(name, _)| *name)
    }
}

/// Below this oklab chroma a colour is grey and its hue is noise (white comes out near 4e-8).
const GREY_CHROMA: f64 = 1e-4;

fn srgb_to_linear(value: f64) -> f64 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(value: f64) -> f64 {
    if value <= 0.0031308 {
        12.92 * value
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}

/// Linear sRGB for an oklch colour, unclamped.
fn oklch_to_linear(lightness: f64, chroma: f64, hue: f64) -> [f64; 3] {
    let a = chroma * hue.to_radians().cos();
    let b = chroma * hue.to_radians().sin();
    let l = (lightness + 0.3963377774 * a + 0.2158037573 * b).powi(3);
    let m = (lightness - 0.1055613458 * a - 0.0638541728 * b).powi(3);
    let s = (lightness - 0.0894841775 * a - 1.2914855480 * b).powi(3);
    [
        4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
        -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
        -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s,
    ]
}

fn in_gamut(linear: [f64; 3]) -> bool {
    linear.iter().all(|c| (-0.000001..=1.000001).contains(c))
}

/// The sRGB colour for an oklch value. When it is outside the gamut, chroma shrinks (keeping
/// lightness and hue) until it fits.
fn oklch_to_color(lightness: f64, chroma: f64, hue: f64, alpha: u8) -> Color {
    let lightness = lightness.clamp(0.0, 1.0);
    let mut chroma = chroma.max(0.0);
    if !in_gamut(oklch_to_linear(lightness, chroma, hue)) {
        let (mut low, mut high) = (0.0, chroma);
        for _ in 0..32 {
            let middle = (low + high) / 2.0;
            if in_gamut(oklch_to_linear(lightness, middle, hue)) {
                low = middle;
            } else {
                high = middle;
            }
        }
        chroma = low;
    }
    let [r, g, b] = oklch_to_linear(lightness, chroma, hue)
        .map(|c| (linear_to_srgb(c.clamp(0.0, 1.0)) * 255.0).round() as u8);
    Color { r, g, b, a: alpha }
}

fn hsl_to_color(hue: f64, saturation: f64, lightness: f64, alpha: u8) -> Color {
    let saturation = saturation.clamp(0.0, 1.0);
    let lightness = lightness.clamp(0.0, 1.0);
    let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let sector = hue.rem_euclid(360.0) / 60.0;
    let x = chroma * (1.0 - (sector.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match sector as u32 {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    let offset = lightness - chroma / 2.0;
    let [r, g, b] = [r, g, b].map(|c| ((c + offset) * 255.0).round().clamp(0.0, 255.0) as u8);
    Color { r, g, b, a: alpha }
}

/// Rounds to `places` decimals and drops trailing zeros: `0.50` is `0.5`, `-0.0` is `0`.
fn trimmed(value: f64, places: usize) -> String {
    let text = format!("{value:.places$}");
    let text = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.')
    } else {
        &text
    };
    if text == "-0" {
        "0".to_owned()
    } else {
        text.to_owned()
    }
}

// -------------------------------------------------------------- parsing

/// Parses one colour, or `None` when the text is anything else.
pub fn parse(query: &str) -> Option<Color> {
    let text = query.trim().to_lowercase();
    if text.is_empty() || text.len() > 100 {
        return None;
    }
    if let Some(hex) = text.strip_prefix('#') {
        return parse_hex(hex);
    }
    if let Some(open) = text.find('(') {
        let inner = text[open + 1..].strip_suffix(')')?;
        if inner.contains(['(', ')']) {
            return None;
        }
        return parse_function(&text[..open], inner);
    }
    named_color(&text)
}

fn parse_hex(hex: &str) -> Option<Color> {
    if !hex.is_ascii() || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let digit = |index: usize| u8::from_str_radix(&hex[index..=index], 16).ok();
    let pair = |index: usize| u8::from_str_radix(&hex[index..index + 2], 16).ok();
    match hex.len() {
        3 | 4 => Some(Color {
            r: digit(0)? * 17,
            g: digit(1)? * 17,
            b: digit(2)? * 17,
            a: if hex.len() == 4 { digit(3)? * 17 } else { 255 },
        }),
        6 | 8 => Some(Color {
            r: pair(0)?,
            g: pair(2)?,
            b: pair(4)?,
            a: if hex.len() == 8 { pair(6)? } else { 255 },
        }),
        _ => None,
    }
}

/// A number with an optional unit suffix.
struct Component {
    value: f64,
    unit: String,
}

fn component(token: &str) -> Option<Component> {
    let split = token
        .find(|c: char| c.is_ascii_alphabetic() || c == '%')
        .unwrap_or(token.len());
    let (number, unit) = token.split_at(split);
    // `f64::from_str` also accepts `inf` and `nan`; those never reach it.
    if !number.starts_with(|c: char| c.is_ascii_digit() || matches!(c, '.' | '-' | '+')) {
        return None;
    }
    let value: f64 = number.parse().ok()?;
    value.is_finite().then(|| Component {
        value,
        unit: unit.to_owned(),
    })
}

/// A 0..=255 colour channel: a number, or a percentage of 255.
fn channel(token: &str) -> Option<f64> {
    let c = component(token)?;
    match c.unit.as_str() {
        "" => Some(c.value.clamp(0.0, 255.0)),
        "%" => Some(c.value.clamp(0.0, 100.0) / 100.0 * 255.0),
        _ => None,
    }
}

/// A fraction from a number or a percentage: `0.5` and `50%` agree. `percent_scale` is the
/// value a bare 100% stands for.
fn fraction(token: &str, percent_scale: f64) -> Option<f64> {
    let c = component(token)?;
    match c.unit.as_str() {
        "" => Some(c.value),
        "%" => Some(c.value / 100.0 * percent_scale),
        _ => None,
    }
}

fn alpha_byte(token: Option<&str>) -> Option<u8> {
    match token {
        None => Some(255),
        Some(token) => Some((fraction(token, 1.0)?.clamp(0.0, 1.0) * 255.0).round() as u8),
    }
}

/// Degrees from `120`, `120deg`, `0.5turn`, `200grad` or `3.14rad`.
fn angle(token: &str) -> Option<f64> {
    let c = component(token)?;
    match c.unit.as_str() {
        "" | "deg" => Some(c.value),
        "turn" => Some(c.value * 360.0),
        "grad" => Some(c.value * 0.9),
        "rad" => Some(c.value.to_degrees()),
        _ => None,
    }
}

fn parse_function(name: &str, inner: &str) -> Option<Color> {
    if !matches!(name, "rgb" | "rgba" | "hsl" | "hsla" | "oklch") {
        return None;
    }
    // Commas, spaces and `/` all separate; a token after `/` is the alpha.
    let spaced = inner.replace('/', " / ");
    let tokens: Vec<&str> = spaced
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|token| !token.is_empty())
        .collect();
    let (colors, alpha) = match tokens.iter().position(|token| *token == "/") {
        Some(slash) => {
            if slash != 3 || tokens.len() != 5 {
                return None;
            }
            (&tokens[..3], Some(tokens[4]))
        }
        None => match tokens.len() {
            3 => (&tokens[..3], None),
            4 => (&tokens[..3], Some(tokens[3])),
            _ => return None,
        },
    };
    let alpha = alpha_byte(alpha)?;
    match name {
        "rgb" | "rgba" => Some(Color {
            r: channel(colors[0])?.round() as u8,
            g: channel(colors[1])?.round() as u8,
            b: channel(colors[2])?.round() as u8,
            a: alpha,
        }),
        "hsl" | "hsla" => Some(hsl_to_color(
            angle(colors[0])?,
            fraction(colors[1], 1.0).map(|s| {
                if colors[1].ends_with('%') {
                    s
                } else {
                    s / 100.0
                }
            })?,
            fraction(colors[2], 1.0).map(|l| {
                if colors[2].ends_with('%') {
                    l
                } else {
                    l / 100.0
                }
            })?,
            alpha,
        )),
        _ => Some(oklch_to_color(
            fraction(colors[0], 1.0)?,
            fraction(colors[1], 0.4)?,
            angle(colors[2])?,
            alpha,
        )),
    }
}

fn named_color(name: &str) -> Option<Color> {
    let (_, packed) = NAMED_COLORS.iter().find(|(known, _)| *known == name)?;
    Some(Color::opaque(
        (packed >> 16) as u8,
        (packed >> 8) as u8,
        *packed as u8,
    ))
}

/// The 148 CSS Color Level 4 named colours.
const NAMED_COLORS: [(&str, u32); 148] = [
    ("aliceblue", 0xf0f8ff),
    ("antiquewhite", 0xfaebd7),
    ("aqua", 0x00ffff),
    ("aquamarine", 0x7fffd4),
    ("azure", 0xf0ffff),
    ("beige", 0xf5f5dc),
    ("bisque", 0xffe4c4),
    ("black", 0x000000),
    ("blanchedalmond", 0xffebcd),
    ("blue", 0x0000ff),
    ("blueviolet", 0x8a2be2),
    ("brown", 0xa52a2a),
    ("burlywood", 0xdeb887),
    ("cadetblue", 0x5f9ea0),
    ("chartreuse", 0x7fff00),
    ("chocolate", 0xd2691e),
    ("coral", 0xff7f50),
    ("cornflowerblue", 0x6495ed),
    ("cornsilk", 0xfff8dc),
    ("crimson", 0xdc143c),
    ("cyan", 0x00ffff),
    ("darkblue", 0x00008b),
    ("darkcyan", 0x008b8b),
    ("darkgoldenrod", 0xb8860b),
    ("darkgray", 0xa9a9a9),
    ("darkgreen", 0x006400),
    ("darkgrey", 0xa9a9a9),
    ("darkkhaki", 0xbdb76b),
    ("darkmagenta", 0x8b008b),
    ("darkolivegreen", 0x556b2f),
    ("darkorange", 0xff8c00),
    ("darkorchid", 0x9932cc),
    ("darkred", 0x8b0000),
    ("darksalmon", 0xe9967a),
    ("darkseagreen", 0x8fbc8f),
    ("darkslateblue", 0x483d8b),
    ("darkslategray", 0x2f4f4f),
    ("darkslategrey", 0x2f4f4f),
    ("darkturquoise", 0x00ced1),
    ("darkviolet", 0x9400d3),
    ("deeppink", 0xff1493),
    ("deepskyblue", 0x00bfff),
    ("dimgray", 0x696969),
    ("dimgrey", 0x696969),
    ("dodgerblue", 0x1e90ff),
    ("firebrick", 0xb22222),
    ("floralwhite", 0xfffaf0),
    ("forestgreen", 0x228b22),
    ("fuchsia", 0xff00ff),
    ("gainsboro", 0xdcdcdc),
    ("ghostwhite", 0xf8f8ff),
    ("gold", 0xffd700),
    ("goldenrod", 0xdaa520),
    ("gray", 0x808080),
    ("green", 0x008000),
    ("greenyellow", 0xadff2f),
    ("grey", 0x808080),
    ("honeydew", 0xf0fff0),
    ("hotpink", 0xff69b4),
    ("indianred", 0xcd5c5c),
    ("indigo", 0x4b0082),
    ("ivory", 0xfffff0),
    ("khaki", 0xf0e68c),
    ("lavender", 0xe6e6fa),
    ("lavenderblush", 0xfff0f5),
    ("lawngreen", 0x7cfc00),
    ("lemonchiffon", 0xfffacd),
    ("lightblue", 0xadd8e6),
    ("lightcoral", 0xf08080),
    ("lightcyan", 0xe0ffff),
    ("lightgoldenrodyellow", 0xfafad2),
    ("lightgray", 0xd3d3d3),
    ("lightgreen", 0x90ee90),
    ("lightgrey", 0xd3d3d3),
    ("lightpink", 0xffb6c1),
    ("lightsalmon", 0xffa07a),
    ("lightseagreen", 0x20b2aa),
    ("lightskyblue", 0x87cefa),
    ("lightslategray", 0x778899),
    ("lightslategrey", 0x778899),
    ("lightsteelblue", 0xb0c4de),
    ("lightyellow", 0xffffe0),
    ("lime", 0x00ff00),
    ("limegreen", 0x32cd32),
    ("linen", 0xfaf0e6),
    ("magenta", 0xff00ff),
    ("maroon", 0x800000),
    ("mediumaquamarine", 0x66cdaa),
    ("mediumblue", 0x0000cd),
    ("mediumorchid", 0xba55d3),
    ("mediumpurple", 0x9370db),
    ("mediumseagreen", 0x3cb371),
    ("mediumslateblue", 0x7b68ee),
    ("mediumspringgreen", 0x00fa9a),
    ("mediumturquoise", 0x48d1cc),
    ("mediumvioletred", 0xc71585),
    ("midnightblue", 0x191970),
    ("mintcream", 0xf5fffa),
    ("mistyrose", 0xffe4e1),
    ("moccasin", 0xffe4b5),
    ("navajowhite", 0xffdead),
    ("navy", 0x000080),
    ("oldlace", 0xfdf5e6),
    ("olive", 0x808000),
    ("olivedrab", 0x6b8e23),
    ("orange", 0xffa500),
    ("orangered", 0xff4500),
    ("orchid", 0xda70d6),
    ("palegoldenrod", 0xeee8aa),
    ("palegreen", 0x98fb98),
    ("paleturquoise", 0xafeeee),
    ("palevioletred", 0xdb7093),
    ("papayawhip", 0xffefd5),
    ("peachpuff", 0xffdab9),
    ("peru", 0xcd853f),
    ("pink", 0xffc0cb),
    ("plum", 0xdda0dd),
    ("powderblue", 0xb0e0e6),
    ("purple", 0x800080),
    ("rebeccapurple", 0x663399),
    ("red", 0xff0000),
    ("rosybrown", 0xbc8f8f),
    ("royalblue", 0x4169e1),
    ("saddlebrown", 0x8b4513),
    ("salmon", 0xfa8072),
    ("sandybrown", 0xf4a460),
    ("seagreen", 0x2e8b57),
    ("seashell", 0xfff5ee),
    ("sienna", 0xa0522d),
    ("silver", 0xc0c0c0),
    ("skyblue", 0x87ceeb),
    ("slateblue", 0x6a5acd),
    ("slategray", 0x708090),
    ("slategrey", 0x708090),
    ("snow", 0xfffafa),
    ("springgreen", 0x00ff7f),
    ("steelblue", 0x4682b4),
    ("tan", 0xd2b48c),
    ("teal", 0x008080),
    ("thistle", 0xd8bfd8),
    ("tomato", 0xff6347),
    ("turquoise", 0x40e0d0),
    ("violet", 0xee82ee),
    ("wheat", 0xf5deb3),
    ("white", 0xffffff),
    ("whitesmoke", 0xf5f5f5),
    ("yellow", 0xffff00),
    ("yellowgreen", 0x9acd32),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb(query: &str) -> (u8, u8, u8, u8) {
        let color = parse(query).unwrap_or_else(|| panic!("{query:?} should be a colour"));
        (color.r, color.g, color.b, color.a)
    }

    fn assert_near(actual: f64, expected: f64, tolerance: f64) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "{actual} is not within {tolerance} of {expected}"
        );
    }

    fn text(query: &str, label: &str) -> String {
        evaluate(query)
            .unwrap_or_else(|| panic!("{query:?} should be a colour"))
            .formats
            .into_iter()
            .find(|format| format.label == label)
            .unwrap_or_else(|| panic!("no {label} format for {query:?}"))
            .text
    }

    #[test]
    fn parses_every_hex_length() {
        assert_eq!(rgb("#ff8800"), (255, 136, 0, 255));
        assert_eq!(rgb("#FF8800"), (255, 136, 0, 255));
        assert_eq!(rgb("#f80"), (255, 136, 0, 255));
        assert_eq!(rgb("#f808"), (255, 136, 0, 136));
        assert_eq!(rgb("#ff880080"), (255, 136, 0, 128));
        assert_eq!(rgb("  #000  "), (0, 0, 0, 255));
    }

    #[test]
    fn rejects_malformed_hex() {
        for query in [
            "#",
            "#f",
            "#ff",
            "#fffff",
            "#fffffff",
            "#fffffffff",
            "#gg0000",
            "ff8800",
            "bad",
            "cafe",
            "#ff 88 00",
            "##fff",
            "#ééé",
        ] {
            assert_eq!(parse(query), None, "{query:?}");
        }
    }

    #[test]
    fn parses_rgb_in_both_syntaxes() {
        assert_eq!(rgb("rgb(255, 136, 0)"), (255, 136, 0, 255));
        assert_eq!(rgb("rgb(255 136 0)"), (255, 136, 0, 255));
        assert_eq!(rgb("RGB(255,136,0)"), (255, 136, 0, 255));
        assert_eq!(rgb("rgba(255, 136, 0, 0.5)"), (255, 136, 0, 128));
        assert_eq!(rgb("rgb(255 136 0 / 50%)"), (255, 136, 0, 128));
        assert_eq!(rgb("rgb(255, 136, 0, 1)"), (255, 136, 0, 255));
        assert_eq!(rgb("rgb(100%, 50%, 0%)"), (255, 128, 0, 255));
        assert_eq!(rgb("rgb(300, -5, 0)"), (255, 0, 0, 255));
        assert_eq!(rgb("rgb(12.4 12.6 0)"), (12, 13, 0, 255));
        assert_eq!(rgb("rgba(0, 0, 0, 0)"), (0, 0, 0, 0));
    }

    #[test]
    fn parses_hsl() {
        assert_eq!(rgb("hsl(32, 100%, 50%)"), (255, 136, 0, 255));
        assert_eq!(rgb("hsl(32 100% 50%)"), (255, 136, 0, 255));
        assert_eq!(rgb("hsl(0, 100%, 50%)"), (255, 0, 0, 255));
        assert_eq!(rgb("hsl(120deg, 100%, 25%)"), (0, 128, 0, 255));
        assert_eq!(rgb("hsl(0.5turn, 100%, 50%)"), (0, 255, 255, 255));
        assert_eq!(rgb("hsl(360, 100%, 50%)"), (255, 0, 0, 255));
        assert_eq!(rgb("hsl(-120, 100%, 50%)"), (0, 0, 255, 255));
        assert_eq!(rgb("hsla(240, 100%, 50%, 0.5)"), (0, 0, 255, 128));
        assert_eq!(rgb("hsl(0, 0%, 100%)"), (255, 255, 255, 255));
        assert_eq!(rgb("hsl(0, 0%, 50%)"), (128, 128, 128, 255));
        assert_eq!(rgb("hsl(200, 50, 50)"), rgb("hsl(200, 50%, 50%)"));
    }

    #[test]
    fn parses_oklch() {
        assert_eq!(rgb("oklch(1 0 0)"), (255, 255, 255, 255));
        assert_eq!(rgb("oklch(0 0 0)"), (0, 0, 0, 255));
        assert_eq!(rgb("oklch(100% 0 0)"), (255, 255, 255, 255));
        assert_eq!(rgb("oklch(0.62796 0.25768 29.23)"), (255, 0, 0, 255));
        assert_eq!(rgb("oklch(62.796% 64.42% 29.23deg)"), (255, 0, 0, 255));
        assert_eq!(rgb("oklch(0.5 0.1 200 / 0.5)").3, 128);
    }

    #[test]
    fn out_of_gamut_oklch_keeps_lightness_and_hue() {
        let color = parse("oklch(0.7 0.4 150)").unwrap();
        let (lightness, chroma, hue) = color.to_oklch();
        assert_near(lightness, 0.7, 0.01);
        assert!(chroma < 0.4);
        assert_near(hue, 150.0, 2.0);
        assert_eq!(rgb("oklch(2 0 0)"), (255, 255, 255, 255));
        assert_eq!(rgb("oklch(-1 0 0)"), (0, 0, 0, 255));
    }

    #[test]
    fn rejects_malformed_functions() {
        for query in [
            "rgb()",
            "rgb(1, 2)",
            "rgb(1, 2, 3, 4, 5)",
            "rgb(1, 2, 3",
            "rgb 1 2 3",
            "rgb (1,2,3)",
            "rgb(a, b, c)",
            "rgb(1, 2, 3deg)",
            "rgb(inf, 0, 0)",
            "rgb(nan, 0, 0)",
            "hsl(1, 2, 3, 4, 5)",
            "hsl(red, 0%, 0%)",
            "hsl(1 2 3 / )",
            "rgb(1 2 3 / 0.5 0.5)",
            "lab(1 2 3)",
            "rgb(rgb(1,2,3), 0, 0)",
            "rgb(1,2,3))",
            "oklch(0.5 0.1)",
            "oklch(0.5 0.1 90 0.5 1)",
            "rgb(1/2/3)",
            "rgb(1 2 3 /)",
            "rgb(--1, 0, 0)",
            "hsl(0, 0%, 0%)x",
        ] {
            assert_eq!(parse(query), None, "{query:?}");
        }
    }

    #[test]
    fn table_has_all_148_distinct_css_names_with_valid_values() {
        assert_eq!(NAMED_COLORS.len(), 148);
        let mut names: Vec<_> = NAMED_COLORS.iter().map(|(name, _)| *name).collect();
        let sorted = names.clone();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 148);
        assert_eq!(names, sorted, "keep the table alphabetical");
        assert!(NAMED_COLORS.iter().all(|(_, value)| *value <= 0xffffff));
    }

    #[test]
    fn named_colours_parse_exactly() {
        assert_eq!(rgb("red"), (255, 0, 0, 255));
        assert_eq!(rgb("Red"), (255, 0, 0, 255));
        assert_eq!(rgb("rebeccapurple"), (102, 51, 153, 255));
        assert_eq!(rgb("  cornflowerblue "), (100, 149, 237, 255));
        assert_eq!(rgb("grey"), rgb("gray"));
        assert_eq!(rgb("aqua"), rgb("cyan"));
        assert_eq!(rgb("fuchsia"), rgb("magenta"));
        assert_eq!(rgb("lightgoldenrodyellow"), (250, 250, 210, 255));
    }

    #[test]
    fn words_that_only_start_with_a_colour_are_not_colours() {
        for query in [
            "redshift",
            "reddit",
            "blueman",
            "red hat",
            "darkred1",
            "gre",
            "gr",
            "re",
            "transparent",
            "firefox",
            "",
            "   ",
            "red!",
            "grayscale",
            "orange juice",
        ] {
            assert_eq!(evaluate(query), None, "{query:?}");
        }
    }

    #[test]
    fn hex_to_hsl_to_rgb_round_trips() {
        for packed in (0u32..=0xffffff).step_by(997) {
            let color = Color::opaque((packed >> 16) as u8, (packed >> 8) as u8, packed as u8);
            let (hue, saturation, lightness) = color.to_hsl();
            let back = hsl_to_color(hue, saturation, lightness, 255);
            assert_eq!(back, color, "{}", color.to_hex());
        }
    }

    #[test]
    fn rgb_to_oklch_to_rgb_round_trips() {
        for packed in (0u32..=0xffffff).step_by(1009) {
            let color = Color::opaque((packed >> 16) as u8, (packed >> 8) as u8, packed as u8);
            let (lightness, chroma, hue) = color.to_oklch();
            assert_eq!(
                oklch_to_color(lightness, chroma, hue, 255),
                color,
                "{}",
                color.to_hex()
            );
        }
    }

    #[test]
    fn displayed_formats_parse_back_to_the_same_colour() {
        for query in [
            "#ff8800",
            "#123456",
            "#fedcba",
            "#000000",
            "#ffffff",
            "#808080",
            "#12345680",
        ] {
            let original = parse(query).unwrap();
            let formats = answer(original).formats;
            for format in formats.iter().filter(|f| f.label != "HSL") {
                assert_eq!(
                    parse(&format.text),
                    Some(original),
                    "{query} as {}",
                    format.label
                );
            }
        }
    }

    #[test]
    fn reference_values_for_ff8800() {
        let answer = evaluate("#ff8800").unwrap();
        let texts: Vec<_> = answer
            .formats
            .iter()
            .map(|f| (f.label, f.text.as_str()))
            .collect();
        assert_eq!(texts[0], ("Hex", "#ff8800"));
        assert_eq!(texts[1], ("RGB", "rgb(255, 136, 0)"));
        assert_eq!(texts[2], ("HSL", "hsl(32, 100%, 50%)"));
        assert_eq!(texts.len(), 4, "no CSS name for ff8800");
        let (lightness, chroma, hue) = Color::opaque(255, 136, 0).to_oklch();
        assert_near(lightness, 0.7442, 0.001);
        assert_near(chroma, 0.1813, 0.001);
        assert_near(hue, 56.5, 0.5);
    }

    #[test]
    fn oklch_reference_values() {
        let (l, c, _) = Color::WHITE.to_oklch();
        assert_near(l, 1.0, 1e-4);
        assert_eq!(c, 0.0);
        assert_eq!(Color::BLACK.to_oklch(), (0.0, 0.0, 0.0));
        let (l, c, h) = Color::opaque(255, 0, 0).to_oklch();
        assert_near(l, 0.62796, 1e-4);
        assert_near(c, 0.25768, 1e-4);
        assert_near(h, 29.23, 0.01);
        let (l, c, h) = Color::opaque(0, 255, 0).to_oklch();
        assert_near(l, 0.86644, 1e-4);
        assert_near(c, 0.29483, 1e-4);
        assert_near(h, 142.50, 0.01);
        let (l, c, h) = Color::opaque(0, 0, 255).to_oklch();
        assert_near(l, 0.45201, 1e-4);
        assert_near(c, 0.31321, 1e-4);
        assert_near(h, 264.05, 0.01);
        assert_eq!(text("white", "OKLCH"), "oklch(1 0 0)");
        assert_eq!(text("black", "OKLCH"), "oklch(0 0 0)");
        assert_eq!(text("red", "OKLCH"), "oklch(0.628 0.258 29.2)");
    }

    #[test]
    fn hsl_reference_values() {
        assert_eq!(text("#ff0000", "HSL"), "hsl(0, 100%, 50%)");
        assert_eq!(text("#00ff00", "HSL"), "hsl(120, 100%, 50%)");
        assert_eq!(text("#0000ff", "HSL"), "hsl(240, 100%, 50%)");
        assert_eq!(text("#808080", "HSL"), "hsl(0, 0%, 50%)");
        assert_eq!(text("#ffffff", "HSL"), "hsl(0, 0%, 100%)");
        assert_eq!(text("#663399", "HSL"), "hsl(270, 50%, 40%)");
        assert_eq!(text("#ff0000", "RGB"), "rgb(255, 0, 0)");
    }

    #[test]
    fn alpha_shows_in_every_format_that_can_carry_it() {
        assert_eq!(text("#ff880080", "Hex"), "#ff880080");
        assert_eq!(text("#ff880080", "RGB"), "rgba(255, 136, 0, 0.5)");
        assert_eq!(text("#ff880080", "HSL"), "hsla(32, 100%, 50%, 0.5)");
        assert_eq!(text("#ff880080", "OKLCH"), "oklch(0.744 0.181 56.5 / 0.5)");
        let faded = evaluate("rgb(255 0 0 / 50%)").unwrap();
        assert!(
            faded.formats.iter().all(|f| f.label != "Name"),
            "a translucent colour has no CSS name"
        );
        assert_eq!(faded.color.alpha(), 128.0 / 255.0);
    }

    #[test]
    fn css_name_is_offered_only_for_exact_matches() {
        assert_eq!(
            evaluate("#ff0000").unwrap().formats.last().unwrap().text,
            "red"
        );
        assert_eq!(text("rgb(255, 99, 71)", "Name"), "tomato");
        assert_eq!(text("#00ffff", "Name"), "cyan");
        assert_eq!(text("#ff00ff", "Name"), "magenta");
        assert_eq!(text("#808080", "Name"), "gray");
        assert_eq!(Color::opaque(255, 0, 1).css_name(), None);
        assert_eq!(Color::opaque(255, 136, 0).css_name(), None);
    }

    #[test]
    fn every_css_name_maps_back_to_a_name_with_the_same_value() {
        for (name, _) in NAMED_COLORS {
            let color = named_color(name).unwrap();
            let back = color.css_name().unwrap();
            assert_eq!(named_color(back), Some(color), "{name}");
        }
    }

    #[test]
    fn contrast_reference_values() {
        assert_near(Color::BLACK.contrast_with(Color::WHITE), 21.0, 1e-9);
        assert_near(Color::WHITE.contrast_with(Color::WHITE), 1.0, 1e-9);
        assert_near(
            Color::opaque(0x76, 0x76, 0x76).contrast_with(Color::WHITE),
            4.54,
            0.005,
        );
        assert_near(
            Color::opaque(0x77, 0x77, 0x77).contrast_with(Color::WHITE),
            4.48,
            0.005,
        );
        let white = evaluate("white").unwrap();
        assert_near(white.on_black.ratio, 21.0, 1e-9);
        assert_near(white.on_white.ratio, 1.0, 1e-9);
        assert_eq!(white.on_black.grade, WcagGrade::Aaa);
        assert_eq!(white.on_white.grade, WcagGrade::Fail);
        let black = evaluate("black").unwrap();
        assert_eq!(black.on_black.text(), "1:1 Fail");
        assert_eq!(black.on_white.text(), "21:1 AAA");
        let orange = evaluate("#ff8800").unwrap();
        assert_near(orange.on_black.ratio, 8.77, 0.01);
        assert_near(orange.on_white.ratio, 2.39, 0.01);
        assert_eq!(orange.on_black.text(), "8.77:1 AAA");
    }

    #[test]
    fn contrast_grades_follow_wcag_thresholds() {
        assert_eq!(Contrast::of(7.0).grade, WcagGrade::Aaa);
        assert_eq!(Contrast::of(6.99).grade, WcagGrade::Aa);
        assert_eq!(Contrast::of(4.5).grade, WcagGrade::Aa);
        assert_eq!(Contrast::of(4.49).grade, WcagGrade::AaLarge);
        assert_eq!(Contrast::of(3.0).grade, WcagGrade::AaLarge);
        assert_eq!(Contrast::of(2.99).grade, WcagGrade::Fail);
        assert_eq!(Contrast::of(4.5).text(), "4.5:1 AA");
    }

    #[test]
    fn contrast_ignores_alpha() {
        let opaque = evaluate("#ff8800").unwrap();
        let faded = evaluate("#ff880040").unwrap();
        assert_eq!(opaque.on_black, faded.on_black);
        assert_eq!(opaque.on_white, faded.on_white);
    }

    #[test]
    fn from_unit_rgb_matches_what_niri_reports() {
        assert_eq!(from_unit_rgb(1.0, 0.0, 0.0), Color::opaque(255, 0, 0));
        assert_eq!(from_unit_rgb(0.0, 0.0, 0.0), Color::BLACK);
        assert_eq!(from_unit_rgb(1.0, 1.0, 1.0), Color::WHITE);
        assert_eq!(from_unit_rgb(0.5, 0.5, 0.5), Color::opaque(128, 128, 128));
        assert_eq!(from_unit_rgb(1.0, 136.0 / 255.0, 0.0).to_hex(), "#ff8800");
        // f64 channels from the IPC, cast down.
        assert_eq!(
            from_unit_rgb(0.2_f64 as f32, 0.4_f64 as f32, 0.6_f64 as f32),
            Color::opaque(51, 102, 153)
        );
    }

    #[test]
    fn from_unit_rgb_clamps_and_survives_bad_floats() {
        assert_eq!(from_unit_rgb(-1.0, 2.0, f32::NAN), Color::opaque(0, 255, 0));
        assert_eq!(
            from_unit_rgb(f32::INFINITY, f32::NEG_INFINITY, 0.0),
            Color::opaque(255, 0, 0)
        );
    }

    #[test]
    fn picked_colours_get_the_same_rows_as_typed_ones() {
        assert_eq!(
            answer(from_unit_rgb(1.0, 0.0, 0.0)),
            evaluate("red").unwrap()
        );
    }

    #[test]
    fn number_trimming() {
        assert_eq!(trimmed(0.5, 2), "0.5");
        assert_eq!(trimmed(21.0, 2), "21");
        assert_eq!(trimmed(-0.0001, 3), "0");
        assert_eq!(trimmed(59.74, 1), "59.7");
        assert_eq!(trimmed(100.0, 0), "100");
    }

    #[test]
    fn overlong_and_hostile_input_is_rejected() {
        assert_eq!(parse(&"a".repeat(1000)), None);
        assert_eq!(parse(&format!("#{}", "f".repeat(1000))), None);
        assert_eq!(parse("rgb(\u{0}, 0, 0)"), None);
        assert_eq!(parse("rgb(1e999, 0, 0)"), None);
    }
}
