//! The greeter's whole path with the pieces that need no window: the real lock model, the
//! bridge, the real protocol on a socketpair, and a fake display manager on its other end.

use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use std::time::{Duration, Instant};

use gpui::{px, size};

use super::*;
use crate::dm::client::{Client, Incoming};
use crate::dm::protocol::{self, SessionId};
use crate::lock::pam::Notice;
use crate::lock::{
    ExitReason, Event as ModelEvent, Key, LockModel, Login as LoginSettings, Now, Output,
    OutputId, Settings,
};

const OUT: OutputId = OutputId(1);
const UNIX0: i64 = 1_791_983_100;

fn pair() -> (OwnedFd, OwnedFd) {
    let mut fds = [0; 2];
    let rc = unsafe {
        libc::socketpair(
            libc::AF_UNIX,
            libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC,
            0,
            fds.as_mut_ptr(),
        )
    };
    assert_eq!(rc, 0);
    unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) }
}

fn login(text: &str) -> Login {
    Login::parse(text).unwrap()
}

fn session(id: &str) -> SessionId {
    SessionId::parse(id).unwrap()
}

/// The display manager's end: what it reads and what it says.
struct Dm(OwnedFd);

impl Dm {
    fn heard(&self) -> Request {
        protocol::receive::<Request>(self.0.as_fd())
            .expect("receive")
            .expect("a request, not the end of the stream")
            .expect("a request that decodes")
    }

    fn says(&self, event: Event) {
        protocol::send(self.0.as_fd(), &event).expect("send");
    }
}

/// The greeter without its windows. Time is whatever the test says it is.
struct Greeter {
    model: LockModel,
    bridge: Bridge,
    client: Client,
    incoming: async_channel::Receiver<Incoming>,
    t0: Instant,
    queue: Vec<EventKind>,
    /// Every effect the model produced, as the edge saw it.
    effects: Vec<String>,
    exits: Vec<ExitReason>,
    asked_for_looks: Vec<Answer>,
}

fn start() -> (Greeter, Dm) {
    let (near, far) = pair();
    let sessions = vec![
        SessionEntry {
            id: session("niri-uwsm"),
            name: "Niri (UWSM)".into(),
        },
        SessionEntry {
            id: session("gnome"),
            name: "GNOME".into(),
        },
    ];
    let names = session_names(&sessions);
    let settings = Settings {
        reduced_motion: false,
        user: "nathan".into(),
        greeting: None,
        controls_width: 300.,
        lock_age_origin: None,
        farewell: None,
        login: Some(LoginSettings::new("nathan", names, "GNOME").unwrap()),
    };
    let t0 = Instant::now();
    let now = Now {
        mono: t0,
        unix: UNIX0,
    };
    let outputs = vec![Output {
        id: OUT,
        size: size(px(1920.), px(1080.)),
    }];
    let (model, _) = LockModel::new(settings, now, outputs);
    let client = Client::over(near);
    let (events, incoming) = async_channel::unbounded();
    client.read_in_background(events, |incoming| incoming).unwrap();
    let greeter = Greeter {
        model,
        bridge: Bridge::new(sessions),
        client,
        incoming,
        t0,
        queue: Vec::new(),
        effects: Vec::new(),
        exits: Vec::new(),
        asked_for_looks: Vec::new(),
    };
    (greeter, Dm(far))
}

impl Greeter {
    fn at(&self, ms: u64) -> Now {
        Now {
            mono: self.t0 + Duration::from_millis(ms),
            unix: UNIX0 + (ms / 1000) as i64,
        }
    }

    fn send(&mut self, ms: u64, kind: EventKind) {
        self.queue.push(kind);
        while !self.queue.is_empty() {
            let kind = self.queue.remove(0);
            let now = self.at(ms);
            let effects = self.model.update(ModelEvent { now, kind });
            self.carry_out(effects);
        }
    }

    fn key(&mut self, ms: u64, key: Key) {
        self.send(ms, EventKind::Key(OUT, key));
    }

    fn type_text(&mut self, ms: u64, text: &str) {
        for c in text.chars() {
            self.key(ms, Key::Char(c));
        }
    }

    fn carry_out(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            self.effects.push(format!("{effect:?}"));
            if let Effect::Exit(reason) = &effect {
                self.exits.push(*reason);
            }
            let steps = self.bridge.effect(effect, self.model.choice());
            self.steps(steps);
        }
    }

    fn steps(&mut self, steps: Vec<Step>) {
        for step in steps {
            match step {
                Step::Send(request) => self.client.send(&request).expect("send"),
                Step::Model(kind) => self.queue.push(kind),
                Step::Look(answer) => self.asked_for_looks.push(answer),
            }
        }
    }

    /// The next thing the display manager said, run through the bridge and the model.
    fn hears(&mut self, ms: u64) {
        let incoming = futures_lite::future::block_on(self.incoming.recv()).expect("an event");
        let Incoming::Event(event) = incoming else {
            panic!("expected an event, got {incoming:?}");
        };
        let steps = self.bridge.event(event).expect("in protocol");
        self.steps(steps);
        // A look is answered the way the edge answers it, from no snapshot at all.
        for answer in std::mem::take(&mut self.asked_for_looks) {
            let look = user_look(&answer, None, &message::Context::default());
            self.queue.push(EventKind::LookReady(look));
        }
        self.send(ms, EventKind::Timer);
    }
}

