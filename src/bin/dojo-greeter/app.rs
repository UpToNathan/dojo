//! The greeter's side of the shared edge (`dojo::lock::edge`): the display manager's socket
//! behind the model's effects, the users' looks, and a last frame that stays as it was.
//!
//! Everything arrives on one channel: the display manager's events from the reader thread and
//! the snapshots that loader threads finish. The pump turns each into model events on the UI
//! thread, so the model sees one ordered stream, as it does on the lock screen.

use std::cell::RefCell;
use std::rc::Rc;

use dojo::dm::client::{Client, Incoming};
use dojo::dm::edge::{Answer, Bridge, Step, user_look};
use dojo::dm::protocol::{Event, Login as Account, Request};
use dojo::lock::edge::{self, Ctx, Host};
use dojo::lock::{Effect, EventKind, ExitReason, Login, LockModel, Settings};
use dojo::session::host::local_hour;
use dojo::session::message;
use dojo::session::paint::Look;
use dojo::session::transition::SHELL_CONTROLS_WIDTH;
use gpui::{App, SessionLockEvent};

use crate::looks::{self, Loaded, Looks};

pub use dojo::lock::edge::mark_start;

const NAME: &str = "dojo-greeter";

pub fn log(line: &str) {
    edge::log(NAME, line);
}

pub struct Options {
    pub client: Client,
    pub bridge: Bridge,
    /// The user the screen starts on, prefilled and already asked about.
    pub account: Account,
    pub login: Login,
    /// The first user's snapshot, loading since `main` started so it overlaps GPUI's start-up.
    pub initial: std::thread::JoinHandle<Loaded>,
    pub reduced_motion: bool,
}

enum Input {
    Dm(Incoming),
    Loaded(Loaded),
}

/// What the host and the pump share. Both run on the UI thread.
struct Greeter {
    client: Client,
    bridge: RefCell<Bridge>,
    looks: RefCell<Looks>,
    inputs: async_channel::Sender<Input>,
}

struct GreeterHost {
    greeter: Rc<Greeter>,
}

/// Runs the GPUI event loop until the session starts, or the connection to the display manager
/// or the compositor ends. Only a started session is a clean exit.
pub fn run(options: Options) -> u8 {
    gpui_platform::application().run(move |cx: &mut App| {
        if let Err(error) = dojo::power::register_font(cx) {
            eprintln!("{NAME}: {error:#}");
        }
        edge::bind_keys(cx);
        edge::bind_login_keys(cx);
        let (inputs, received) = async_channel::unbounded::<Input>();
        if let Err(error) = options.client.read_in_background(inputs.clone(), Input::Dm) {
            eprintln!("{NAME}: could not start the reader thread: {error}");
            cx.quit();
            return;
        }
        let initial = match options.initial.join() {
            Ok(initial) => initial,
            Err(_) => {
                eprintln!("{NAME}: the first look could not be loaded");
                cx.quit();
                return;
            }
        };
        let resting = options.account.as_str().to_owned();
        let mut looks = Looks::new(&resting);
        looks.insert(&initial);
        let first = user_look(&initial.answer, initial.snapshot.as_ref(), &context());
        let settings = Settings {
            reduced_motion: options.reduced_motion
                || cx.reduce_motion()
                || initial
                    .snapshot
                    .as_ref()
                    .is_some_and(|look| look.settings_or_default().reduced_motion),
            user: resting.clone(),
            greeting: None,
            controls_width: SHELL_CONTROLS_WIDTH,
            lock_age_origin: None,
            farewell: None,
            login: Some(options.login),
        };
        let (model, effects) = LockModel::new(settings, edge::now(), edge::outputs(cx));
        let greeter = Rc::new(Greeter {
            client: options.client,
            bridge: RefCell::new(options.bridge),
            looks: RefCell::new(looks),
            inputs,
        });
        let locker = edge::start(
            cx,
            Box::new(GreeterHost {
                greeter: greeter.clone(),
            }),
            model,
            effects,
        );
        // The first user's name, greetings and farewells; the display manager adds the print.
        locker.update(cx, |locker, cx| {
            locker.dispatch(EventKind::LookReady(first), cx)
        });
        let ask = greeter.bridge.borrow_mut().ask(&options.account);
        greeter.send(&ask, cx);
        edge::pump(cx, &locker, received, move |input, cx| {
            greeter.arrived(input, cx)
        });
    });
    edge::exit_status()
}

