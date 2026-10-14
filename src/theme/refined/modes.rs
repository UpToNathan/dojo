use crate::extraction::Mode;

pub(super) struct Style {
    pub hue: f64,
    pub secondary: f64,
    pub hues: [f64; 6],
    pub chroma: f64,
    pub tint: f64,
    pub lightness_offset: f64,
    pub contrast: f64,
}

fn rotate(h: f64, delta: f64) -> f64 {
    (h + delta).rem_euclid(360.)
}
fn toward(h: f64, target: f64, amount: f64) -> f64 {
    rotate(h, ((target - h + 180.).rem_euclid(360.) - 180.) * amount)
}

impl Style {
    pub fn new(mode: Mode, source_hue: f64, source_chroma: f64) -> Self {
        let canonical = [25., 145., 75., 250., 305., 195.];
        let mut hue = source_hue;
        let mut secondary = rotate(hue, 35.);
        let mut hues = canonical.map(|h| toward(h, source_hue, 0.15));
        let mut scale = 1.;
        let mut offset = 0.;
        let mut contrast = 4.5;
        let mut tint_scale = 1.;
        match mode {
            Mode::Normal => {}
            Mode::Monochromatic => {
                hues = [hue; 6];
                secondary = hue;
                scale = 0.35;
                tint_scale = 0.6;
            }
            Mode::Analogous => {
                hues = [-30., -15., 0., 15., 30., 45.].map(|d| rotate(hue, d));
                secondary = rotate(hue, 30.);
            }
            Mode::Complementary => {
                secondary = rotate(hue, 180.);
                hues = [hue, secondary, hue, secondary, hue, secondary];
            }
            Mode::Triadic => {
                secondary = rotate(hue, 120.);
                hues = [0., 120., 240., 0., 120., 240.].map(|d| rotate(hue, d));
            }
            Mode::SplitComplementary => {
                secondary = rotate(hue, 150.);
                hues = [0., 150., 210., 0., 150., 210.].map(|d| rotate(hue, d));
            }
            Mode::Tetradic => {
                secondary = rotate(hue, 90.);
                hues = [0., 90., 180., 270., 0., 180.].map(|d| rotate(hue, d));
            }
            Mode::Duotone => {
                secondary = rotate(hue, 180.);
                hues = [hue, hue, secondary, secondary, hue, secondary];
                scale = 0.8;
                tint_scale = 1.2;
            }
            Mode::Pastel => {
                scale = 0.55;
                offset = 0.05;
                tint_scale = 0.65;
            }
            Mode::Muted => {
                scale = 0.6;
                tint_scale = 0.75;
            }
            Mode::Material => {
                scale = 1.15;
                hues = canonical.map(|h| toward(h, source_hue, 0.25));
            }
            Mode::Colorful => {
                scale = 1.45;
                tint_scale = 1.25;
            }
            Mode::Bright => {
                scale = 1.25;
                offset = 0.06;
            }
            Mode::HighContrast => {
                scale = 1.2;
                contrast = 7.;
                tint_scale = 0.45;
            }
            Mode::Fire
            | Mode::Ocean
            | Mode::Forest
            | Mode::Earthtone
            | Mode::Neon
            | Mode::Sunset
            | Mode::Vaporwave
            | Mode::Midnight
            | Mode::Aurora => {
                let (anchor, family, strength, lightness, tint) = match mode {
                    Mode::Fire => (35., [15., 35., 65., 25., 355., 55.], 1.3, 0., 1.15),
                    Mode::Ocean => (215., [230., 175., 195., 220., 260., 190.], 1., 0., 1.25),
                    Mode::Forest => (145., [125., 150., 95., 165., 75., 175.], 0.85, 0., 1.3),
                    Mode::Earthtone => (65., [35., 105., 65., 85., 15., 125.], 0.65, -0.025, 1.2),
                    Mode::Neon => (300., [350., 150., 95., 260., 315., 195.], 1.8, 0.035, 0.9),
                    Mode::Sunset => (40., [20., 75., 55., 320., 350., 285.], 1.15, 0.025, 1.25),
                    Mode::Vaporwave => {
                        (310., [335., 190., 290., 250., 315., 205.], 1.25, 0.04, 1.35)
                    }
                    Mode::Midnight => {
                        (265., [280., 230., 250., 265., 300., 220.], 0.7, -0.025, 1.4)
                    }
                    Mode::Aurora => (
                        175.,
                        [155., 185., 110., 235., 290., 195.],
                        1.15,
                        0.015,
                        1.25,
                    ),
                    _ => unreachable!(),
                };
                let shift = ((source_hue - anchor + 180.).rem_euclid(360.) - 180.) * 0.12;
                hue = rotate(anchor, shift);
                hues = family.map(|h| rotate(h, shift));
                secondary = hues[4];
                scale = strength;
                offset = lightness;
                tint_scale = tint;
            }
        }
        let chroma = if source_chroma == 0. {
            0.
        } else {
            (source_chroma * 1.5).clamp(0.06, 0.14) * scale
        };
        Self {
            hue,
            secondary,
            hues,
            chroma: chroma.min(0.23),
            tint: (source_chroma * 0.4 * tint_scale).min(0.028),
            lightness_offset: offset,
            contrast,
        }
    }
}
