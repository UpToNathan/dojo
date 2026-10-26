use std::path::Path;

use anyhow::{Context as _, bail};
use dojo::extraction::Mode;
use dojo::theme::{self, Appearance, Paths, Selection, Theme, ThemeStore};

fn main() {
    if let Err(error) = run() {
        eprintln!("Dojo theme: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "help".into());
    if command == "help" || command == "--help" {
        println!(
            "Usage:\n  dojo-theme preview IMAGE OUTPUT.html\n  dojo-theme generate IMAGE [--mode MODE]\n  dojo-theme apply IMAGE [--mode MODE] [--appearance system|dark|light] [--targets LIST]\n  dojo-theme presets\n  dojo-theme procedural preview STYLE OUTPUT.png [--seed N] [--aspect W/H] [--edge PX] [--colors theme|random | --palette COLORS] [--light]\n  dojo-theme procedural apply STYLE [--seed N] [--layout span|fill] [--colors theme|random | --palette COLORS]\n  dojo-theme preset ID [--targets LIST]\n  dojo-theme select GENERATION [--appearance system|dark|light] [--targets LIST]\n  dojo-theme rollback\n  dojo-theme lock-wallpaper\n  dojo-theme status\n  dojo-theme watch\nTargets: {}",
            theme::TARGETS.join(",")
        );
        return Ok(());
    }
    let store = ThemeStore::new(Paths::user_default()?);
    if command == "presets" {
        anyhow::ensure!(args.next().is_none(), "presets takes no arguments");
        for preset in theme::presets()? {
            println!(
                "{}\t{}\t{}",
                preset.id,
                preset.appearance.name(),
                preset.name
            );
        }
        return Ok(());
    }
    if command == "procedural" {
        return procedural(&store, args);
    }
    if command == "preset" {
        let id = args
            .next()
            .context("preset requires an ID; use presets to list them")?;
        let mut targets = theme::TARGETS
            .iter()
            .map(|target| (*target).to_string())
            .collect::<Vec<_>>();
        while let Some(option) = args.next() {
            anyhow::ensure!(
                option == "--targets",
                "preset appearance comes from the selected variant; expected --targets"
            );
            targets = args
                .next()
                .context("--targets requires a list")?
                .split(',')
                .map(str::to_string)
                .collect();
        }
        let preset = theme::presets()?
            .into_iter()
            .find(|preset| preset.id == id)
            .context("unknown preset ID")?;
        return print_report(theme::apply_preset(&store, &preset, targets)?);
    }
    match command.as_str() {
        "refresh-target" => {
            let target = args.next().context("refresh-target requires a target")?;
            anyhow::ensure!(args.next().is_none(), "refresh-target takes one target");
            println!(
                "{}",
                serde_json::to_string_pretty(&theme::refresh_target(&store, &target)?)?
            );
            return Ok(());
        }
        "preview" => {
            let image = args.next().context("preview requires an image")?;
            let output = args
                .next()
                .context("preview requires an output HTML path")?;
            anyhow::ensure!(args.next().is_none(), "preview takes IMAGE OUTPUT.html");
            theme::preview(Path::new(&image), Path::new(&output))?;
            println!("Preview written to {output}; no theme changes were applied");
            return Ok(());
        }
        "status" => {
            anyhow::ensure!(args.next().is_none(), "status takes no arguments");
            let report = match std::fs::read(store.paths.root.join("last-apply.json")) {
                Ok(bytes) => Some(serde_json::from_slice::<serde_json::Value>(&bytes)?),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(error.into()),
            };
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({"selection":store.selection()?,"last_apply":report})
                )?
            );
            return Ok(());
        }
        "rollback" => {
            anyhow::ensure!(args.next().is_none(), "rollback takes no arguments");
            let previous = store
                .previous()?
                .context("no previous theme application is available")?;
            println!(
                "{}",
                serde_json::to_string_pretty(&theme::apply(&store, previous)?)?
            );
            return Ok(());
        }
        "lock-wallpaper" => {
            anyhow::ensure!(args.next().is_none(), "lock-wallpaper takes no arguments");
            let report = theme::refresh_lock_wallpaper(&store)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            return Ok(());
        }
        "watch" => {
            anyhow::ensure!(args.next().is_none(), "watch takes no arguments");
            return theme::watch(&store);
        }
        "generate" | "apply" | "select" => {}
        _ => bail!("unknown theme command {command:?}; use --help"),
    }
    let input = args
        .next()
        .context("image path or generation id is required")?;
    let mut mode = Mode::Normal;
    let mut appearance = "system".to_string();
    let mut targets = theme::TARGETS
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>();
    while let Some(option) = args.next() {
        match option.as_str() {
            "--mode" => mode = args.next().context("--mode requires a value")?.parse()?,
            "--appearance" => appearance = args.next().context("--appearance requires a value")?,
            "--targets" => {
                targets = args
                    .next()
                    .context("--targets requires a comma-separated list")?
                    .split(',')
                    .map(str::to_string)
                    .collect()
            }
            _ => bail!("unknown option {option:?}"),
        }
    }
    for target in &targets {
        anyhow::ensure!(
            theme::TARGETS.contains(&target.as_str()),
            "unknown target {target}"
        );
    }
    let generation = if command == "select" {
        input
    } else {
        let theme = Theme::generate(Path::new(&input), mode)?;
        store.generate(&theme)?
    };
    if command == "generate" {
        println!("{generation}");
        return Ok(());
    }
    let follow_system = appearance == "system";
    let selected_appearance = if follow_system {
        theme::system_appearance()?
    } else {
        Appearance::parse(&appearance)?
    };
    let report = theme::apply(
        &store,
        Selection {
            generation,
            appearance: selected_appearance,
            follow_system,
            targets,
        },
    )?;
    print_report(report)
}

