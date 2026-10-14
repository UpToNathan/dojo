//! Experimental wallpaper recipe. Standard OKLab math is independent of Aether's port.
use super::{Appearance, Rgb, SemanticPalette};
use image::ImageDecoder as _;
use serde::Serialize;
use std::collections::BTreeMap;

mod modes;
use crate::extraction::Mode;
use modes::Style;

pub const REVISION: &str = "dojo-semantic-wallpaper-2";
const FEEDBACK: [(&str, usize, f64, f64); 4] = [
    ("error", 0, 10., 35.),
    ("warning", 2, 55., 95.),
    ("success", 1, 125., 165.),
    ("info", 5, 205., 260.),
];

#[derive(Serialize)]
pub struct FeedbackProposals {
    pub dark: BTreeMap<String, String>,
    pub light: BTreeMap<String, String>,
}

#[derive(Serialize)]
pub struct RefinedTheme {
    pub revision: &'static str,
    pub mode: Mode,
    pub feedback_proposals: FeedbackProposals,
    pub feedback_ranges: BTreeMap<String, [f64; 2]>,
    pub source_hue: f64,
    pub source_chroma: f64,
    pub dark: SemanticPalette,
    pub light: SemanticPalette,
}

pub fn derive_wallpaper(bytes: &[u8]) -> anyhow::Result<RefinedTheme> {
    derive_wallpaper_mode(bytes, Mode::Normal)
}

pub fn derive_wallpaper_mode(bytes: &[u8], mode: Mode) -> anyhow::Result<RefinedTheme> {
    let (h, c) = source(bytes)?;
    make_theme(h, c, mode)
}

pub fn derive_wallpaper_modes(bytes: &[u8]) -> anyhow::Result<Vec<RefinedTheme>> {
    let (h, c) = source(bytes)?;
    Mode::ALL
        .into_iter()
        .map(|mode| make_theme(h, c, mode))
        .collect()
}

fn source(bytes: &[u8]) -> anyhow::Result<(f64, f64)> {
    anyhow::ensure!(bytes.len() <= 50 * 1024 * 1024, "wallpaper exceeds 50 MiB");
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    let mut decoder = reader.into_decoder()?;
    let (width, height) = decoder.dimensions();
    anyhow::ensure!(
        u64::from(width) * u64::from(height) <= super::MAX_WALLPAPER_PIXELS,
        "wallpaper exceeds pixel limit"
    );
    let orientation = decoder.orientation()?;
    let mut image = image::DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    let sample = image.thumbnail(256, 256).to_rgba8();
    let mut bins = [(0., 0., 0., 0.); 36];
    let mut opaque = 0;
    for pixel in sample.pixels() {
        if pixel[3] < 128 {
            continue;
        }
        opaque += 1;
        let (l, c, h) = oklch(Rgb(pixel[0], pixel[1], pixel[2]));
        if c < 0.015 || !(0.15..=0.9).contains(&l) {
            continue;
        }
        let weight = c * c;
        let bin = &mut bins[(h / 10.).floor() as usize % 36];
        bin.0 += weight;
        bin.1 += h.to_radians().cos() * weight;
        bin.2 += h.to_radians().sin() * weight;
        bin.3 += c * weight;
    }
    anyhow::ensure!(opaque > 0, "wallpaper has no opaque pixels");
    let strongest = bins.into_iter().max_by(|a, b| a.0.total_cmp(&b.0)).unwrap();
    let hue = if strongest.0 > 0. {
        strongest.2.atan2(strongest.1).to_degrees().rem_euclid(360.)
    } else {
        250.
    };
    let chroma = if strongest.0 > 0. {
        strongest.3 / strongest.0
    } else {
        0.
    };
    Ok((hue, chroma))
}

fn make_theme(hue: f64, chroma: f64, mode: Mode) -> anyhow::Result<RefinedTheme> {
    let style = Style::new(mode, hue, chroma);
    let (dark, dark_proposals) = palette(&style, Appearance::Dark);
    let (light, light_proposals) = palette(&style, Appearance::Light);
    validate(&dark)?;
    validate(&light)?;
    Ok(RefinedTheme {
        revision: REVISION,
        mode,
        source_hue: hue,
        source_chroma: chroma,
        feedback_proposals: FeedbackProposals {
            dark: dark_proposals,
            light: light_proposals,
        },
        feedback_ranges: FEEDBACK
            .into_iter()
            .map(|(role, _, low, high)| (role.to_string(), [low, high]))
            .collect(),
        dark,
        light,
    })
}

