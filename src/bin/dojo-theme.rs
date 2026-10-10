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
            "Usage:\n  dojo-theme preview IMAGE OUTPUT.html\n  dojo-theme presets\n  dojo-theme preset ID [--targets LIST]\n  dojo-theme generate IMAGE [--mode MODE]\n  dojo-theme apply IMAGE [--mode MODE] [--appearance system|dark|light] [--targets LIST]\n  dojo-theme select GENERATION [--appearance system|dark|light] [--targets LIST]\n  dojo-theme rollback\n  dojo-theme status\n  dojo-theme watch\nTargets: {}",
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