/// The clock and the boot state the greeting and farewell selection reads.
fn context() -> message::Context {
    let now = edge::now();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.subsec_nanos());
    message::Context {
        hour: local_hour(now.unix),
        fresh_boot: dojo::session::login::fresh_boot(),
        seed: (now.unix as u64) << 32 | u64::from(nanos),
        ..message::Context::default()
    }
}

impl Greeter {
    /// A request that cannot be sent means the display manager is gone: the greeter ends.
    fn send(&self, request: &Request, cx: &mut App) {
        if let Err(error) = self.client.send(request) {
            eprintln!("{NAME}: lost the display manager: {error}");
            cx.quit();
        }
    }

    fn steps(&self, steps: Vec<Step>, events: &mut Vec<EventKind>, cx: &mut App) {
        for step in steps {
            match step {
                Step::Send(request) => self.send(&request, cx),
                Step::Model(kind) => events.push(kind),
                Step::Look(answer) => self.look_wanted(answer, events),
            }
        }
    }

    /// The display manager answered for a user: finish the answer from their snapshot, reading
    /// it first if this is the first time they are asked for.
    fn look_wanted(&self, answer: Answer, events: &mut Vec<EventKind>) {
        let known = self
            .looks
            .borrow()
            .read(&answer.login)
            .map(|snapshot| user_look(&answer, snapshot, &context()));
        if let Some(look) = known {
            events.push(EventKind::LookReady(look));
            return;
        }
        let inputs = self.inputs.clone();
        let root = looks::state_directory();
        std::thread::spawn(move || {
            let _ = inputs.send_blocking(Input::Loaded(looks::load(&root, answer)));
        });
    }

    fn arrived(&self, input: Input, cx: &mut App) -> Vec<EventKind> {
        let mut events = Vec::new();
        match input {
            Input::Dm(Incoming::Event(event)) => {
                if let Event::SessionFailed { reason } = &event {
                    log(&format!("event=dm-session-failed reason={reason:?}"));
                }
                let steps = self.bridge.borrow_mut().event(event);
                match steps {
                    Ok(steps) => self.steps(steps, &mut events, cx),
                    Err(violation) => {
                        eprintln!("{NAME}: the display manager broke the protocol: {violation:?}");
                        cx.quit();
                    }
                }
            }
            Input::Dm(Incoming::Malformed(error)) => {
                eprintln!("{NAME}: the display manager sent {error}");
                cx.quit();
            }
            Input::Dm(Incoming::Closed(_)) if self.bridge.borrow().handed_over() => {
                // It stops the greeter to start the session, so SessionStarted never comes.
                log("event=handed-over");
                edge::exit_cleanly();
                cx.quit();
            }
            Input::Dm(Incoming::Closed(error)) => {
                match error {
                    Some(error) => eprintln!("{NAME}: lost the display manager: {error}"),
                    None => eprintln!("{NAME}: the display manager closed the socket"),
                }
                cx.quit();
            }
            Input::Loaded(loaded) => {
                self.looks.borrow_mut().insert(&loaded);
                events.push(EventKind::LookReady(user_look(
                    &loaded.answer,
                    loaded.snapshot.as_ref(),
                    &context(),
                )));
            }
        }
        events
    }
}

impl Host for GreeterHost {
    fn name(&self) -> &'static str {
        NAME
    }

    fn look(&self, login: Option<&str>) -> Rc<Look> {
        self.greeter.looks.borrow().look(login)
    }

    fn session_lock(&mut self, _: &SessionLockEvent) {}

    fn apply(&mut self, effect: Effect, ctx: Ctx<'_>) {
        match effect {
            // The shell draws the login opening, as the model already assumes.
            Effect::ProbeShellOverlay => ctx.queue.push_back(EventKind::ShellOverlay(true)),
            Effect::Exit(reason) => {
                // Only a started session is a clean end: the screen stays as the closed state
                // the shell continues from. Anything else makes the display manager start a new greeter.
                if reason == ExitReason::Unlocked {
                    edge::exit_cleanly();
                }
                log(&format!("event=exit reason={reason:?}"));
                ctx.cx.quit();
            }
            // The greeter never releases the session lock: it ends with the compositor.
            Effect::DisplayOff | Effect::SampleChips | Effect::CloseShellSurfaces => {}
            effect => {
                let steps = self
                    .greeter
                    .bridge
                    .borrow_mut()
                    .effect(effect, ctx.model.choice());
                let mut events = Vec::new();
                self.greeter.steps(steps, &mut events, ctx.cx);
                ctx.queue.extend(events);
            }
        }
    }

    /// Nothing is remembered from a login screen: the user's history is theirs to write.
    fn greeting_shown(&mut self, _: String) {}

    fn greeting_stored(&self) -> bool {
        true
    }
}