fn begin(generation: u32, method: Method, name: &str, id: &str) -> Request {
    Request::Begin {
        generation,
        method,
        login: login(name),
        session: session(id),
    }
}

fn report(generation: u32, method: Method, report: Report) -> Event {
    Event::Report {
        generation,
        method,
        report,
    }
}

fn user(name: &str, fingerprint: bool) -> Event {
    Event::User {
        login: login(name),
        display_name: Some(name.to_uppercase()),
        fingerprint,
    }
}

/// Locked at 0 ms; the first user asked about and answered.
fn locked() -> (Greeter, Dm) {
    let (mut greeter, dm) = start();
    let ask = greeter.bridge.ask(&login("nathan"));
    greeter.client.send(&ask).unwrap();
    assert_eq!(dm.heard(), Request::SelectUser { login: login("nathan") });
    dm.says(user("nathan", false));
    greeter.hears(1);
    greeter.send(10, EventKind::Locked);
    (greeter, dm)
}

#[test]
fn a_password_login_runs_from_the_first_prompt_to_the_started_session() {
    let (mut greeter, dm) = locked();
    // The conversation begins for the user and the session chip's choice.
    assert_eq!(dm.heard(), begin(1, Method::Password, "nathan", "gnome"));
    dm.says(report(1, Method::Password, Report::Prompt { echo: false }));
    greeter.hears(20);

    greeter.type_text(100, "wrong");
    greeter.key(150, Key::Enter);
    let Request::Answer { generation: 1, secret: wrong } = dm.heard() else {
        panic!("the password is answered");
    };
    assert_eq!(wrong.into_password().expose(), b"wrong");
    dm.says(report(1, Method::Password, Report::Finished(PamOutcome::Rejected)));
    greeter.hears(200);
    // A fresh conversation, with a fresh generation, replaces the refused one.
    assert_eq!(dm.heard(), begin(2, Method::Password, "nathan", "gnome"));
    dm.says(report(2, Method::Password, Report::Prompt { echo: false }));
    greeter.hears(300);

    greeter.type_text(400, "kumite");
    greeter.key(450, Key::Enter);
    let Request::Answer { generation: 2, secret: right } = dm.heard() else {
        panic!("the password is answered again");
    };
    assert_eq!(right.into_password().expose(), b"kumite");
    dm.says(report(2, Method::Password, Report::Finished(PamOutcome::Success)));
    greeter.hears(500);
    assert!(greeter.exits.is_empty());

    // The opening plays out; its end asks the display manager to start the session.
    for ms in [800, 950, 1_100] {
        greeter.send(ms, EventKind::Timer);
    }
    assert_eq!(dm.heard(), Request::StartSession { generation: 2 });
    assert!(greeter.exits.is_empty(), "it waits for the answer");

    dm.says(Event::SessionStarted);
    greeter.hears(2_000);
    assert_eq!(greeter.exits, [ExitReason::Unlocked]);
}

#[test]
fn a_session_that_fails_to_start_is_asked_for_again_on_a_new_conversation() {
    let (mut greeter, dm) = locked();
    dm.heard();
    dm.says(report(1, Method::Password, Report::Prompt { echo: false }));
    greeter.hears(20);
    greeter.type_text(100, "kumite");
    greeter.key(150, Key::Enter);
    dm.heard();
    dm.says(report(1, Method::Password, Report::Finished(PamOutcome::Success)));
    greeter.hears(200);
    for ms in [500, 650, 800, 950, 1_100] {
        greeter.send(ms, EventKind::Timer);
    }
    assert_eq!(dm.heard(), Request::StartSession { generation: 1 });

    dm.says(Event::SessionFailed {
        reason: SessionFailure::SpawnFailed,
    });
    greeter.hears(1_500);
    assert!(greeter.exits.is_empty());
    assert_eq!(dm.heard(), begin(2, Method::Password, "nathan", "gnome"));
    let scene = greeter.model.scene(OUT, greeter.at(1_500)).unwrap();
    let crate::lock::Role::Active(panel) = scene.role else {
        panic!("the active output");
    };
    assert_eq!(
        panel.line,
        Some(crate::lock::Line::Hint(
            failure_text(SessionFailure::SpawnFailed).to_owned()
        ))
    );
}