fn palette(style: &Style, appearance: Appearance) -> (SemanticPalette, BTreeMap<String, String>) {
    let dark = appearance == Appearance::Dark;
    let h = style.hue;
    let (canvas_l, panel_l, raised_l, text_l, muted_l, color_l) = if dark {
        (0.16, 0.21, 0.26, 0.94, 0.73, 0.76 + style.lightness_offset)
    } else {
        (
            0.91,
            0.955,
            0.985,
            0.23,
            0.43,
            0.43 - style.lightness_offset,
        )
    };
    let tint = style.tint;
    let accent_chroma = style.chroma;
    let canvas = rgb(canvas_l, tint, h);
    let panel = rgb(panel_l, tint * 0.9, h);
    let raised = rgb(raised_l, tint * 0.7, h);
    let surfaces = [canvas, panel, raised];
    let text = fit(text_l, 0.012, h, &surfaces, 7.);
    let muted = fit(muted_l, 0.02, h, &surfaces, 4.5);
    let accent = fit(color_l, accent_chroma, h, &surfaces, style.contrast);
    let secondary = fit(
        color_l,
        accent_chroma * 0.85,
        style.secondary,
        &surfaces,
        4.5,
    );
    let hover = rgb(if dark { 0.30 } else { 0.87 }, tint * 1.4, h);
    let selected = rgb(if dark { 0.38 } else { 0.79 }, accent_chroma * 0.43, h);
    let selected_text = fit(text_l, 0.012, h, &[selected], 4.5);
    let mut colors = BTreeMap::new();
    for (role, value) in [
        ("background", canvas),
        ("surface", panel),
        ("surface_raised", raised),
        ("foreground", text),
        ("foreground_muted", muted),
        (
            "foreground_disabled",
            rgb(if dark { 0.53 } else { 0.62 }, 0.012, h),
        ),
        ("background_hover", hover),
        ("background_highlight", selected),
        ("selection_foreground", selected_text),
        ("hover_foreground", fit(text_l, 0.012, h, &[hover], 4.5)),
        ("primary", accent),
        ("secondary", secondary),
        (
            "primary_foreground",
            fit(if dark { 0.16 } else { 0.99 }, 0.01, h, &[accent], 4.5),
        ),
        ("border", rgb(if dark { 0.62 } else { 0.52 }, 0.025, h)),
        (
            "focus",
            fit(color_l, accent_chroma, h, &[canvas, panel, selected], 3.),
        ),
    ] {
        colors.insert(role.to_string(), value.hex());
    }
    let mut proposals = BTreeMap::new();
    for (role, slot, low, high) in FEEDBACK {
        let proposed = rgb(color_l, style.chroma, style.hues[slot]);
        let hue = project_hue(style.hues[slot], low, high);
        let chroma = style.chroma.clamp(0.045, 0.20);
        let fill = bounded_rgb(
            if dark { 0.25 } else { 0.91 },
            (chroma * 0.28).clamp(0.01, 0.045),
            hue,
            (low, high),
            0.005,
        )
        .expect("feedback fill must retain its semantic hue");
        let foreground = fit_semantic(
            color_l,
            chroma,
            hue,
            (low, high),
            &[canvas, panel, raised, fill],
            style.contrast,
        );
        proposals.insert(role.into(), proposed.hex());
        colors.insert(role.into(), foreground.hex());
        colors.insert(format!("{role}_background"), fill.hex());
    }
    colors.insert(
        "diff_added_background".into(),
        colors["success_background"].clone(),
    );
    colors.insert(
        "diff_removed_background".into(),
        colors["error_background"].clone(),
    );
    for (token, slot) in [
        ("keyword", 4),
        ("string", 1),
        ("number", 2),
        ("type", 5),
        ("operator", 3),
    ] {
        colors.insert(
            format!("syntax_{token}"),
            fit(
                color_l,
                style.chroma * 0.85,
                style.hues[slot],
                &surfaces,
                4.5,
            )
            .hex(),
        );
    }
    for (token, value) in [
        ("function", accent),
        ("variable", text),
        ("comment", muted),
        ("punctuation", muted),
    ] {
        colors.insert(format!("syntax_{token}"), value.hex());
    }
    let mut terminal = [canvas; 16];
    terminal[7] = text;
    terminal[8] = muted;
    terminal[15] = text;
    for (i, (low, high)) in [
        (10., 35.),
        (125., 165.),
        (55., 95.),
        (205., 260.),
        (285., 345.),
        (175., 205.),
    ]
    .into_iter()
    .enumerate()
    {
        let hue = project_hue(style.hues[i], low, high);
        let c = style.chroma.clamp(0.045, 0.20);
        terminal[i + 1] = fit_semantic(color_l, c, hue, (low, high), &surfaces, style.contrast);
        terminal[i + 9] = fit_semantic(
            if dark { color_l + 0.06 } else { color_l - 0.06 },
            c,
            hue,
            (low, high),
            &surfaces,
            style.contrast,
        );
    }
    (
        SemanticPalette {
            appearance,
            authored: false,
            colors,
            terminal: terminal.map(Rgb::hex),
        },
        proposals,
    )
}

