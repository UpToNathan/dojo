//! The lock program's side of the shared edge (`dojo::lock::edge`): the PAM workers, logind's
//! `LockedHint`, the takeover marker, and unlocking the session.

use std::rc::Rc;

use dojo::lock::edge::{self, Ctx, Host};
use dojo::lock::pam::{Report, WorkerId};
use dojo::lock::worker::Helper;
use dojo::lock::{Effect, EventKind, ExitReason, LockModel, Now, Settings};
use dojo::session::paint::Look;
use gpui::{App, SessionLockEvent};

use crate::host::{self, Workers};
use crate::lifecycle::Marker;

pub use dojo::lock::edge::mark_start;

const NAME: &str = "dojo-lock";

/// One line on stderr, with milliseconds since the process started. Never a key or a secret.
pub fn log(line: &str) {
    edge::log(NAME, line);
}

pub fn now() -> Now {
    edge::now()
}

pub struct Options {
    pub settings: Settings,
    /// Decoding since `main` started, so it overlaps GPUI's start-up.
    pub wallpaper: std::thread::JoinHandle<Option<std::sync::Arc<gpui::RenderImage>>>,
    pub helper: Helper,
    pub marker: Option<Marker>,
}

struct LockHost {
    look: Rc<Look>,
    workers: Workers,
    marker: Option<Marker>,
    /// The compositor confirmed the lock; only then may the session be unlocked.
    locked: bool,
    /// The `LockedHint = true` call, while logind may still be answering it.
    hint: Option<std::sync::mpsc::Receiver<()>>,
    greeting_stored: bool,
}

/// Runs the GPUI event loop until the model exits or the connection ends.
pub fn run(options: Options) -> u8 {
    gpui_platform::application().run(move |cx: &mut App| {
        if let Err(error) = dojo::power::register_font(cx) {
            eprintln!("dojo-lock: {error:#}");
        }
        edge::bind_keys(cx);
        let (reports, received) = async_channel::unbounded::<(WorkerId, Report)>();
        let settings = Settings {
            reduced_motion: options.settings.reduced_motion || cx.reduce_motion(),
            ..options.settings
        };
        let look = Rc::new(host::load_look(options.wallpaper.join().unwrap_or(None)));
        let (model, effects) = LockModel::new(settings, now(), edge::outputs(cx));
        let lock = LockHost {
            look,
            workers: Workers::start(options.helper, reports),
            marker: options.marker,
            locked: false,
            hint: None,
            greeting_stored: false,
        };
        let locker = edge::start(cx, Box::new(lock), model, effects);
        edge::pump(cx, &locker, received, |(id, report), _| {
            log(&format!(
                "event=report method={:?} generation={} report={report:?}",
                id.method, id.generation
            ));
            vec![EventKind::Worker(id, report)]
        });
    });
    edge::exit_status()
}

impl Host for LockHost {
    fn name(&self) -> &'static str {
        NAME
    }

    fn look(&self, _: Option<&str>) -> Rc<Look> {
        self.look.clone()
    }

    fn session_lock(&mut self, event: &SessionLockEvent) {
        if !matches!(event, SessionLockEvent::Locked) {
            return;
        }
        if !self.locked
            && let Some(marker) = &self.marker
        {
            let since = marker.read().unwrap_or(now().unix);
            if let Err(error) = marker.write(since) {
                eprintln!("dojo-lock: could not write the takeover marker: {error}");
            }
        }
        if !self.locked {
            self.hint = Some(host::set_locked_hint(true));
        }
        self.locked = true;
    }

    fn apply(&mut self, effect: Effect, ctx: Ctx<'_>) {
        match effect {
            Effect::Spawn(id) => self.workers.spawn(id),
            Effect::Kill(id) => self.workers.kill(id),
            Effect::Answer { to, secret } => self.workers.answer(to, secret),
            Effect::ProbeShellOverlay => {
                let parked = host::shell_overlay_parked();
                log(&format!("event=shell-overlay parked={parked}"));
                ctx.queue.push_back(EventKind::ShellOverlay(parked));
            }
            // The login screen's effects; this program never builds a login model.
            Effect::SwapLook(_) | Effect::Power(_) => {}
            Effect::Unlock { .. } => {
                if !self.locked {
                    eprintln!("dojo-lock: refusing to unlock a session that was never locked");
                    return;
                }
                if let Some(marker) = &self.marker {
                    marker.remove();
                }
                self.clear_hint();
                ctx.cx.unlock_session();
                log("event=unlocked");
            }
            Effect::Exit(reason) => {
                if reason == ExitReason::Ended {
                    if let Some(marker) = &self.marker {
                        marker.remove();
                    }
                    self.clear_hint();
                }
                edge::exit_cleanly();
                log(&format!("event=exit reason={reason:?}"));
                ctx.cx.quit();
            }
            // The shared edge carries these out.
            Effect::DisplayOff | Effect::SampleChips | Effect::CloseShellSurfaces => {}
        }
    }

    fn greeting_shown(&mut self, text: String) {
        if !std::mem::replace(&mut self.greeting_stored, true) {
            host::store_greeting(text);
        }
    }

    fn greeting_stored(&self) -> bool {
        self.greeting_stored
    }
}

impl LockHost {
    /// Sets logind's `LockedHint` back to false, if this locker set it. Waits
    /// for the earlier call first so the two cannot cross, and for the answer
    /// for a moment only: the session is about to be released either way.
    fn clear_hint(&mut self) {
        let Some(earlier) = self.hint.take() else {
            return;
        };
        let _ = earlier.recv_timeout(dojo::session::logind::HINT_TIMEOUT);
        let _ = host::set_locked_hint(false).recv_timeout(dojo::session::logind::HINT_TIMEOUT);
    }
}