#[test]
fn switching_user_session_and_method_keeps_every_request_in_order_and_on_the_right_generation() {
    let (mut greeter, dm) = locked();
    assert_eq!(dm.heard(), begin(1, Method::Password, "nathan", "gnome"));
    dm.says(report(1, Method::Password, Report::Prompt { echo: false }));
    greeter.hears(20);

    greeter.key(100, Key::Tab);
    greeter.type_text(110, "alyssa");
    greeter.key(200, Key::Enter);
    // The look is asked for before the old conversation is cancelled and the new one begins.
    assert_eq!(dm.heard(), Request::SelectUser { login: login("alyssa") });
    assert_eq!(dm.heard(), Request::Cancel { generation: 1 });
    assert_eq!(dm.heard(), begin(2, Method::Password, "alyssa", "gnome"));
    // Alyssa has a print, so the sensor starts once the display manager says so.
    dm.says(user("alyssa", true));
    greeter.hears(250);
    assert_eq!(dm.heard(), begin(3, Method::Fingerprint, "alyssa", "gnome"));

    // Another session restarts both conversations for it.
    greeter.key(300, Key::NextSession);
    assert_eq!(dm.heard(), Request::Cancel { generation: 2 });
    assert_eq!(dm.heard(), begin(4, Method::Password, "alyssa", "niri-uwsm"));
    assert_eq!(dm.heard(), Request::Cancel { generation: 3 });
    assert_eq!(dm.heard(), begin(5, Method::Fingerprint, "alyssa", "niri-uwsm"));

    // A finger wins: the password conversation is cancelled and the session starts from the finger's.
    dm.says(report(5, Method::Fingerprint, Report::Finished(PamOutcome::Success)));
    greeter.hears(400);
    assert_eq!(dm.heard(), Request::Cancel { generation: 4 });
    for ms in [700, 850, 1_000, 1_150] {
        greeter.send(ms, EventKind::Timer);
    }
    assert_eq!(dm.heard(), Request::StartSession { generation: 5 });
}

#[test]
fn a_look_answer_for_a_user_who_has_since_changed_is_dropped() {
    let (mut greeter, dm) = locked();
    dm.heard();
    greeter.key(100, Key::Tab);
    greeter.type_text(110, "alyssa");
    greeter.key(200, Key::Enter);
    greeter.key(300, Key::Tab);
    greeter.type_text(310, "bob");
    greeter.key(400, Key::Enter);
    for _ in 0..2 {
        dm.heard();
    }
    dm.says(user("alyssa", true));
    dm.says(user("bob", false));
    greeter.hears(500);
    greeter.hears(510);
    assert_eq!(greeter.model.choice().unwrap().login, "bob");
    let scene = greeter.model.scene(OUT, greeter.at(520)).unwrap();
    assert_eq!(scene.look.unwrap().login, "bob");
    // Alyssa's print was never armed.
    assert!(!greeter.effects.iter().any(|e| e.contains("Fingerprint")));
}

#[test]
fn a_name_the_display_manager_cannot_hold_never_reaches_it() {
    let (mut greeter, dm) = locked();
    dm.heard();
    dm.says(report(1, Method::Password, Report::Prompt { echo: false }));
    greeter.hears(20);
    greeter.key(100, Key::Tab);
    greeter.type_text(110, "Bob");
    greeter.key(200, Key::Enter);
    // Only the cancel of the old conversation is heard; the new one stays in the greeter.
    assert_eq!(dm.heard(), Request::Cancel { generation: 1 });
    greeter.type_text(300, "anything");
    greeter.key(350, Key::Enter);
    let scene = greeter.model.scene(OUT, greeter.at(400)).unwrap();
    assert!(scene.look.is_some());
    // The refusal shakes the pill like any wrong password, and nothing else is sent.
    greeter.send(400, EventKind::Timer);
    greeter.client.send(&Request::Hello { version: 7 }).unwrap();
    assert_eq!(dm.heard(), Request::Hello { version: 7 });
}

#[test]
fn leaving_cancels_the_conversations_and_asks_for_the_occasion_after_its_farewell() {
    let (mut greeter, dm) = locked();
    dm.heard();
    dm.says(report(1, Method::Password, Report::Prompt { echo: false }));
    greeter.hears(20);
    greeter.key(100, Key::Leave(crate::lock::Leave::Restart));
    assert_eq!(dm.heard(), Request::Cancel { generation: 1 });
    // No farewell is known for this user, so the occasion follows the release.
    greeter.send(250, EventKind::Timer);
    assert_eq!(
        dm.heard(),
        Request::Power {
            occasion: PowerOccasion::Restart
        }
    );
    assert!(greeter.exits.is_empty());
}

#[test]
fn a_notice_from_the_display_manager_reaches_the_model_as_a_worker_report() {
    let (mut greeter, dm) = locked();
    dm.heard();
    dm.says(report(
        1,
        Method::Password,
        Report::Notice(Notice::PasswordPaused {
            remaining: Duration::from_secs(120),
        }),
    ));
    greeter.hears(30);
    assert!(greeter.effects.iter().any(|e| e.contains("Kill")));
    assert_eq!(dm.heard(), Request::Cancel { generation: 1 });
}
