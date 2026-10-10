use std::path::Path;

use anyhow::{Context as _, bail};
use dojo::extraction::{self, Mode};

fn main() {
    if let Err(error) = run() {
        eprintln!("Palette extraction failed: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        bail!("Usage: dojo-palette IMAGE [--mode MODE] [--light|--dark] | --list-modes");
    };
    if path == "--list-modes" {
        anyhow::ensure!(args.next().is_none(), "--list-modes takes no arguments");
        for mode in Mode::ALL {
            println!("{}", mode.name());
        }
        return Ok(());
    }
    let mut light = false;
    let mut mode = Mode::Normal;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--mode" => mode = args.next().context("--mode requires a value")?.parse()?,
            "--light" => light = true,
            "--dark" => light = false,
            _ => bail!("unknown argument {argument:?}"),
        }
    }
    let palette = extraction::extract(Path::new(&path), light, mode)?;
    let output = serde_json::json!({
        "mode": mode.name(),
        "light": light,
        "colors": palette.hex_colors(),
        "algorithm_source": extraction::UPSTREAM_REVISION,
    });
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}
