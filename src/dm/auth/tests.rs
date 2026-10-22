use super::*;
use crate::lock::pam::Notice;
use crate::lock::secret::Password;

const G: u32 = 3;

fn login() -> Login {
    Login::parse("nathan").unwrap()
}

fn session() -> SessionId {
    SessionId::parse("niri-uwsm").unwrap()
}

fn secret(text: &str) -> Secret {
    let mut password = Password::new();
    text.chars().for_each(|c| assert!(password.push(c)));
    Secret::new(password)
}

fn request(request: Request) -> Input {
    Input::Greeter(request)
}

fn report(report: Report) -> Input {
    Input::Helper(HelperEvent::Report(report))
}

fn session_event(event: SessionEvent) -> Input {
    Input::Helper(HelperEvent::Session(event))
}

fn begin(generation: u32) -> Input {
    request(Request::Begin {
        generation,
        method: Method::Password,
        login: login(),
        session: session(),
    })
}

fn relayed(report: Report) -> Effect {
    Effect::Relay(Event::Report {
        generation: G,
        method: Method::Password,
        report,
    })
}

fn hint() -> Report {
    Report::Notice(Notice::Hint {
        text: "hint".into(),
        error: false,
    })
}

fn success() -> Report {
    Report::Finished(PamOutcome::Success)
}

/// Drives a fresh attempt into `phase` through real inputs only.
fn at(phase: Phase) -> Attempt {
    let path: &[fn() -> Input] = &[
        || begin(G),
        || report(success()),
        || request(Request::StartSession { generation: G }),
        || session_event(SessionEvent::Started),
        || session_event(SessionEvent::Ended),
    ];
    let steps = match phase {
        Phase::Idle => 0,
        Phase::Authenticating => 1,
        Phase::Authenticated => 2,
        Phase::Starting => 3,
        Phase::SessionRunning => 4,
        Phase::Closed => 5,
    };
    let mut attempt = Attempt::default();
    for step in &path[..steps] {
        attempt.reduce(step()).unwrap();
    }
    assert_eq!(attempt.phase(), phase);
    attempt
}

type Make = fn() -> Input;

const INPUTS: &[(&str, Make)] = &[
    ("begin", || begin(G)),
    ("answer", || {
        request(Request::Answer {
            generation: G,
            secret: secret("pw"),
        })
    }),
    ("cancel", || request(Request::Cancel { generation: G })),
    ("start_session", || {
        request(Request::StartSession { generation: G })
    }),
    ("stale_answer", || {
        request(Request::Answer {
            generation: G + 1,
            secret: secret("pw"),
        })
    }),
    ("stale_cancel", || {
        request(Request::Cancel { generation: G + 1 })
    }),
    ("stale_start", || {
        request(Request::StartSession { generation: G + 1 })
    }),
    ("hello", || request(Request::Hello { version: 1 })),
    ("select_user", || {
        request(Request::SelectUser { login: login() })
    }),
    ("power", || {
        request(Request::Power {
            occasion: crate::dm::protocol::PowerOccasion::Shutdown,
        })
    }),
    ("prompt", || report(Report::Prompt { echo: false })),
    ("hint", || report(hint())),
    ("finished_success", || report(success())),
    ("finished_rejected", || {
        report(Report::Finished(PamOutcome::Rejected))
    }),
    ("crashed", || report(Report::Crashed)),
    ("started", || session_event(SessionEvent::Started)),
    ("failed", || {
        session_event(SessionEvent::Failed(SessionFailure::Refused))
    }),
    ("ended", || session_event(SessionEvent::Ended)),
];

const PHASES: [Phase; 6] = [
    Phase::Idle,
    Phase::Authenticating,
    Phase::Authenticated,
    Phase::Starting,
    Phase::SessionRunning,
    Phase::Closed,
];

#[derive(Debug, PartialEq)]
enum Outcome {
    Accept(Phase),
    Reject(Rejected),
}

/// The whole transition table. Anything not listed is out of order.
fn expected(phase: Phase, input: &str) -> Outcome {
    use Outcome::{Accept, Reject};
    use Phase::*;
    if phase == Closed {
        return Reject(Rejected::Closed);
    }
    match (phase, input) {
        (_, "hello" | "select_user" | "power") => Reject(Rejected::Unrelated),
        (Idle, "begin") => Accept(Authenticating),
        (Authenticating, "answer") => Accept(Authenticating),
        (Authenticating | Authenticated, "cancel") => Accept(Closed),
        (Authenticated, "start_session") => Accept(Starting),
        (Authenticating, "prompt" | "hint") => Accept(Authenticating),
        (Authenticating, "finished_success") => Accept(Authenticated),
        (Authenticating, "finished_rejected" | "crashed") => Accept(Closed),
        (Authenticated | Starting | SessionRunning, "crashed") => Accept(Closed),
        (Starting, "started") => Accept(SessionRunning),
        (Starting, "failed" | "ended") => Accept(Closed),
        (SessionRunning, "ended") => Accept(Closed),
        (Idle, "stale_answer" | "stale_cancel" | "stale_start") => Reject(Rejected::OutOfOrder),
        (_, "stale_answer" | "stale_cancel" | "stale_start") => Reject(Rejected::Stale),
        _ => Reject(Rejected::OutOfOrder),
    }
}

