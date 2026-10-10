use super::{
    Mode,
    color::{Lch, Rgb},
};

mod modes;

type Colors = [Rgb; 16];
const HUES: [f64; 6] = [29.2, 139.1, 111.3, 266.7, 326.4, 194.8];
const ORDER: [usize; 6] = [3, 0, 4, 1, 5, 2];

pub(super) fn generate(colors: &[Rgb], weights: &[f64], light: bool, mode: Mode) -> Colors {
    match mode {
        Mode::Normal if monochrome(colors, weights) => mono(colors, light, 46.),
        Mode::Normal => chromatic(colors, weights, light),
        Mode::Monochromatic => mono(colors, light, 28.),
        _ => modes::generate(colors, light, mode),
    }
}

pub(super) fn normalize(palette: &mut Colors) {
    for slot in 1..=6 {
        if palette[slot].contrast(palette[0]) < 4.5 {
            palette[slot] = boost(palette[slot], palette[0], 4.5);
            palette[slot + 8] = bright(palette[slot]);
        }
    }
}

fn hue_distance(a: f64, b: f64) -> f64 {
    let distance = (a - b).abs();
    if distance > 180. {
        360. - distance
    } else {
        distance
    }
}

fn monochrome(colors: &[Rgb], weights: &[f64]) -> bool {
    let (mut achromatic, mut chromatic, mut sin, mut cos) = (0., 0., 0., 0.);
    for (color, weight) in colors.iter().zip(weights) {
        let lch = color.lch();
        if lch.c < 0.04 {
            achromatic += weight;
        } else {
            chromatic += weight;
            sin += lch.h.to_radians().sin() * weight;
            cos += lch.h.to_radians().cos() * weight;
        }
    }
    if chromatic / (achromatic + chromatic) < 0.06 {
        return true;
    }
    (sin * sin + cos * cos).sqrt() / chromatic > 0.85
}

fn extremity(colors: &[Rgb], weights: &[f64], lightest: bool, used: &[bool]) -> usize {
    let mut best = 0;
    let mut score = f64::NEG_INFINITY;
    for (index, color) in colors.iter().enumerate() {
        if used.get(index).copied().unwrap_or(false) {
            continue;
        }
        let l = color.lab().l;
        let candidate =
            if lightest { l } else { 1. - l } + weights.get(index).copied().unwrap_or(0.) * 0.6;
        if candidate > score {
            best = index;
            score = candidate;
        }
    }
    best
}

fn foreground(
    colors: &[Rgb],
    weights: &[f64],
    light: bool,
    background: Rgb,
    used: &[bool],
) -> (Rgb, Option<usize>) {
    let index = extremity(colors, weights, !light, used);
    let selected = colors[index];
    if selected.contrast(background) >= 7. {
        return (selected, Some(index));
    }
    let mut candidates: Vec<_> = colors
        .iter()
        .enumerate()
        .filter(|(i, c)| !used[*i] && c.contrast(background) > selected.contrast(background))
        .map(|(i, c)| (i, c.contrast(background)))
        .collect();
    candidates.sort_by(|a, b| b.1.total_cmp(&a.1));
    if let Some(&(index, contrast)) = candidates.first() {
        if contrast >= 4.5 {
            return (colors[index], Some(index));
        }
    }
    let mut lab = selected.lab();
    lab.l = if background.lab().l < 0.5 { 0.97 } else { 0.05 };
    (lab.rgb(), None)
}

fn score(lch: Lch, target: f64, light: bool) -> f64 {
    let chroma = if lch.c < 0.035 {
        80.
    } else if lch.c < 0.05 {
        40.
    } else if lch.c < 0.06 {
        15.
    } else if lch.c <= 0.20 {
        0.
    } else {
        (lch.c - 0.20) * 50.
    };
    let lightness = if lch.l < 0.25 {
        (0.25 - lch.l) * 200.
    } else if lch.l > 0.87 {
        (lch.l - 0.87) * 150.
    } else {
        (lch.l - if light { 0.45 } else { 0.60 }).abs() * 20.
    };
    hue_distance(lch.h, target) * 2. + chroma + lightness
}

fn extract_hues(colors: &[Rgb], weights: &[f64], light: bool) -> Colors {
    let top = colors.len().min(12);
    let top_weights = &weights[..weights.len().min(top)];
    let background_index = extremity(&colors[..top], top_weights, light, &[]);
    let background = synthesize_bg(colors[background_index], light, false);
    let mut used = vec![false; colors.len()];
    used[background_index] = true;
    let (foreground, foreground_index) = foreground(colors, weights, light, background, &used);
    if let Some(index) = foreground_index {
        used[index] = true;
    }
    let mut palette = [Rgb::default(); 16];
    palette[0] = background;
    palette[7] = foreground;
    let mut assigned = [false; 6];
    for _ in 0..6 {
        let mut best = None;
        let mut minimum = f64::INFINITY;
        for slot in 0..6 {
            if assigned[slot] {
                continue;
            }
            for (index, color) in colors.iter().enumerate() {
                if used[index] {
                    continue;
                }
                let candidate = score(color.lch(), HUES[slot], light);
                if candidate < minimum {
                    minimum = candidate;
                    best = Some((slot, index));
                }
            }
        }
        if let Some((slot, index)) = best {
            palette[slot + 1] = colors[index];
            assigned[slot] = true;
            used[index] = true;
        }
    }
    palette
}

