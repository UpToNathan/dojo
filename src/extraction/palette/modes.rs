use super::*;

struct Mood {
    background_dark: Lch,
    background_light: Lch,
    foreground_dark: Lch,
    foreground_light: Lch,
    anchor: f64,
    tint: f64,
    chroma: (f64, f64),
    lightness: (f64, f64),
    stagger: [f64; 6],
}

fn mood(mode: Mode) -> Option<Mood> {
    let (
        background_dark,
        background_light,
        foreground_dark,
        foreground_light,
        anchor,
        tint,
        chroma,
        lightness,
        stagger,
    ) = match mode {
        Mode::Fire => (
            [0.09, 0.045, 28.],
            [0.95, 0.030, 45.],
            [0.90, 0.040, 55.],
            [0.22, 0.060, 20.],
            30.,
            0.28,
            (0.07, 0.12),
            (0.62, 0.46),
            [-0.05, 0.01, 0.07, -0.04, -0.02, 0.03],
        ),
        Mode::Ocean => (
            [0.10, 0.050, 240.],
            [0.96, 0.030, 215.],
            [0.90, 0.035, 205.],
            [0.20, 0.055, 230.],
            220.,
            0.28,
            (0.09, 0.14),
            (0.60, 0.44),
            [-0.05, 0.02, 0.06, -0.03, -0.01, 0.03],
        ),
        Mode::Forest => (
            [0.13, 0.040, 145.],
            [0.95, 0.025, 130.],
            [0.88, 0.035, 125.],
            [0.24, 0.050, 140.],
            145.,
            0.25,
            (0.06, 0.10),
            (0.66, 0.48),
            [-0.05, 0.02, 0.07, -0.03, -0.01, 0.04],
        ),
        Mode::Earthtone => (
            [0.18, 0.040, 40.],
            [0.93, 0.025, 60.],
            [0.84, 0.050, 50.],
            [0.26, 0.055, 35.],
            45.,
            0.40,
            (0.05, 0.09),
            (0.58, 0.44),
            [-0.10, -0.04, 0.04, 0.10, -0.07, 0.07],
        ),
        Mode::Neon => (
            [0.05, 0.030, 280.],
            [0.97, 0.015, 280.],
            [0.92, 0.045, 175.],
            [0.18, 0.060, 290.],
            0.,
            0.,
            (0.20, 0.26),
            (0.72, 0.50),
            [-0.03, 0.01, 0.04, -0.02, -0.01, 0.02],
        ),
        Mode::Sunset => (
            [0.12, 0.060, 350.],
            [0.96, 0.030, 30.],
            [0.90, 0.050, 30.],
            [0.24, 0.060, 350.],
            20.,
            0.32,
            (0.12, 0.18),
            (0.70, 0.50),
            [-0.05, 0.02, 0.07, -0.04, -0.02, 0.04],
        ),
        Mode::Vaporwave => (
            [0.15, 0.060, 315.],
            [0.96, 0.025, 320.],
            [0.90, 0.060, 190.],
            [0.26, 0.060, 310.],
            320.,
            0.28,
            (0.10, 0.16),
            (0.74, 0.52),
            [-0.04, 0.02, 0.06, -0.03, -0.01, 0.04],
        ),
        Mode::Midnight => (
            [0.07, 0.035, 250.],
            [0.95, 0.020, 240.],
            [0.88, 0.020, 220.],
            [0.22, 0.045, 250.],
            250.,
            0.20,
            (0.07, 0.12),
            (0.62, 0.42),
            [-0.04, 0.01, 0.05, -0.03, -0.02, 0.03],
        ),
        Mode::Aurora => (
            [0.08, 0.045, 240.],
            [0.96, 0.025, 220.],
            [0.92, 0.040, 175.],
            [0.20, 0.055, 240.],
            160.,
            0.42,
            (0.12, 0.18),
            (0.70, 0.50),
            [-0.05, 0.02, 0.06, -0.03, -0.01, 0.04],
        ),
        _ => return None,
    };
    let lch = |v: [f64; 3]| Lch::new(v[0], v[1], v[2]);
    Some(Mood {
        background_dark: lch(background_dark),
        background_light: lch(background_light),
        foreground_dark: lch(foreground_dark),
        foreground_light: lch(foreground_light),
        anchor,
        tint,
        chroma,
        lightness,
        stagger,
    })
}

