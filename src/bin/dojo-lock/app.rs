//! The edge: one `Locker` entity owns the model. Every input becomes an
//! `EventKind` passed to `dispatch`, which runs the model, carries out its
//! effects and then projects the result onto windows, frames and one timer.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use dojo::lock::pam::{Report, WorkerId};
use dojo::lock::worker::Helper;
use dojo::lock::{
    Effect, Event, EventKind, ExitReason, LockModel, Now, Output, OutputId, Scene, Settings,
};
use gpui::{
    App, AppContext as _, Bounds, Context, DisplayEvent, DisplayId, Entity, SessionLockEvent, Task,
    WindowBounds, WindowHandle, WindowKind, WindowOptions, point, px, size,
};

use dojo::session::paint::Look;

use crate::host::{self, Workers};
use crate::lifecycle::{self, Marker};
use crate::view::{self, LockView, Painted};

static START: OnceLock<Instant> = OnceLock::new();

pub fn mark_start() {
    START.get_or_init(Instant::now);
}

/// One line on stderr, with milliseconds since the process started. Never a key or a secret.
pub fn log(line: &str) {
    let elapsed = START.get().map_or(0, |start| start.elapsed().as_millis());
    let unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis());
    eprintln!("dojo-lock: {line} ms={elapsed} unix_ms={unix_ms}");
}

pub fn now() -> Now {
    Now {
        mono: Instant::now(),
        unix: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs() as i64),
    }
}

pub struct Options {
    pub settings: Settings,
    /// Decoding since `main` started, so it overlaps GPUI's start-up.
    pub wallpaper: std::thread::JoinHandle<Option<std::sync::Arc<gpui::RenderImage>>>,
    pub helper: Helper,
    pub marker: Option<Marker>,
}

struct LockWindow {
    handle: WindowHandle<LockView>,
    view: Entity<LockView>,
    painted: Painted,
}

pub struct Locker {
    model: LockModel,
    look: Rc<Look>,
    windows: BTreeMap<OutputId, LockWindow>,
    /// Outputs whose window is being opened outside this entity's update.
    opening: BTreeSet<OutputId>,
    /// The one deadline timer, for `model.next_deadline()`.
    wake: Option<(Instant, Task<()>)>,
    workers: Workers,
    marker: Option<Marker>,
    /// The compositor confirmed the lock; only then may the session be unlocked.
    locked: bool,
    /// The `LockedHint = true` call, while logind may still be answering it.
    hint: Option<std::sync::mpsc::Receiver<()>>,
    /// Each window's configured size, which wins over the display's bounds.
    viewports: BTreeMap<OutputId, gpui::Size<gpui::Pixels>>,
    /// Events produced while carrying out effects, run before the next sync.
    queue: VecDeque<EventKind>,
    greeting_stored: bool,
}

/// Runs the GPUI event loop until the model exits or the connection ends.
pub fn run(options: Options) -> u8 {
    gpui_platform::application().run(move |cx: &mut App| {
        if let Err(error) = dojo::power::register_font(cx) {
            eprintln!("dojo-lock: {error:#}");
        }
        view::bind_keys(cx);
        let (reports, received) = async_channel::unbounded::<(WorkerId, Report)>();
        let settings = Settings {
            reduced_motion: options.settings.reduced_motion || cx.reduce_motion(),
            ..options.settings
        };
        let look = Rc::new(host::load_look(options.wallpaper.join().unwrap_or(None)));
        let (model, effects) = LockModel::new(settings, now(), outputs(cx));
        let locker = cx.new(|_| Locker {
            model,
            look,
            windows: BTreeMap::new(),
            opening: BTreeSet::new(),
            wake: None,
            workers: Workers::start(options.helper, reports),
            marker: options.marker,
            locked: false,
            hint: None,
            viewports: BTreeMap::new(),
            queue: VecDeque::new(),
            greeting_stored: false,
        });

        cx.on_session_lock_event({
            let locker = locker.clone();
            move |event, cx| locker.update(cx, |locker, cx| locker.session_lock(event, cx))
        })
        .detach();
        cx.observe_displays({
            let locker = locker.clone();
            move |_: DisplayEvent, cx| {
                locker.update(cx, |locker, cx| {
                    let set = locker.outputs(cx);
                    locker.dispatch(EventKind::Outputs(set), cx)
                });
            }
        })
        .detach();
        cx.spawn({
            let locker = locker.clone();
            async move |cx| {
                while let Ok((id, report)) = received.recv().await {
                    log(&format!(
                        "event=report method={:?} generation={} report={report:?}",
                        id.method, id.generation
                    ));
                    locker.update(cx, |locker, cx| {
                        locker.dispatch(EventKind::Worker(id, report), cx)
                    });
                }
            }
        })
        .detach();
        locker.update(cx, |locker, cx| {
            locker.carry_out(effects, cx);
            locker.sync(cx);
        });
    });
    lifecycle::exit_status()
}