fn synthesize_bg(color: Rgb, light: bool, mono: bool) -> Rgb {
    let lch = color.lch();
    if light {
        if lch.l >= if mono { 0.75 } else { 0.85 } {
            color
        } else {
            Lch::new(0.94, lch.c.min(0.04), lch.h).rgb()
        }
    } else if lch.l <= if mono { 0.35 } else { 0.20 } {
        color
    } else {
        Lch::new(
            if mono { 0.14 } else { 0.12 },
            lch.c.min(if mono { 0.06 } else { 0.05 }),
            lch.h,
        )
        .rgb()
    }
}

fn bright(color: Rgb) -> Rgb {
    let lch = color.lch();
    if lch.l >= 0.78 {
        Lch::new(
            (lch.l + 0.04).min(0.94),
            (lch.c + 0.04).max(lch.c * 1.3).min(0.32),
            lch.h,
        )
        .rgb()
    } else {
        let boost = ((0.92 - lch.l) * 0.6).min(0.12).max(0.04);
        Lch::new((lch.l + boost).min(0.92), (lch.c * 1.1).min(0.30), lch.h).rgb()
    }
}

fn boost(color: Rgb, background: Rgb, ratio: f64) -> Rgb {
    let mut lch = color.lch();
    let dark = background.lab().l < 0.5;
    while if dark { lch.l < 0.95 } else { lch.l > 0.05 } {
        lch.l += if dark { 0.03 } else { -0.03 };
        let candidate = lch.rgb();
        if candidate.contrast(background) >= ratio {
            return candidate;
        }
    }
    lch.rgb()
}

fn comment(background: Rgb) -> Rgb {
    let bg = background.lch();
    let dark = bg.l < 0.5;
    let mut lightness = if dark {
        (bg.l + 0.30).min(1.)
    } else {
        (bg.l - 0.30).max(0.)
    };
    let mut color = Lch::new(lightness, 0.01, bg.h).rgb();
    while color.contrast(background) < 3.
        && if dark {
            lightness < 0.95
        } else {
            lightness > 0.05
        }
    {
        lightness += if dark { 0.05 } else { -0.05 };
        color = Lch::new(lightness, 0.01, bg.h).rgb();
    }
    color
}

fn finalize(palette: &mut Colors) {
    palette[8] = comment(palette[0]);
    for slot in 1..=6 {
        palette[slot + 8] = bright(palette[slot]);
    }
    palette[15] = bright(palette[7]);
    normalize(palette);
}

fn salient(colors: &[Rgb]) -> f64 {
    let mut chromas: Vec<_> = colors.iter().map(|c| c.lch().c).collect();
    chromas.sort_by(f64::total_cmp);
    chromas[(0.9 * (chromas.len() - 1) as f64).round() as usize]
}

fn chromatic(colors: &[Rgb], weights: &[f64], light: bool) -> Colors {
    let mut palette = extract_hues(colors, weights, light);
    let mut bins = [[0.; 3]; 12];
    for (color, weight) in colors.iter().zip(weights) {
        let lch = color.lch();
        if lch.c < 0.04 {
            continue;
        }
        let bin = (lch.h / 30.) as usize % 12;
        bins[bin][0] += weight;
        bins[bin][1] += lch.h.to_radians().sin() * weight;
        bins[bin][2] += lch.h.to_radians().cos() * weight;
    }
    let clusters: Vec<_> = bins
        .into_iter()
        .filter(|v| v[0] >= 0.025)
        .map(|v| (v[1].atan2(v[2]).to_degrees() + 360.) % 360.)
        .collect();
    let chroma = (salient(colors) * 0.85).clamp(0.05, 0.16);
    for slot in 1..=6 {
        let mut lch = palette[slot].lch();
        let gray = lch.c < 0.03;
        if gray || !clusters.iter().any(|h| hue_distance(lch.h, *h) <= 30.) {
            if let Some(h) = clusters.iter().min_by(|a, b| {
                hue_distance(**a, HUES[slot - 1]).total_cmp(&hue_distance(**b, HUES[slot - 1]))
            }) {
                lch.h = *h;
            } else if gray {
                lch.c = 0.;
            }
        }
        if lch.c > 0. && lch.c < chroma {
            lch.c = chroma;
        }
        palette[slot] = lch.rgb();
    }
    for i in 1..=6 {
        for j in i + 1..=6 {
            if palette[i].lab().distance(palette[j].lab()) < 0.05 {
                let mut lch = palette[j].lch();
                lch.l = if light {
                    (lch.l - 0.10).clamp(0.20, 0.85)
                } else {
                    (lch.l + 0.10).clamp(0.30, 0.92)
                };
                palette[j] = lch.rgb();
            }
        }
    }
    finalize(&mut palette);
    palette
}

