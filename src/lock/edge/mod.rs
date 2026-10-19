//! The edge both `dojo-lock` and `dojo-greeter` share: one [`Locker`] entity owns the model.
//! Every input becomes an `EventKind` passed to `dispatch`, which runs the model, carries out its
//! effects and then projects the result onto windows, frames and one timer.
//!
//! What differs between the programs is a [`Host`]: where looks come from and what the model's
//! effects mean (PAM workers and `unlock_session` for the lock screen, the display manager's
//! socket for the login screen). Everything else is here once.

mod system;
mod view;

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::rc::Rc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use gpui::{
    App, AppContext as _, Bounds, Context, DisplayEvent, DisplayId, Entity, SessionLockEvent, Task,
    WindowBounds, WindowHandle, WindowKind, WindowOptions, point, px, size,
};

use super::{Effect, Event, EventKind, LockModel, Now, Output, OutputId, Scene};
use crate::session::paint::Look;
use view::{LockView, Painted};

pub use view::{bind_keys, bind_login_keys};

static START: OnceLock<Instant> = OnceLock::new();

/// Starts the clock the log lines count from. Call it first thing in `main`.
pub fn mark_start() {
    START.get_or_init(Instant::now);
}

/// One line on stderr, with milliseconds since the process started. Never a key or a secret.
pub fn log(program: &str, line: &str) {
    let elapsed = START.get().map_or(0, |start| start.elapsed().as_millis());
    let unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis());
    eprintln!("{program}: {line} ms={elapsed} unix_ms={unix_ms}");
}

pub fn now() -> Now {
    Now {
        mono: Instant::now(),
        unix: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs() as i64),
    }
}

/// `EX_TEMPFAIL`: the event loop ended without the model deciding to exit, so the session may
/// still be locked (or the login screen unanswered) and a supervisor should start the program
/// again. A lost Wayland connection ends the loop this way.
pub const ENDED_UNEXPECTEDLY: u8 = 75;

static EXIT: AtomicU8 = AtomicU8::new(ENDED_UNEXPECTEDLY);

/// Called for the model's `Exit`: a clean end, whatever its reason.
pub fn exit_cleanly() {
    EXIT.store(0, Ordering::SeqCst);
}

pub fn exit_status() -> u8 {
    EXIT.load(Ordering::SeqCst)
}

/// What a [`Host`] may touch while it carries out an effect.
pub struct Ctx<'a> {
    /// Events the effect produces, run through the model before anything is projected.
    pub queue: &'a mut VecDeque<EventKind>,
    /// The model as the effect left it, for `choice()`.
    pub model: &'a LockModel,
    pub cx: &'a mut App,
}

/// The parts of the edge that belong to one program.
pub trait Host: 'static {
    /// The program's name: the prefix of its log lines and its windows' app id.
    fn name(&self) -> &'static str;
    /// The look to paint for `login` on the login screen, or the one look of the lock screen
    /// for `None`. An unknown user gets the default look.
    fn look(&self, login: Option<&str>) -> Rc<Look>;
    /// The compositor confirmed or ended the session lock.
    fn session_lock(&mut self, event: &SessionLockEvent);
    /// Carries out every effect the shared edge does not: workers, unlocking, exiting.
    fn apply(&mut self, effect: Effect, ctx: Ctx<'_>);
    /// A greeting was painted for the first time.
    fn greeting_shown(&mut self, text: String);
    /// Nothing more to learn from painting greetings.
    fn greeting_stored(&self) -> bool;
}

struct LockWindow {
    handle: WindowHandle<LockView>,
    view: Entity<LockView>,
    painted: Painted,
}

pub struct Locker {
    model: LockModel,
    host: Box<dyn Host>,
    windows: BTreeMap<OutputId, LockWindow>,
    /// Outputs whose window is being opened outside this entity's update.
    opening: BTreeSet<OutputId>,
    /// The one deadline timer, for `model.next_deadline()`.
    wake: Option<(Instant, Task<()>)>,
    /// Each window's configured size, which wins over the display's bounds.
    viewports: BTreeMap<OutputId, gpui::Size<gpui::Pixels>>,
    /// Events produced while carrying out effects, run before the next sync.
    queue: VecDeque<EventKind>,
}