fn project_hue(h: f64, low: f64, high: f64) -> f64 {
    if (low..=high).contains(&h) {
        return h;
    }
    let distance = |a: f64, b: f64| ((a - b + 180.).rem_euclid(360.) - 180.).abs();
    if distance(h, low) <= distance(h, high) {
        low
    } else {
        high
    }
}

fn bounded_rgb(l: f64, c: f64, h: f64, range: (f64, f64), minimum_chroma: f64) -> Option<Rgb> {
    let (low, high) = range;
    let nearest = project_hue(h, low, high);
    for step in 0..=100 {
        let hue = nearest + ((low + high) * 0.5 - nearest) * f64::from(step) / 100.;
        let candidate = rgb(l, c, hue);
        let (_, actual_chroma, actual_hue) = oklch(candidate);
        if (low..=high).contains(&actual_hue) && actual_chroma >= minimum_chroma {
            return Some(candidate);
        }
    }
    None
}

fn fit_semantic(
    preferred: f64,
    chroma: f64,
    hue: f64,
    range: (f64, f64),
    backgrounds: &[Rgb],
    ratio: f64,
) -> Rgb {
    fit_candidates(preferred, backgrounds, ratio, |l| {
        bounded_rgb(l, chroma, hue, range, 0.025)
    })
}

fn fit(preferred: f64, chroma: f64, hue: f64, backgrounds: &[Rgb], ratio: f64) -> Rgb {
    fit_candidates(preferred, backgrounds, ratio, |l| Some(rgb(l, chroma, hue)))
}

fn fit_candidates(
    preferred: f64,
    backgrounds: &[Rgb],
    ratio: f64,
    candidate: impl Fn(f64) -> Option<Rgb>,
) -> Rgb {
    let mut choices = (0..=100).map(|i| f64::from(i) / 100.).collect::<Vec<_>>();
    choices.sort_by(|a, b| (a - preferred).abs().total_cmp(&(b - preferred).abs()));
    choices
        .into_iter()
        .filter_map(candidate)
        .find(|color| backgrounds.iter().all(|bg| color.contrast(*bg) >= ratio))
        .expect("controlled surfaces must admit a readable foreground")
}

fn validate(p: &SemanticPalette) -> anyhow::Result<()> {
    p.validate()?;
    for (role, _, low, high) in FEEDBACK {
        for suffix in ["", "_background"] {
            let (_, _, hue) = oklch(Rgb::parse(p.color(&format!("{role}{suffix}")))?);
            anyhow::ensure!(
                (low..=high).contains(&hue),
                "{role}{suffix} lost its semantic hue"
            );
        }
    }
    for role in [
        "foreground",
        "foreground_muted",
        "primary",
        "secondary",
        "error",
        "warning",
        "success",
        "info",
    ] {
        for surface in ["background", "surface", "surface_raised"] {
            anyhow::ensure!(
                Rgb::parse(p.color(role))?.contrast(Rgb::parse(p.color(surface))?) >= 4.5,
                "{role} fails contrast on {surface}"
            );
        }
    }
    for (fg, bg) in [
        ("primary_foreground", "primary"),
        ("selection_foreground", "background_highlight"),
        ("hover_foreground", "background_hover"),
        ("error", "error_background"),
        ("warning", "warning_background"),
        ("success", "success_background"),
        ("info", "info_background"),
    ] {
        anyhow::ensure!(
            Rgb::parse(p.color(fg))?.contrast(Rgb::parse(p.color(bg))?) >= 4.5,
            "{fg}/{bg} fails contrast"
        );
    }
    Ok(())
}