#[test]
fn every_input_in_every_phase_follows_the_table_and_rejection_changes_nothing() {
    for phase in PHASES {
        for (name, make) in INPUTS {
            let mut attempt = at(phase);
            let before = attempt.clone();
            let result = attempt.reduce(make());
            match expected(phase, name) {
                Outcome::Accept(next) => {
                    assert!(result.is_ok(), "{phase:?} + {name}: {result:?}");
                    assert_eq!(attempt.phase(), next, "{phase:?} + {name}");
                }
                Outcome::Reject(why) => {
                    assert_eq!(result, Err(why), "{phase:?} + {name}");
                    assert_eq!(attempt, before, "{phase:?} + {name} changed the state");
                }
            }
        }
    }
}

#[test]
fn a_password_login_from_begin_to_session_end() {
    let mut attempt = Attempt::default();
    assert_eq!(
        attempt.reduce(begin(G)),
        Ok(vec![Effect::Spawn {
            method: Method::Password,
            login: login()
        }])
    );
    assert_eq!(attempt.generation(), Some(G));
    assert_eq!(
        attempt.reduce(report(Report::Prompt { echo: false })),
        Ok(vec![relayed(Report::Prompt { echo: false })])
    );
    assert_eq!(
        attempt.reduce(request(Request::Answer {
            generation: G,
            secret: secret("fake-secret")
        })),
        Ok(vec![Effect::Answer(secret("fake-secret"))])
    );
    assert_eq!(
        attempt.reduce(report(success())),
        Ok(vec![relayed(success())])
    );
    assert_eq!(
        attempt.reduce(request(Request::StartSession { generation: G })),
        Ok(vec![Effect::StartSession(session())])
    );
    assert_eq!(
        attempt.reduce(session_event(SessionEvent::Started)),
        Ok(vec![Effect::Relay(Event::SessionStarted)])
    );
    assert_eq!(attempt.phase(), Phase::SessionRunning);
    assert_eq!(
        attempt.reduce(session_event(SessionEvent::Ended)),
        Ok(vec![Effect::Close])
    );
    assert_eq!(attempt.phase(), Phase::Closed);
}

#[test]
fn a_failed_or_crashed_conversation_is_relayed_then_closed() {
    for end in [
        Report::Finished(PamOutcome::Rejected),
        Report::Finished(PamOutcome::MaxTries),
        Report::Finished(PamOutcome::Other(13)),
        Report::Crashed,
    ] {
        let mut attempt = at(Phase::Authenticating);
        assert_eq!(
            attempt.reduce(report(end.clone())),
            Ok(vec![relayed(end), Effect::Close])
        );
        assert_eq!(attempt.phase(), Phase::Closed);
    }
}

#[test]
fn notices_pass_through_while_authenticating() {
    let mut attempt = at(Phase::Authenticating);
    for notice in [
        Notice::PasswordLocked,
        Notice::PasswordPaused {
            remaining: std::time::Duration::from_secs(600),
        },
    ] {
        let report = Report::Notice(notice);
        assert_eq!(
            attempt.reduce(self::report(report.clone())),
            Ok(vec![relayed(report)])
        );
    }
    assert_eq!(attempt.phase(), Phase::Authenticating);
}

#[test]
fn a_helper_that_dies_after_success_fails_the_session_for_the_greeter() {
    for phase in [Phase::Authenticated, Phase::Starting] {
        let mut attempt = at(phase);
        assert_eq!(
            attempt.reduce(report(Report::Crashed)),
            Ok(vec![
                Effect::Relay(Event::SessionFailed {
                    reason: SessionFailure::HelperCrashed
                }),
                Effect::Close
            ])
        );
    }
}

#[test]
fn a_session_that_fails_to_start_is_relayed_with_its_reason() {
    for reason in [SessionFailure::Refused, SessionFailure::SpawnFailed] {
        let mut attempt = at(Phase::Starting);
        assert_eq!(
            attempt.reduce(session_event(SessionEvent::Failed(reason))),
            Ok(vec![
                Effect::Relay(Event::SessionFailed { reason }),
                Effect::Close
            ])
        );
        assert_eq!(attempt.phase(), Phase::Closed);
    }
    let mut attempt = at(Phase::Starting);
    assert_eq!(
        attempt.reduce(session_event(SessionEvent::Ended)),
        Ok(vec![
            Effect::Relay(Event::SessionFailed {
                reason: SessionFailure::Exited
            }),
            Effect::Close
        ])
    );
}

#[test]
fn start_session_is_refused_before_success_and_twice() {
    let mut attempt = at(Phase::Authenticating);
    assert_eq!(
        attempt.reduce(request(Request::StartSession { generation: G })),
        Err(Rejected::OutOfOrder)
    );
    assert_eq!(attempt.phase(), Phase::Authenticating);
    let mut attempt = at(Phase::Authenticated);
    assert!(
        attempt
            .reduce(request(Request::StartSession { generation: G }))
            .is_ok()
    );
    assert_eq!(
        attempt.reduce(request(Request::StartSession { generation: G })),
        Err(Rejected::OutOfOrder)
    );
}

#[test]
fn an_answer_for_a_stale_generation_never_reaches_the_helper() {
    let mut attempt = at(Phase::Authenticating);
    assert_eq!(
        attempt.reduce(request(Request::Answer {
            generation: G - 1,
            secret: secret("old")
        })),
        Err(Rejected::Stale)
    );
}

#[test]
fn cancel_before_the_session_closes_and_after_it_does_not() {
    for phase in [Phase::Authenticating, Phase::Authenticated] {
        let mut attempt = at(phase);
        assert_eq!(
            attempt.reduce(request(Request::Cancel { generation: G })),
            Ok(vec![Effect::Close])
        );
    }
    for phase in [Phase::Starting, Phase::SessionRunning] {
        let mut attempt = at(phase);
        assert_eq!(
            attempt.reduce(request(Request::Cancel { generation: G })),
            Err(Rejected::OutOfOrder)
        );
    }
}
