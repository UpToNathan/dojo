mod audio;
mod bluetooth;
mod bluetooth_ui;
mod clock;
mod displays_ui;
mod edge_reservation;
mod file_events;
mod frame;
mod launcher;
mod model_picker;
mod niri_layout;
mod picker_menu;
mod quick_terminal;
mod session_overlay;
mod shell;
mod sign_in;
mod startup;
mod state;
mod system_menu;
mod theme_picker;
mod tray;
mod wallpaper;
mod wallpaper_cache;
mod wifi;
mod wifi_ui;

use anyhow::bail;
use control::ControlCommand;
use dojo::{config, control, design, geometry, power, speech};

fn main() {
    if let Err(error) = run() {
        eprintln!("Dojo failed: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    startup::begin();
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
        Some("--launcher-up") => ControlCommand::LauncherNavigation(launcher::Navigation::Up),
        Some("--launcher-down") => ControlCommand::LauncherNavigation(launcher::Navigation::Down),
        Some("--launcher-activate") => {
            ControlCommand::LauncherNavigation(launcher::Navigation::Activate)
        }
        Some("--launcher-dismiss") => {
            ControlCommand::LauncherNavigation(launcher::Navigation::Dismiss)
        }
        Some("--theme") => ControlCommand::Theme,
        Some("--displays") => ControlCommand::Displays,
        Some("--wifi") => ControlCommand::Wifi,
        Some("--system-menu") => ControlCommand::SystemMenu,
        Some("--models") => ControlCommand::Models,
        Some("--terminal") => ControlCommand::Terminal,
        Some("--terminal-closed") => ControlCommand::TerminalClosed,
        Some("--leave") => {
            let word = arguments
                .next()
                .ok_or_else(|| anyhow::anyhow!("missing occasion"))?;
            let departure = dojo::session::action::Departure::parse(&word).ok_or_else(|| {
                anyhow::anyhow!(
                    "unknown occasion {word:?}; use lock, logout, suspend, suspend-then-hibernate, hibernate, restart or shutdown"
                )
            })?;
            ControlCommand::Leave(departure)
        }
        Some("--farewell") => {
            let word = arguments
                .next()
                .ok_or_else(|| anyhow::anyhow!("missing occasion"))?;
            let occasion =
                dojo::session::action::Departure::parse_farewell(&word).ok_or_else(|| {
                    anyhow::anyhow!(
                        "unknown occasion {word:?}; use logout, suspend, hibernate, restart or shutdown"
                    )
                })?;
            ControlCommand::Farewell(occasion)
        }
        Some("--speech-state") => {
            let phase = arguments
                .next()
                .ok_or_else(|| anyhow::anyhow!("missing speech state"))?;
            let phase = serde_json::from_value(serde_json::Value::String(phase))?;
            ControlCommand::Speech(speech::SpeechEvent::Phase(phase))
        }
        Some(flag @ ("--speech-response" | "--speech-transcript")) => {
            let text = arguments
                .next()
                .ok_or_else(|| anyhow::anyhow!("missing speech text"))?;
            let event = if flag == "--speech-response" {
                speech::SpeechEvent::Response(text)
            } else {
                speech::SpeechEvent::Transcript(text)
            };
            event.validate()?;
            ControlCommand::Speech(event)
        }
        Some("--help") | Some("-h") => {
            println!(
                "Usage: dojo [--toggle|--enable|--disable|--quit|--launcher|--launcher-up|--launcher-down|--launcher-activate|--launcher-dismiss|--theme|--displays|--wifi|--system-menu|--models|--terminal|--leave OCCASION|--farewell OCCASION|--launcher-benchmark|--speech-state STATE|--speech-response TEXT|--speech-transcript TEXT]"
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

    if control::send(&command)? {
        return Ok(());
    }

    match command {
        ControlCommand::Toggle | ControlCommand::Enable => shell::run(Some(true)),
        ControlCommand::Disable => state::persist_frame_enabled(false),
        ControlCommand::Quit => Ok(()),
        ControlCommand::Launcher => shell::run_with_launcher(None, true),
        ControlCommand::LauncherNavigation(_) => {
            bail!("start Dojo and open the launcher before navigating it")
        }
        ControlCommand::Theme => shell::run_with_theme_picker(None),
        ControlCommand::Models => shell::run_with_models(None),
        ControlCommand::Displays | ControlCommand::Wifi | ControlCommand::SystemMenu => {
            bail!("start Dojo before opening shell settings")
        }
        ControlCommand::Terminal | ControlCommand::TerminalClosed => {
            bail!("start Dojo before using its quick terminal")
        }
        ControlCommand::Leave(departure) => session_overlay::leave_without_shell(departure),
        ControlCommand::Farewell(_) => Ok(()),
        ControlCommand::Speech(_) => bail!("start Dojo before sending speech events"),
    }
}
