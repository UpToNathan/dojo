mod config;
mod control;
mod frame;
mod geometry;
mod launcher;
mod niri_layout;
mod power;
mod shell;
mod state;
mod theme_picker;
mod tray;
mod wallpaper;

use anyhow::bail;
use dojo::design;
use control::ControlCommand;

fn main() {
    if let Err(error) = run() {
        eprintln!("Dojo failed: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    let mut arguments = std::env::args().skip(1);
    let command = match arguments.next().as_deref() {
        Some("--launcher-benchmark") => {
            if arguments.next().is_some() {
                bail!("--launcher-benchmark takes no arguments");
            }
            return launcher::benchmark();
        }
        None => return shell::run(None),
        Some("--toggle") => ControlCommand::Toggle,
        Some("--enable") => ControlCommand::Enable,
        Some("--disable") => ControlCommand::Disable,
        Some("--quit") => ControlCommand::Quit,
        Some("--launcher") => ControlCommand::Launcher,
        Some("--theme") => ControlCommand::Theme,
        Some("--help") | Some("-h") => {
            println!(
                "Usage: dojo [--toggle|--enable|--disable|--quit|--launcher|--theme|--launcher-benchmark]"
            );
            return Ok(());
        }
        Some(argument) => {
            bail!("unknown argument {argument:?}; use --help")
        }
    };

    if arguments.next().is_some() {
        bail!("Dojo accepts one control argument at a time");
    }

    if control::send(command)? {
        return Ok(());
    }

    match command {
        ControlCommand::Toggle | ControlCommand::Enable => shell::run(Some(true)),
        ControlCommand::Disable => state::persist_frame_enabled(false),
        ControlCommand::Quit => Ok(()),
        ControlCommand::Launcher => shell::run_with_launcher(None, true),
        ControlCommand::Theme => shell::run_with_theme_picker(None),
    }
}
