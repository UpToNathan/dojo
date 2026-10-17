//! Host services behind the model's effects: theme assets, chips, the PAM
//! worker thread, Niri IPC and the account. Everything here is best effort
//! except the workers, whose failures become model events.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

use dojo::config::{ConfigStore, ThemeColors};
use dojo::lock::pam::{Method, Report, WorkerId};
use dojo::lock::secret::Password;
use dojo::lock::worker::{Helper, Worker};
use dojo::lock::{GreetingSet, HostChips, Now};
use dojo::session::host::{error_color, local_hour};
use dojo::session::{handoff, logind};
use dojo::session::message;
use dojo::session::paint::Look;
pub use dojo::session::host::{Account, decode_wallpaper};
use dojo::session::{Occasion, history};
use gpui::RenderImage;

/// The theme, read once at start. The locker never reloads it.
pub fn load_look(wallpaper: Option<Arc<RenderImage>>) -> Look {
    let colors = match ConfigStore::user_default().and_then(|mut store| store.load()) {
        Ok(Some(colors)) => colors,
        Ok(None) => ThemeColors::default(),
        Err(error) => {
            eprintln!("dojo-lock: using default colours: {error:#}");
            ThemeColors::default()
        }
    };
    Look::new(colors, gpui::rgb(error_color()), wallpaper)
}

pub fn sample_chips() -> HostChips {
    let status = dojo::power::read_status();
    let battery = status.capacity.map(|capacity| {
        let glyph = dojo::power::battery_glyph(Some(capacity), status.battery_status.as_deref());
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
pub fn power_off_monitors() {
    static LOGGED: AtomicBool = AtomicBool::new(false);
    std::thread::spawn(|| {
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
            eprintln!("dojo-lock: could not power off the monitors: {failure}");
        }
    });
}

/// Closing the launcher, tray menus and pickers belongs to the shell. A later
/// workstream wires this to the shell's control socket; until then the shell's
/// own closing transition is the only thing that hides them, so this does nothing.
pub fn close_shell_surfaces() {}

/// The three unlock greetings for this lock, or `None` when greetings are off.
pub fn greetings(enabled: bool, account: &Account, now: Now) -> Option<GreetingSet> {
    if !enabled {
        return None;
    }
    let previous =
        history::default_directory().and_then(|dir| history::load(&dir, Occasion::Unlock));
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.subsec_nanos());
    let base = message::Context {
        hour: local_hour(now.unix),
        previous,
        seed: (now.unix as u64) << 32 | u64::from(nanos),
        ..message::Context::default()
    };
    let name = message::display_name(account.gecos.as_deref(), &account.login);
    Some(GreetingSet::choose(&base, &name))
}

/// The shell's farewell for this lock, read and deleted whether or not it is used.
pub fn take_farewell(enabled: bool) -> Option<String> {
    let directory = handoff::directory()?;
    handoff::take_farewell(&directory, enabled, std::time::SystemTime::now())
}

/// Whether a live shell holds a parked overlay and will draw the unlock opening.
/// A missing, stale or foreign `overlay-ready` means the locker opens by itself.
pub fn shell_overlay_parked() -> bool {
    handoff::directory()
        .is_some_and(|directory| handoff::overlay_live(&directory, std::path::Path::new("/proc")))
}

/// Sets logind's `LockedHint` on a thread of its own, so a slow logind never
/// holds up the lock screen. The first failure is logged and later ones are not.
/// The channel receives once the call is over.
pub fn set_locked_hint(locked: bool) -> mpsc::Receiver<()> {
    static WARNED: AtomicBool = AtomicBool::new(false);
    let (done, finished) = mpsc::channel();
    std::thread::spawn(move || {
        if let Err(error) = logind::set_locked_hint(locked)
            && !WARNED.swap(true, Ordering::SeqCst)
        {
            eprintln!("dojo-lock: could not set logind LockedHint={locked}: {error:#}");
        }
        let _ = done.send(());
    });
    finished
}

/// Remembers the greeting that was shown so the next unlock picks another.
pub fn store_greeting(text: String) {
    std::thread::spawn(move || {
        if let Some(dir) = history::default_directory()
            && let Err(error) = history::store(&dir, Occasion::Unlock, &text)
        {
            eprintln!("dojo-lock: could not store the greeting: {error}");
        }
    });
}

enum Command {
    Spawn(WorkerId),
    Kill(WorkerId),
    Answer(u32, Password),
}

/// The PAM workers, owned by one long-lived thread: their PDEATHSIG fires when
/// the spawning thread exits, and kills block while the tree is reaped.
pub struct Workers {
    commands: mpsc::Sender<Command>,
}

impl Workers {
    pub fn start(helper: Helper, reports: async_channel::Sender<(WorkerId, Report)>) -> Workers {
        let (commands, received) = mpsc::channel();
        std::thread::Builder::new()
            .name("dojo-lock-pam".into())
            .spawn(move || run_workers(helper, received, reports))
            .expect("spawning the PAM worker thread");
        Workers { commands }
    }

    pub fn spawn(&self, id: WorkerId) {
        let _ = self.commands.send(Command::Spawn(id));
    }

    pub fn kill(&self, id: WorkerId) {
        let _ = self.commands.send(Command::Kill(id));
    }

    pub fn answer(&self, to: u32, secret: Password) {
        let _ = self.commands.send(Command::Answer(to, secret));
    }
}

fn run_workers(
    helper: Helper,
    commands: mpsc::Receiver<Command>,
    reports: async_channel::Sender<(WorkerId, Report)>,
) {
    let mut live: Vec<Worker> = Vec::new();
    for command in commands {
        match command {
            Command::Spawn(id) => {
                // The model runs one worker per method, so a new spawn means the last one is done.
                live.retain_mut(|worker| {
                    if worker.id().method != id.method {
                        return true;
                    }
                    worker.kill();
                    false
                });
                match helper.spawn(id, &reports) {
                    Ok(worker) => live.push(worker),
                    Err(error) => {
                        eprintln!(
                            "dojo-lock: could not start the {:?} worker: {error}",
                            id.method
                        );
                        let _ = reports.try_send((id, Report::Crashed));
                    }
                }
            }
            Command::Kill(id) => {
                if let Some(index) = live.iter().position(|worker| worker.id() == id) {
                    let survivors = live.swap_remove(index).kill();
                    if !survivors.is_empty() {
                        eprintln!(
                            "dojo-lock: {:?} worker processes survived SIGKILL: {survivors:?}",
                            id.method
                        );
                    }
                }
            }
            Command::Answer(to, secret) => {
                let target = WorkerId {
                    method: Method::Password,
                    generation: to,
                };
                match live.iter().find(|worker| worker.id() == target) {
                    Some(worker) => {
                        if let Err(error) = worker.answer(secret) {
                            eprintln!("dojo-lock: could not answer the password worker: {error}");
                        }
                    }
                    None => eprintln!("dojo-lock: no password worker {to} to answer"),
                }
            }
        }
    }
}