/// Registers the compositor's lock and display events and starts the model's first effects.
pub fn start(
    cx: &mut App,
    host: Box<dyn Host>,
    model: LockModel,
    effects: Vec<Effect>,
) -> Entity<Locker> {
    let locker = cx.new(|_| Locker {
        model,
        host,
        windows: BTreeMap::new(),
        opening: BTreeSet::new(),
        wake: None,
        viewports: BTreeMap::new(),
        queue: VecDeque::new(),
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
    locker.update(cx, |locker, cx| {
        locker.carry_out(effects, cx);
        locker.sync(cx);
    });
    locker
}

/// Feeds `locker` from a channel for as long as it stays open.
pub fn pump<T: 'static>(
    cx: &mut App,
    locker: &Entity<Locker>,
    received: async_channel::Receiver<T>,
    event: impl Fn(T) -> EventKind + 'static,
) {
    let locker = locker.clone();
    cx.spawn(async move |cx| {
        while let Ok(item) = received.recv().await {
            let kind = event(item);
            locker.update(cx, |locker, cx| locker.dispatch(kind, cx));
        }
    })
    .detach();
}

/// The displays, as the model's outputs.
pub fn outputs(cx: &App) -> Vec<Output> {
    cx.displays()
        .iter()
        .map(|display| Output {
            id: OutputId(u64::from(display.id())),
            size: display.bounds().size,
        })
        .collect()
}

impl Locker {
    pub fn log(&self, line: &str) {
        log(self.host.name(), line);
    }

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

    pub fn look(&self, login: Option<&str>) -> Rc<Look> {
        self.host.look(login)
    }

    pub fn greeting_stored(&self) -> bool {
        self.host.greeting_stored()
    }

    pub fn greeting_shown(&mut self, text: String) {
        self.host.greeting_shown(text);
    }

    fn session_lock(&mut self, event: SessionLockEvent, cx: &mut Context<Self>) {
        let kind = match event {
            SessionLockEvent::Locked => {
                self.log("event=locked");
                EventKind::Locked
            }
            SessionLockEvent::Finished => {
                self.log("event=finished");
                EventKind::Finished
            }
        };
        self.host.session_lock(&event);
        self.dispatch(kind, cx);
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
            Effect::DisplayOff => system::power_off_monitors(self.host.name()),
            Effect::SampleChips => self
                .queue
                .push_back(EventKind::Chips(system::sample_chips())),
            Effect::CloseShellSurfaces => system::close_shell_surfaces(),
            effect => self.host.apply(
                effect,
                Ctx {
                    queue: &mut self.queue,
                    model: &self.model,
                    cx,
                },
            ),
        }
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
    let initial = locker
        .read(cx)
        .output_size(id)
        .unwrap_or_else(|| size(px(800.), px(600.)));
    let name = locker.read(cx).host.name();
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
            app_id: Some(name.into()),
            kind: WindowKind::SessionLock,
            ..Default::default()
        },
        |window, cx| {
            let entity = cx.new(|cx| LockView::new(id, locker.clone(), painted.clone(), window, cx));
            window.focus(&entity.read(cx).focus_handle(), cx);
            view = Some(entity.clone());
            entity
        },
    );
    let window = match (result, view) {
        (Ok(handle), Some(view)) => {
            log(name, &format!("event=window-opened output={}", id.0));
            Some(LockWindow {
                handle,
                view,
                painted,
            })
        }
        (Err(error), _) => {
            eprintln!(
                "{name}: could not open a lock window on output {}: {error:#}",
                id.0
            );
            None
        }
        (Ok(_), None) => None,
    };
    locker.update(cx, |locker, cx| locker.opened(id, window, cx));
}