fn linear(x: u8) -> f64 {
    let x = f64::from(x) / 255.;
    if x <= 0.04045 {
        x / 12.92
    } else {
        ((x + 0.055) / 1.055).powf(2.4)
    }
}
fn oklch(rgb: Rgb) -> (f64, f64, f64) {
    let (r, g, b) = (linear(rgb.0), linear(rgb.1), linear(rgb.2));
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    let light = 0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s;
    let a = 1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s;
    let b = 0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s;
    (light, a.hypot(b), b.atan2(a).to_degrees().rem_euclid(360.))
}
fn rgb(l: f64, mut c: f64, h: f64) -> Rgb {
    loop {
        let a = c * h.to_radians().cos();
        let b = c * h.to_radians().sin();
        let ll = (l + 0.3963377774 * a + 0.2158037573 * b).powi(3);
        let mm = (l - 0.1055613458 * a - 0.0638541728 * b).powi(3);
        let ss = (l - 0.0894841775 * a - 1.2914855480 * b).powi(3);
        let channels = [
            4.0767416621 * ll - 3.3077115913 * mm + 0.2309699292 * ss,
            -1.2684380046 * ll + 2.6097574011 * mm - 0.3413193965 * ss,
            -0.0041960863 * ll - 0.7034186147 * mm + 1.7076147010 * ss,
        ];
        if channels.iter().all(|x| (-0.000001..=1.000001).contains(x)) || c < 0.00001 {
            let [r, g, b] = channels.map(|x| {
                let x = x.clamp(0., 1.);
                let x = if x <= 0.0031308 {
                    12.92 * x
                } else {
                    1.055 * x.powf(1. / 2.4) - 0.055
                };
                (x * 255.).round() as u8
            });
            return Rgb(r, g, b);
        }
        c *= 0.9;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn image_accents_change_while_status_hues_survive_and_gray_stays_neutral() -> anyhow::Result<()>
    {
        let mut accents = Vec::new();
        for pixel in [[220, 40, 35, 255], [30, 180, 70, 255], [120, 120, 120, 255]] {
            let mut bytes = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(8, 8, image::Rgba(pixel)))
                .write_to(&mut bytes, image::ImageFormat::Png)?;
            let theme = derive_wallpaper(bytes.get_ref())?;
            for palette in [&theme.dark, &theme.light] {
                let (_, chroma, hue) = oklch(Rgb::parse(palette.color("error"))?);
                assert!((10. ..=35.).contains(&hue) && chroma > 0.025);
                let (_, chroma, hue) = oklch(Rgb::parse(palette.color("warning"))?);
                assert!((55. ..=95.).contains(&hue) && chroma > 0.025);
                if pixel[0] == pixel[1] && pixel[1] == pixel[2] {
                    let Rgb(r, g, b) = Rgb::parse(palette.color("primary"))?;
                    assert_eq!((r, g), (g, b));
                }
            }
            accents.push(theme.dark.color("primary").to_string());
        }
        assert_ne!(accents[0], accents[1]);
        Ok(())
    }
    #[test]
    fn all_modes_keep_semantic_ranges_and_contrast_for_every_wallpaper_hue() -> anyhow::Result<()> {
        for h in (0..360).step_by(15) {
            for mode in Mode::ALL {
                let theme = make_theme(f64::from(h), 0.12, mode)?;
                for p in [&theme.dark, &theme.light] {
                    for (role, _, low, high) in FEEDBACK {
                        for suffix in ["", "_background"] {
                            let (_, c, actual) =
                                oklch(Rgb::parse(p.color(&format!("{role}{suffix}")))?);
                            assert!(
                                (low..=high).contains(&actual),
                                "{mode:?} {role}{suffix}: {actual}"
                            );
                            if suffix.is_empty() {
                                assert!(c > 0.025, "{role} lost color");
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
    #[test]
    fn modes_change_fitted_feedback_without_losing_semantics() -> anyhow::Result<()> {
        let mono = make_theme(155., 0.12, Mode::Monochromatic)?;
        let pastel = make_theme(155., 0.12, Mode::Pastel)?;
        let ocean = make_theme(155., 0.12, Mode::Ocean)?;
        assert_ne!(mono.dark.color("warning"), pastel.dark.color("warning"));
        assert_ne!(ocean.dark.color("error"), pastel.dark.color("error"));
        assert_ne!(
            ocean.light.color("background"),
            pastel.light.color("background")
        );
        Ok(())
    }
    #[test]
    fn standard_oklab_round_trips_primary_colors() {
        for color in [
            Rgb(255, 0, 0),
            Rgb(0, 255, 0),
            Rgb(0, 0, 255),
            Rgb(255, 255, 255),
            Rgb(0, 0, 0),
        ] {
            let (l, c, h) = oklch(color);
            assert_eq!(rgb(l, c, h), color);
        }
    }
}