fn dominant_hue(colors: &[Rgb]) -> f64 {
    let mut best = 0.;
    let mut hue = 0.;
    for color in colors {
        let lch = color.lch();
        if lch.c > best {
            best = lch.c;
            hue = lch.h;
        }
    }
    hue
}

pub(super) fn generate(colors: &[Rgb], light: bool, mode: Mode) -> Colors {
    if let Some(spec) = mood(mode) {
        let base = extract_hues(colors, &[], light);
        let mut result = [Rgb::default(); 16];
        result[0] = if light {
            spec.background_light
        } else {
            spec.background_dark
        }
        .rgb();
        result[7] = if light {
            spec.foreground_light
        } else {
            spec.foreground_dark
        }
        .rgb();
        for slot in 0..6 {
            let lch = base[slot + 1].lch();
            let difference = (spec.anchor - lch.h + 540.) % 360. - 180.;
            let h = (lch.h + difference * spec.tint + 360.) % 360.;
            result[slot + 1] = Lch::new(
                ((if light {
                    spec.lightness.1
                } else {
                    spec.lightness.0
                }) + spec.stagger[slot])
                    .clamp(0.05, 0.95),
                lch.c.clamp(spec.chroma.0, spec.chroma.1),
                h,
            )
            .rgb();
        }
        finalize(&mut result);
        return result;
    }
    match mode {
        Mode::Material => material(colors, light),
        Mode::Analogous => analogous(colors, light),
        Mode::Complementary
        | Mode::Triadic
        | Mode::SplitComplementary
        | Mode::Tetradic
        | Mode::Duotone => scheme(colors, light, mode),
        _ => transform(colors, light, mode),
    }
}

fn transform(colors: &[Rgb], light: bool, mode: Mode) -> Colors {
    let base = extract_hues(colors, &[], light);
    let background = base[0].lch();
    let foreground = base[7].lch();
    let (bg, fg, l, chroma, stagger): (Lch, Lch, f64, (f64, f64), [f64; 6]) = match (mode, light) {
        (Mode::Pastel, true) => (
            Lch::new(0.96, 0.012, background.h),
            Lch::new(0.32, 0.05, foreground.h),
            0.55,
            (0.045, 0.07),
            [-0.04, 0.02, 0.05, -0.03, -0.01, 0.03],
        ),
        (Mode::Pastel, false) => (
            Lch::new(0.20, 0.022, background.h),
            Lch::new(0.85, 0.04, foreground.h),
            0.78,
            (0.05, 0.075),
            [-0.04, 0.02, 0.05, -0.03, -0.01, 0.03],
        ),
        (Mode::Colorful, true) => (
            Lch::new(0.97, 0.012, background.h),
            Lch::new(0.18, 0.04, foreground.h),
            0.50,
            (0.14, 0.20),
            [-0.05, 0.02, 0.06, -0.03, -0.02, 0.04],
        ),
        (Mode::Colorful, false) => (
            Lch::new(0.10, 0.022, background.h),
            Lch::new(0.92, 0.03, foreground.h),
            0.65,
            (0.14, 0.20),
            [-0.05, 0.02, 0.06, -0.03, -0.02, 0.04],
        ),
        (Mode::Muted, true) => (
            Lch::new(0.94, 0.010, background.h),
            Lch::new(0.28, 0.045, foreground.h),
            0.48,
            (0.035, 0.065),
            [-0.10, -0.04, 0.04, 0.10, -0.07, 0.07],
        ),
        (Mode::Muted, false) => (
            Lch::new(0.16, 0.018, background.h),
            Lch::new(0.84, 0.035, foreground.h),
            0.62,
            (0.035, 0.065),
            [-0.10, -0.04, 0.04, 0.10, -0.07, 0.07],
        ),
        (Mode::Bright, true) => (
            Lch::new(0.98, 0.010, background.h),
            Lch::new(0.15, 0.04, foreground.h),
            0.42,
            (0.10, 0.18),
            [-0.04, 0.01, 0.06, -0.03, -0.02, 0.03],
        ),
        (Mode::Bright, false) => (
            Lch::new(0.10, 0.020, background.h),
            Lch::new(0.92, 0.025, foreground.h),
            0.78,
            (0.10, 0.18),
            [-0.04, 0.01, 0.06, -0.03, -0.02, 0.03],
        ),
        (Mode::HighContrast, true) => (
            Lch::new(0.99, 0.005, background.h),
            Lch::new(0.10, 0.020, foreground.h),
            0.32,
            (0.10, 0.18),
            [-0.03, 0.01, 0.04, -0.02, -0.01, 0.02],
        ),
        (Mode::HighContrast, false) => (
            Lch::new(0.05, 0.010, background.h),
            Lch::new(0.95, 0.015, foreground.h),
            0.82,
            (0.10, 0.18),
            [-0.03, 0.01, 0.04, -0.02, -0.01, 0.02],
        ),
        _ => unreachable!("mode dispatch handles all other palette families"),
    };
    let mut result = [Rgb::default(); 16];
    result[0] = bg.rgb();
    result[7] = fg.rgb();
    for slot in 0..6 {
        let lch = base[slot + 1].lch();
        let c = if mode == Mode::Muted {
            lch.c * 0.5
        } else {
            lch.c
        };
        result[slot + 1] = Lch::new(
            (l + stagger[slot]).clamp(0.05, 0.95),
            c.clamp(chroma.0, chroma.1),
            lch.h,
        )
        .rgb();
    }
    finalize(&mut result);
    if mode == Mode::HighContrast {
        for slot in 1..=6 {
            if result[slot].contrast(result[0]) < 7. {
                result[slot] = boost(result[slot], result[0], 7.);
                result[slot + 8] = bright(result[slot]);
            }
        }
    }
    result
}