fn tint(colors: &[Rgb]) -> Option<f64> {
    let (mut sin, mut cos, mut sum) = (0., 0., 0.);
    for color in colors {
        let lch = color.lch();
        if lch.c > 0.005 {
            sin += lch.h.to_radians().sin() * lch.c;
            cos += lch.h.to_radians().cos() * lch.c;
            sum += lch.c;
        }
    }
    (sum / colors.len() as f64 >= 0.008).then(|| (sin.atan2(cos).to_degrees() + 360.) % 360.)
}

fn readable(background: Rgb, light: bool) -> f64 {
    let mut value = if light { 0.80 } else { 0.20 };
    while if light { value >= 0.05 } else { value <= 0.95 } {
        if Lch::new(value, 0., 0.).rgb().contrast(background) >= 4.5 {
            return value;
        }
        value += if light { -0.01 } else { 0.01 };
    }
    if light { 0.45 } else { 0.55 }
}

fn headroom(background: Rgb, light: bool) -> Rgb {
    let lch = background.lch();
    if light && readable(background, true) < 0.42 {
        Lch::new(0.95, lch.c.min(0.04), lch.h).rgb()
    } else if !light && readable(background, false) > 0.60 {
        Lch::new(0.13, lch.c.min(0.06), lch.h).rgb()
    } else {
        background
    }
}

fn ramp(background: Rgb, light: bool) -> [f64; 6] {
    let start = readable(background, light) + if light { -0.02 } else { 0.02 };
    let step = if light {
        -0.072_f64.max((start - 0.16) / 5.)
    } else {
        0.072_f64.max((0.92 - start) / 5.)
    };
    let mut values = [0.; 6];
    for (position, slot) in ORDER.into_iter().enumerate() {
        values[slot] = (start + step * position as f64).clamp(0.06, 0.97);
    }
    values
}

fn mono(colors: &[Rgb], light: bool, arc: f64) -> Colors {
    let (darkest, lightest) =
        colors
            .iter()
            .copied()
            .fold((colors[0], colors[0]), |(darkest, lightest), color| {
                (
                    if color.lab().l < darkest.lab().l {
                        color
                    } else {
                        darkest
                    },
                    if color.lab().l > lightest.lab().l {
                        color
                    } else {
                        lightest
                    },
                )
            });
    let mut palette = [Rgb::default(); 16];
    let hue = tint(colors);
    if let Some(hue) = hue {
        palette[0] = headroom(
            synthesize_bg(if light { lightest } else { darkest }, light, true),
            light,
        );
        palette[7] = if light { darkest } else { lightest };
        if palette[0].contrast(palette[7]) < 7. {
            let mut lab = palette[7].lab();
            lab.l = if palette[0].lab().l < 0.5 { 0.97 } else { 0.05 };
            palette[7] = lab.rgb();
        }
        let chromas = [0.09, 0.11, 0.13, 0.10, 0.12, 0.14];
        let brights = [0.11, 0.14, 0.16, 0.12, 0.15, 0.17];
        let factor = (salient(colors) / (chromas.iter().sum::<f64>() / 6.)).clamp(0.4, 1.);
        let lightness = ramp(palette[0], light);
        for slot in 0..6 {
            let offset = (HUES[slot] - hue + 540.) % 360. - 180.;
            let h = (hue + offset / 180. * arc + 360.) % 360.;
            palette[slot + 1] = Lch::new(lightness[slot], chromas[slot] * factor, h).rgb();
            palette[slot + 9] = Lch::new(
                (lightness[slot] + if light { -0.07 } else { 0.07 }).clamp(0.18, 0.96),
                brights[slot] * factor,
                h,
            )
            .rgb();
        }
        palette[15] = if light {
            Lch::new(0.08, 0.03, hue)
        } else {
            Lch::new(0.97, 0.015, hue)
        }
        .rgb();
    } else {
        palette[0] = headroom(
            if light {
                Lch::new(0.94_f64.max(lightest.lab().l), 0., 0.)
            } else {
                Lch::new(0.14_f64.min(darkest.lab().l), 0., 0.)
            }
            .rgb(),
            light,
        );
        palette[7] = if light {
            Lch::new(0.22_f64.min(darkest.lab().l), 0., 0.)
        } else {
            Lch::new(0.88_f64.max(lightest.lab().l), 0., 0.)
        }
        .rgb();
        let lightness = ramp(palette[0], light);
        for slot in 0..6 {
            palette[slot + 1] = Lch::new(lightness[slot], 0., 0.).rgb();
            palette[slot + 9] = Lch::new(
                (lightness[slot] + if light { -0.06 } else { 0.06 }).clamp(0.05, 0.97),
                0.,
                0.,
            )
            .rgb();
        }
        palette[15] = Lch::new(if light { 0.04 } else { 0.99 }, 0., 0.).rgb();
    }
    palette[8] = comment(palette[0]);
    palette
}
