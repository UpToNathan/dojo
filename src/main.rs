mod control;
mod frame;
mod geometry;
mod niri_layout;
mod power;
mod shell;
mod state;
mod tray;

use anyhow::bail;
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
        None => return shell::run(None),
        Some("--toggle") => ControlCommand::Toggle,
        Some("--enable") => ControlCommand::Enable,
        Some("--disable") => ControlCommand::Disable,
        Some("--quit") => ControlCommand::Quit,
        Some("--help") | Some("-h") => {
            println!("Usage: dojo [--toggle|--enable|--disable|--quit]");
            return Ok(());
        }
        Some(argument) => {
            bail!("unknown argument {argument:?}; use --toggle, --enable, --disable, or --quit")
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
    }
}
