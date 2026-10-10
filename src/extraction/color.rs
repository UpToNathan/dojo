use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Lab {
    pub l: f64,
    pub a: f64,
    pub b: f64,
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Lch {
    pub l: f64,
    pub c: f64,
    pub h: f64,
}

fn linear(channel: f64) -> f64 {
    if channel <= 0.04045 {
        channel / 12.92
    } else {
        ((channel + 0.055) / 1.055).powf(2.4)
    }
}

fn srgb(channel: f64) -> u8 {
    let value = channel.clamp(0., 1.);
    let value = if value <= 0.0031308 {
        12.92 * value
    } else {
        1.055 * value.powf(1. / 2.4) - 0.055
    };
    (value * 255.).round().clamp(0., 255.) as u8
}

impl Rgb {
    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }

    pub fn parse(value: &str) -> anyhow::Result<Self> {
        let digits = value.strip_prefix('#').unwrap_or(value);
        anyhow::ensure!(
            digits.len() == 6 && digits.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid RGB color {value:?}"
        );
        let value = u32::from_str_radix(digits, 16)?;
        Ok(Self((value >> 16) as u8, (value >> 8) as u8, value as u8))
    }

    pub(super) fn lab(self) -> Lab {
        let r = linear(self.0 as f64 / 255.);
        let g = linear(self.1 as f64 / 255.);
        let b = linear(self.2 as f64 / 255.);
        let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
        let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
        // Aether uses the coefficients below, rather than the usual OKLab s row.
        let s = (0.0883024619 * r + 0.2164557844 * g + 0.6952417517 * b).cbrt();
        Lab {
            l: 0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
            a: 1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
            b: 0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s,
        }
    }

    pub(super) fn lch(self) -> Lch {
        self.lab().lch()
    }

    pub fn contrast(self, other: Self) -> f64 {
        let luminance = |color: Self| {
            0.2126 * linear(color.0 as f64 / 255.)
                + 0.7152 * linear(color.1 as f64 / 255.)
                + 0.0722 * linear(color.2 as f64 / 255.)
        };
        let a = luminance(self);
        let b = luminance(other);
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    pub(super) fn hsl(self) -> [f64; 3] {
        let [r, g, b] = [self.0, self.1, self.2].map(|v| v as f64 / 255.);
        let maximum = r.max(g).max(b);
        let minimum = r.min(g).min(b);
        let l = (maximum + minimum) / 2.;
        if maximum == minimum {
            return [0., 0., l * 100.];
        }
        let delta = maximum - minimum;
        let s = if l > 0.5 {
            delta / (2. - maximum - minimum)
        } else {
            delta / (maximum + minimum)
        };
        let h = if maximum == r {
            ((g - b) / delta + if g < b { 6. } else { 0. }) / 6.
        } else if maximum == g {
            ((b - r) / delta + 2.) / 6.
        } else {
            ((r - g) / delta + 4.) / 6.
        };
        [h * 360., s * 100., l * 100.]
    }

    pub(super) fn from_hsl(h: f64, s: f64, l: f64) -> Self {
        let (h, s, l) = (h / 360., s / 100., l / 100.);
        let q = if l < 0.5 { l * (1. + s) } else { l + s - l * s };
        let p = 2. * l - q;
        let channel = |mut t: f64| {
            if t < 0. {
                t += 1.;
            }
            if t > 1. {
                t -= 1.;
            }
            let value = if s == 0. {
                l
            } else if t < 1. / 6. {
                p + (q - p) * 6. * t
            } else if t < 0.5 {
                q
            } else if t < 2. / 3. {
                p + (q - p) * (2. / 3. - t) * 6.
            } else {
                p
            };
            (value * 255.).round().clamp(0., 255.) as u8
        };
        Self(channel(h + 1. / 3.), channel(h), channel(h - 1. / 3.))
    }
}

impl Lab {
    pub(super) fn rgb(self) -> Rgb {
        let l = (self.l + 0.3963377774 * self.a + 0.2158037573 * self.b).powi(3);
        let m = (self.l - 0.1055613458 * self.a - 0.0638541728 * self.b).powi(3);
        let s = (self.l - 0.0894841775 * self.a - 1.2914855480 * self.b).powi(3);
        Rgb(
            srgb(4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s),
            srgb(-1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s),
            srgb(-0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s),
        )
    }

    pub(super) fn lch(self) -> Lch {
        let mut h = self.b.atan2(self.a).to_degrees();
        if h < 0. {
            h += 360.;
        }
        Lch {
            l: self.l,
            c: (self.a * self.a + self.b * self.b).sqrt(),
            h,
        }
    }

    pub(super) fn distance(self, other: Self) -> f64 {
        ((self.l - other.l).powi(2) + (self.a - other.a).powi(2) + (self.b - other.b).powi(2))
            .sqrt()
    }
}

impl Lch {
    pub(super) fn new(l: f64, c: f64, h: f64) -> Self {
        Self { l, c, h }
    }
    pub(super) fn rgb(self) -> Rgb {
        let radians = self.h.to_radians();
        Lab {
            l: self.l,
            a: self.c * radians.cos(),
            b: self.c * radians.sin(),
        }
        .rgb()
    }
}