fn outputs(cx: &App) -> Vec<Output> {
    cx.displays()
        .iter()
        .map(|display| Output {
            id: OutputId(u64::from(display.id())),
            size: display.bounds().size,
        })
        .collect()
}

impl Locker {
    /// The only way anything changes. Every input source ends here.
    pub fn dispatch(&mut self, kind: EventKind, cx: &mut Context<Self>) {
        self.queue.push_back(kind);
        self.carry_out(Vec::new(), cx);
        self.sync(cx);
    }

    /// The lock surface for `output` was configured to `size`.
    pub fn viewport(
        &mut self,
        output: OutputId,
        size: gpui::Size<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) {
        if self.viewports.insert(output, size) != Some(size) {
            let set = self.outputs(cx);
            self.dispatch(EventKind::Outputs(set), cx);
        }
    }

    /// The displays, sized by their lock windows where those are configured.
    fn outputs(&self, cx: &App) -> Vec<Output> {
        outputs(cx)
            .into_iter()
            .map(|output| Output {
                size: self
                    .viewports
                    .get(&output.id)
                    .copied()
                    .unwrap_or(output.size),
                ..output
            })
            .collect()
    }

    /// The size the model lays `output` out for. A window paints only when its
    /// own size is this one, so a frame is never laid out for another size.
    pub fn output_size(&self, output: OutputId) -> Option<gpui::Size<gpui::Pixels>> {
        self.model.output_size(output)
    }

    pub fn scene(&self, output: OutputId) -> Option<Scene> {
        self.model.scene(output, now())
    }

    pub fn greeting_stored(&self) -> bool {
        self.greeting_stored
    }

    pub fn greeting_shown(&mut self, text: String) {
        if !std::mem::replace(&mut self.greeting_stored, true) {
            host::store_greeting(text);
        }
    }

    fn session_lock(&mut self, event: SessionLockEvent, cx: &mut Context<Self>) {
        match event {
            SessionLockEvent::Locked => {
                log("event=locked");
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
                self.dispatch(EventKind::Locked, cx);
            }
            SessionLockEvent::Finished => {
                log("event=finished");
                self.dispatch(EventKind::Finished, cx);
            }
        }
    }

    /// Runs queued events through the model and carries out every effect, including those of
    /// events the effects queue (a chips sample), before anything is projected.
    fn carry_out(&mut self, mut effects: Vec<Effect>, cx: &mut Context<Self>) {
        loop {
            for effect in effects {
                self.apply(effect, cx);
            }
            let Some(kind) = self.queue.pop_front() else {
                return;
            };
            effects = self.model.update(Event { now: now(), kind });
        }
    }

    fn apply(&mut self, effect: Effect, cx: &mut Context<Self>) {
        match effect {
            Effect::Spawn(id) => self.workers.spawn(id),
            Effect::Kill(id) => self.workers.kill(id),
            Effect::Answer { to, secret } => self.workers.answer(to, secret),
            Effect::DisplayOff => host::power_off_monitors(),
            Effect::SampleChips => self.queue.push_back(EventKind::Chips(host::sample_chips())),
            Effect::ProbeShellOverlay => {
                let parked = host::shell_overlay_parked();
                log(&format!("event=shell-overlay parked={parked}"));
                self.queue.push_back(EventKind::ShellOverlay(parked));
            }
            Effect::CloseShellSurfaces => host::close_shell_surfaces(),
            // Login mode only; this program never builds a login model.
            Effect::SwapLook(_) => {}
            Effect::Unlock => {
                if !self.locked {
                    eprintln!("dojo-lock: refusing to unlock a session that was never locked");
                    return;
                }
                if let Some(marker) = &self.marker {
                    marker.remove();
                }
                self.clear_hint();
                cx.unlock_session();
                log("event=unlocked");
            }
            Effect::Exit(reason) => {
                if reason == ExitReason::Ended {
                    if let Some(marker) = &self.marker {
                        marker.remove();
                    }
                    self.clear_hint();
                }
                lifecycle::exit_cleanly();
                log(&format!("event=exit reason={reason:?}"));
                cx.quit();
            }
        }
    }

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