fn material(colors: &[Rgb], light: bool) -> Colors {
    let mut result = [Rgb::default(); 16];
    result[0] = if light {
        Rgb(250, 250, 250)
    } else {
        Rgb(18, 18, 18)
    };
    result[7] = if light {
        Rgb(33, 33, 33)
    } else {
        Rgb(255, 255, 255)
    };
    let mut used = vec![false; colors.len()];
    for slot in 0..6 {
        let mut best = 0;
        let mut minimum = f64::INFINITY;
        for (index, color) in colors.iter().enumerate() {
            if !used[index] {
                let value = score(color.lch(), HUES[slot], light);
                if value < minimum {
                    best = index;
                    minimum = value;
                }
            }
        }
        let [h, s, l] = colors[best].hsl();
        result[slot + 1] = Rgb::from_hsl(
            h,
            s.max(35.),
            if light {
                l.clamp(35., 60.)
            } else {
                l.clamp(45., 70.)
            },
        );
        used[best] = true;
    }
    result[8] = if light {
        Rgb(117, 117, 117)
    } else {
        Rgb(158, 158, 158)
    };
    for slot in 1..=6 {
        let [h, s, l] = result[slot].hsl();
        result[slot + 8] = Rgb::from_hsl(
            h,
            (s + 8.).min(100.),
            if light {
                (l - 8.).max(30.)
            } else {
                (l + 8.).min(75.)
            },
        );
    }
    result[15] = if light {
        Rgb(0, 0, 0)
    } else {
        Rgb(255, 255, 255)
    };
    result
}

