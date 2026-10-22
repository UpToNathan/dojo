//! Host services both programs ask of the machine: the corner chips' status and Niri's monitors.
//! Best effort throughout; a failure is logged once and never reaches the model.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::lock::HostChips;

pub fn sample_chips() -> HostChips {
    let status = crate::power::read_status();
    let battery = status.capacity.map(|capacity| {
        let glyph = crate::power::battery_glyph(Some(capacity), status.battery_status.as_deref());
        format!("{glyph} {capacity}%")
    });
    HostChips {
        battery,
        network: Some(network_label()),
    }
}

/// Wired, Wi-Fi or offline from sysfs operstate. No SSID: that needs NetworkManager.
fn network_label() -> String {
    let mut wired = false;
    let mut wireless = false;
    for entry in std::fs::read_dir("/sys/class/net")
        .into_iter()
        .flatten()
        .flatten()
    {
        let path = entry.path();
        if entry.file_name() == "lo" {
            continue;
        }
        let up =
            std::fs::read_to_string(path.join("operstate")).is_ok_and(|state| state.trim() == "up");
        if up && path.join("wireless").exists() {
            wireless = true;
        } else if up {
            wired = true;
        }
    }
    match (wired, wireless) {
        (true, _) => "\u{f0200} Wired".into(),
        (false, true) => "\u{f05a9} Wi-Fi".into(),
        (false, false) => "\u{f05aa} Offline".into(),
    }
}

/// Asks Niri to power the monitors off. Best effort, off the UI thread; the
/// first failure is logged and later ones are not.
pub fn power_off_monitors(program: &'static str) {
    static LOGGED: AtomicBool = AtomicBool::new(false);
    std::thread::spawn(move || {
        let result = niri_ipc::socket::Socket::connect().and_then(|mut socket| {
            socket.send(niri_ipc::Request::Action(
                niri_ipc::Action::PowerOffMonitors {},
            ))
        });
        let failure = match result {
            Ok(Ok(_)) => return,
            Ok(Err(message)) => message,
            Err(error) => error.to_string(),
        };
        if !LOGGED.swap(true, Ordering::Relaxed) {
            eprintln!("{program}: could not power off the monitors: {failure}");
        }
    });
}

/// Closing the launcher, tray menus and pickers belongs to the shell. A later
/// workstream wires this to the shell's control socket; until then the shell's
/// own closing transition is the only thing that hides them, so this does nothing.
pub fn close_shell_surfaces() {}