    /// Projects the model: a lock window per output, a frame only where the
    /// scene changed, and one timer for the next deadline.
    fn sync(&mut self, cx: &mut Context<Self>) {
        let wanted: BTreeSet<OutputId> = self.model.outputs().collect();
        let missing: Vec<OutputId> = wanted
            .iter()
            .filter(|id| !self.windows.contains_key(id) && !self.opening.contains(id))
            .copied()
            .collect();
        if !missing.is_empty() {
            self.opening.extend(&missing);
            let locker = cx.entity();
            // Opening a window draws it at once, and its view reads this entity.
            cx.defer(move |cx| {
                for id in missing {
                    open_window(&locker, id, cx);
                }
            });
        }
        let stale: Vec<OutputId> = self
            .windows
            .keys()
            .filter(|id| !wanted.contains(id))
            .copied()
            .collect();
        for id in stale {
            if let Some(window) = self.windows.remove(&id) {
                cx.defer(move |cx| {
                    let _ = window
                        .handle
                        .update(cx, |_, window, _| window.remove_window());
                });
            }
        }

        let now = now();
        for (id, window) in &self.windows {
            let scene = self.model.scene(*id, now);
            if window.painted.borrow().as_ref() != scene.as_ref() {
                App::notify(cx, window.view.entity_id());
            }
        }

        let next = self.model.next_deadline();
        if self.wake.as_ref().map(|(at, _)| *at) != next {
            self.wake = next.map(|at| {
                let task = cx.spawn(async move |locker, cx| {
                    cx.background_executor()
                        .timer(at.saturating_duration_since(Instant::now()))
                        .await;
                    let _ = locker.update(cx, |locker, cx| {
                        locker.wake = None;
                        locker.dispatch(EventKind::Timer, cx);
                    });
                });
                (at, task)
            });
        }
    }

    fn opened(&mut self, id: OutputId, window: Option<LockWindow>, cx: &mut Context<Self>) {
        self.opening.remove(&id);
        match window {
            Some(window) if self.model.outputs().any(|output| output == id) => {
                self.windows.insert(id, window);
            }
            Some(window) => {
                cx.defer(move |cx| {
                    let _ = window
                        .handle
                        .update(cx, |_, window, _| window.remove_window());
                });
            }
            None if self.windows.is_empty() && self.opening.is_empty() => {
                // On Linux a quit requested before the event loop starts is lost.
                cx.spawn(async |_, cx| cx.update(|cx| cx.quit())).detach();
            }
            None => {}
        }
        self.sync(cx);
    }
}

fn open_window(locker: &Entity<Locker>, id: OutputId, cx: &mut App) {
    let look = locker.read(cx).look.clone();
    let initial = locker
        .read(cx)
        .output_size(id)
        .unwrap_or_else(|| size(px(800.), px(600.)));
    let painted = Painted::default();
    let mut view = None;
    let result = cx.open_window(
        WindowOptions {
            titlebar: None,
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: point(px(0.), px(0.)),
                size: initial,
            })),
            display_id: Some(DisplayId::new(id.0)),
            app_id: Some("dojo-lock".into()),
            kind: WindowKind::SessionLock,
            ..Default::default()
        },
        |window, cx| {
            let entity =
                cx.new(|cx| LockView::new(id, locker.clone(), look, painted.clone(), window, cx));
            window.focus(&entity.read(cx).focus_handle(), cx);
            view = Some(entity.clone());
            entity
        },
    );
    let window = match (result, view) {
        (Ok(handle), Some(view)) => {
            log(&format!("event=window-opened output={}", id.0));
            Some(LockWindow {
                handle,
                view,
                painted,
            })
        }
        (Err(error), _) => {
            eprintln!(
                "dojo-lock: could not open a lock window on output {}: {error:#}",
                id.0
            );
            None
        }
        (Ok(_), None) => None,
    };
    locker.update(cx, |locker, cx| locker.opened(id, window, cx));
}