fn analogous(colors: &[Rgb], light: bool) -> Colors {
    let hue = dominant_hue(colors);
    let darkest = colors
        .iter()
        .map(|c| c.lab().l)
        .fold(f64::INFINITY, f64::min);
    let lightest = colors
        .iter()
        .map(|c| c.lab().l)
        .fold(f64::NEG_INFINITY, f64::max);
    let mut result = [Rgb::default(); 16];
    result[0] = if light {
        Lch::new(0.92_f64.max(lightest), 0.02, hue)
    } else {
        Lch::new(0.16_f64.min(darkest), 0.03, hue)
    }
    .rgb();
    result[7] = if light {
        Lch::new(0.28_f64.min(darkest + 0.05), 0.04, hue)
    } else {
        Lch::new(0.88_f64.max(lightest - 0.05), 0.025, hue)
    }
    .rgb();
    let offsets = [-30., -18., -6., 6., 18., 30.];
    let chromas = [0.10, 0.13, 0.09, 0.14, 0.10, 0.12];
    let stagger = [-0.06, 0.04, -0.04, 0.06, -0.05, 0.05];
    let base: f64 = if light { 0.50 } else { 0.62 };
    for slot in 0..6 {
        let h = (hue + offsets[slot] + 360.) % 360.;
        result[slot + 1] =
            Lch::new((base + stagger[slot]).clamp(0.30, 0.85), chromas[slot], h).rgb();
        result[slot + 9] = Lch::new(
            if light {
                (base + stagger[slot] - 0.10).clamp(0.20, 0.70)
            } else {
                (base + stagger[slot] + 0.10).clamp(0.30, 0.92)
            },
            chromas[slot] + 0.02,
            h,
        )
        .rgb();
    }
    result[8] = comment(result[0]);
    result[15] = if light {
        Lch::new(0.10, 0.04, hue)
    } else {
        Lch::new(0.97, 0.015, hue)
    }
    .rgb();
    result
}

fn scheme(colors: &[Rgb], light: bool, mode: Mode) -> Colors {
    let h = dominant_hue(colors);
    let opposite = (h + 180.) % 360.;
    let (hues, chromas, offsets, base): ([f64; 6], [f64; 6], [f64; 6], f64) = match mode {
        Mode::Complementary => (
            [h - 8., opposite - 8., h, opposite, h + 8., opposite + 8.],
            [0.14, 0.14, 0.11, 0.11, 0.16, 0.16],
            [-0.06, -0.04, 0.04, 0.02, 0.08, 0.06],
            if light { 0.48 } else { 0.65 },
        ),
        Mode::Triadic => (
            [
                h,
                (h + 120.) % 360.,
                (h + 240.) % 360.,
                h,
                (h + 120.) % 360.,
                (h + 240.) % 360.,
            ],
            [0.13, 0.13, 0.13, 0.16, 0.16, 0.16],
            [0.05, 0.05, 0.05, -0.07, -0.07, -0.07],
            if light { 0.48 } else { 0.65 },
        ),
        Mode::SplitComplementary => (
            [
                h,
                (h + 150.) % 360.,
                (h + 210.) % 360.,
                h,
                (h + 150.) % 360.,
                (h + 210.) % 360.,
            ],
            [0.14, 0.13, 0.13, 0.17, 0.16, 0.16],
            [0.05, 0.05, 0.05, -0.06, -0.06, -0.06],
            if light { 0.48 } else { 0.65 },
        ),
        Mode::Tetradic => (
            [
                h,
                (h + 90.) % 360.,
                opposite,
                (h + 270.) % 360.,
                h,
                opposite,
            ],
            [0.12, 0.12, 0.12, 0.12, 0.15, 0.15],
            [0.04, -0.02, 0.04, -0.02, 0.10, 0.10],
            if light { 0.48 } else { 0.65 },
        ),
        Mode::Duotone => (
            [h, opposite, h, opposite, h, opposite],
            [0.09; 6],
            [-0.10, -0.10, 0.04, 0.04, 0.14, 0.14],
            if light { 0.50 } else { 0.62 },
        ),
        _ => unreachable!("only color schemes reach this function"),
    };
    let mut result = [Rgb::default(); 16];
    let (bg, fg) = if mode == Mode::Duotone {
        if light {
            (Lch::new(0.97, 0.008, h), Lch::new(0.20, 0.025, h))
        } else {
            (Lch::new(0.12, 0.018, h), Lch::new(0.90, 0.020, h))
        }
    } else if light {
        (Lch::new(0.96, 0.012, h), Lch::new(0.22, 0.04, h))
    } else {
        (Lch::new(0.13, 0.022, h), Lch::new(0.88, 0.025, h))
    };
    result[0] = bg.rgb();
    result[7] = fg.rgb();
    for slot in 0..6 {
        result[slot + 1] = Lch::new(
            (base + offsets[slot]).clamp(0.30, 0.85),
            chromas[slot],
            (hues[slot] + 360.) % 360.,
        )
        .rgb();
    }
    finalize(&mut result);
    result
}