/// Generated wallpapers: `preview` draws a style to a PNG without touching the desktop, `apply`
/// selects one, keeping the palette, and saves the layout (span unless told otherwise).
fn procedural(store: &ThemeStore, mut args: impl Iterator<Item = String>) -> anyhow::Result<()> {
    use dojo::procedural::{Palette, Recipe, Style, desktop, parse_colors};
    use dojo::theme::wallpaper_layout::WallpaperLayout;

    let action = args.next().context("procedural takes preview or apply")?;
    let style = args.next().context("procedural needs a style")?;
    let style = Style::parse(&style).with_context(|| {
        format!(
            "unknown style {style:?}; expected {}",
            Style::ALL.map(Style::name).join(", ")
        )
    })?;
    let output = match action.as_str() {
        "preview" => Some(args.next().context("preview needs an output PNG path")?),
        "apply" => None,
        other => bail!("unknown procedural action {other:?}"),
    };
    let mut seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos() as u64;
    let mut aspect = None;
    let mut edge = 1920;
    let mut layout = WallpaperLayout::Span;
    let mut colours = None;
    let mut pasted = None;
    let mut light = false;
    while let Some(option) = args.next() {
        let mut value = || args.next().with_context(|| format!("{option} needs a value"));
        match option.as_str() {
            "--seed" => seed = value()?.parse().context("--seed is a whole number")?,
            "--edge" => edge = value()?.parse().context("--edge is a whole number")?,
            "--aspect" => {
                let text = value()?;
                aspect = Some(match text.split_once('/') {
                    Some((width, height)) => width.parse::<f64>()? / height.parse::<f64>()?,
                    None => text.parse()?,
                });
            }
            "--colors" => {
                colours = Some(match value()?.as_str() {
                    "theme" => true,
                    "random" => false,
                    other => bail!("--colors is theme or random, not {other:?}"),
                })
            }
            "--palette" => pasted = Some(value()?),
            "--light" => light = true,
            "--layout" => {
                layout = match value()?.as_str() {
                    "span" => WallpaperLayout::Span,
                    "fill" => WallpaperLayout::Fill,
                    other => bail!("--layout is span or fill, not {other:?}"),
                }
            }
            _ => bail!("unknown option {option:?}"),
        }
    }
    anyhow::ensure!(
        colours.is_none() || pasted.is_none(),
        "--colors and --palette choose the colours two ways; give one"
    );
    let mut palette = match (pasted, colours) {
        (Some(text), _) => Palette::from_colors(&parse_colors(&text))?,
        (None, Some(false)) => Palette::from_seed(seed),
        // By default the wallpaper wears the selected theme's colours, when there is one.
        (None, themed) => match store.selection()? {
            Some(selection) => {
                let theme = store.theme(&selection.generation)?;
                Palette::from_theme(theme.palette(selection.appearance))?
            }
            None if themed == Some(true) => bail!("--colors theme needs a selected theme"),
            None => Palette::from_seed(seed),
        },
    };
    // A theme's own appearance wins; for other colours `--light` asks for the sun.
    palette.light |= light;
    let recipe = Recipe::with_palette(style, seed, palette);
    let outputs = desktop::niri_outputs().unwrap_or_default();
    let (shape, retained_edge) = desktop::retained(&outputs, layout);
    let aspect = aspect.unwrap_or(shape);
    if let Some(output) = output {
        std::fs::write(&output, recipe.reference_png(aspect, edge)?)?;
        println!("{} seed {seed} written to {output}", style.name());
        return Ok(());
    }
    layout.write(&store.paths.root)?;
    println!("{} seed {seed}, layout {}", style.name(), layout.name());
    print_report(theme::apply_procedural(store, recipe, aspect, retained_edge, None)?)
}

fn print_report(report: theme::ApplyReport) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(&report)?);
    if report.targets.iter().any(|t| {
        matches!(
            t.status.as_str(),
            "blocked" | "reload_failed" | "unsupported" | "needs_setup" | "needs_selection"
        )
    }) {
        bail!("theme selected with incomplete integrations; see target results above");
    }
    Ok(())
}
