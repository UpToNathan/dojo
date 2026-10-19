use super::*;
use crate::dm::launch::SessionEvent;
use crate::lock::pam::{Notice, PamOutcome, Report};
use crate::lock::secret::Password;

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

fn begin(generation: u32, method: Method) -> Request {
    Request::Begin {
        generation,
        method,
        login: login(),
        session: session(),
    }
}

fn report(report: Report) -> HelperEvent {
    HelperEvent::Report(report)
}

fn success() -> HelperEvent {
    report(Report::Finished(PamOutcome::Success))
}

fn relayed(generation: u32, method: Method, report: Report) -> Action {
    Action::Relay(Event::Report {
        generation,
        method,
        report,
    })
}

fn kill(generation: u32) -> Action {
    Action::Kill { generation }
}

#[test]
fn a_begin_spawns_a_helper_for_its_method() {
    let mut registry = Registry::new();
    assert_eq!(
        registry.request(begin(1, Method::Password)),
        Ok(vec![Action::Spawn {
            generation: 1,
            method: Method::Password,
            login: login()
        }])
    );
    assert_eq!(registry.phase(1), Some(Phase::Authenticating));
    assert_eq!(registry.winner(), None);
}

#[test]
fn both_methods_run_side_by_side_and_answers_find_their_generation() {
    let mut registry = Registry::new();
    registry.request(begin(1, Method::Fingerprint)).unwrap();
    registry.request(begin(2, Method::Password)).unwrap();
    assert_eq!(
        registry.request(Request::Answer {
            generation: 2,
            secret: secret("pw")
        }),
        Ok(vec![Action::Answer {
            generation: 2,
            secret: secret("pw")
        }])
    );
    assert_eq!(
        registry.helper(1, report(Report::Notice(Notice::PasswordLocked))),
        [relayed(
            1,
            Method::Fingerprint,
            Report::Notice(Notice::PasswordLocked)
        )]
    );
    assert_eq!(
        registry.request(Request::Answer {
            generation: 9,
            secret: secret("x")
        }),
        Err(Rejected::Stale)
    );
}

#[test]
fn the_first_success_wins_and_kills_the_loser_after_relaying_the_result() {
    let mut registry = Registry::new();
    registry.request(begin(1, Method::Fingerprint)).unwrap();
    registry.request(begin(2, Method::Password)).unwrap();
    assert_eq!(
        registry.helper(1, success()),
        [
            relayed(
                1,
                Method::Fingerprint,
                Report::Finished(PamOutcome::Success)
            ),
            kill(2)
        ]
    );
    let winner = registry.winner().unwrap();
    assert_eq!((winner.generation, winner.login.as_str()), (1, "nathan"));
    assert_eq!(registry.phase(2), None);
    assert_eq!(
        registry.helper(2, success()),
        [],
        "a late result from the loser is dropped"
    );
    assert_eq!(registry.winner().unwrap().generation, 1);
}

#[test]
fn a_rejected_attempt_is_closed_and_the_method_can_begin_again() {
    let mut registry = Registry::new();
    registry.request(begin(1, Method::Password)).unwrap();
    let rejected = report(Report::Finished(PamOutcome::Rejected));
    assert_eq!(
        registry.helper(1, rejected),
        [
            relayed(1, Method::Password, Report::Finished(PamOutcome::Rejected)),
            kill(1)
        ]
    );
    assert!(registry.is_empty());
    assert!(registry.request(begin(2, Method::Password)).is_ok());
    assert_eq!(
        registry
            .request(begin(1, Method::Password))
            .map(|a| a.len()),
        Ok(2),
        "a generation whose attempt is gone can begin again, replacing the method's live attempt"
    );
}

#[test]
fn begin_replaces_the_methods_live_attempt() {
    let mut registry = Registry::new();
    registry.request(begin(1, Method::Fingerprint)).unwrap();
    registry.request(begin(2, Method::Password)).unwrap();
    let actions = registry.request(begin(3, Method::Password)).unwrap();
    assert_eq!(
        actions,
        [
            kill(2),
            Action::Spawn {
                generation: 3,
                method: Method::Password,
                login: login()
            }
        ]
    );
    assert_eq!(registry.phase(1), Some(Phase::Authenticating));
    assert_eq!(registry.phase(2), None);
    assert_eq!(registry.phase(3), Some(Phase::Authenticating));
}

#[test]
fn a_repeated_generation_or_a_begin_after_a_win_is_refused() {
    let mut registry = Registry::new();
    registry.request(begin(1, Method::Fingerprint)).unwrap();
    assert_eq!(
        registry.request(begin(1, Method::Password)),
        Err(Rejected::OutOfOrder)
    );
    registry.helper(1, success());
    assert_eq!(
        registry.request(begin(5, Method::Password)),
        Err(Rejected::OutOfOrder)
    );
    assert_eq!(registry.phase(1), Some(Phase::Authenticated));
}

#[test]
fn cancelling_a_winner_clears_it() {
    let mut registry = Registry::new();
    registry.request(begin(1, Method::Password)).unwrap();
    registry.helper(1, success());
    assert_eq!(
        registry.request(Request::Cancel { generation: 1 }),
        Ok(vec![kill(1)])
    );
    assert_eq!(registry.winner(), None);
    assert!(registry.request(begin(2, Method::Password)).is_ok());
}

#[test]
fn start_session_is_accepted_only_for_the_winner() {
    let mut registry = Registry::new();
    registry.request(begin(1, Method::Fingerprint)).unwrap();
    registry.request(begin(2, Method::Password)).unwrap();
    assert_eq!(
        registry.request(Request::StartSession { generation: 1 }),
        Err(Rejected::OutOfOrder),
        "before the result"
    );
    registry.helper(2, success());
    assert_eq!(
        registry.request(Request::StartSession { generation: 1 }),
        Err(Rejected::Stale),
        "the loser is gone"
    );
    assert_eq!(
        registry.request(Request::StartSession { generation: 2 }),
        Ok(vec![Action::StartSession {
            generation: 2,
            session: session()
        }])
    );
    assert_eq!(registry.phase(2), Some(Phase::Starting));
}

#[test]
fn an_unresolvable_session_is_refused_and_the_helper_killed() {
    let mut registry = Registry::new();
    registry.request(begin(1, Method::Password)).unwrap();
    registry.helper(1, success());
    registry
        .request(Request::StartSession { generation: 1 })
        .unwrap();
    assert_eq!(
        registry.refuse_start(1, SessionFailure::UnknownSession),
        [
            Action::Relay(Event::SessionFailed {
                reason: SessionFailure::UnknownSession
            }),
            kill(1)
        ]
    );
    assert_eq!(registry.winner(), None);
    assert!(registry.is_empty());
    assert_eq!(registry.refuse_start(1, SessionFailure::UnknownSession), []);
}

#[test]
fn a_session_runs_and_ends_through_the_winners_helper() {
    let mut registry = Registry::new();
    registry.request(begin(1, Method::Password)).unwrap();
    registry.helper(1, success());
    registry
        .request(Request::StartSession { generation: 1 })
        .unwrap();
    assert_eq!(
        registry.helper(1, HelperEvent::Session(SessionEvent::Started)),
        [Action::Relay(Event::SessionStarted)]
    );
    assert_eq!(registry.phase(1), Some(Phase::SessionRunning));
    assert_eq!(
        registry.helper(1, HelperEvent::Session(SessionEvent::Ended)),
        [kill(1)]
    );
    assert_eq!(registry.winner(), None);
    assert!(registry.is_empty());
}

#[test]
fn a_helper_that_says_the_wrong_thing_is_killed() {
    let mut registry = Registry::new();
    registry.request(begin(1, Method::Password)).unwrap();
    registry.helper(1, success());
    assert_eq!(
        registry.helper(1, HelperEvent::Session(SessionEvent::Ended)),
        [kill(1)],
        "a session ending before it started"
    );
    assert!(registry.is_empty());
    assert_eq!(registry.winner(), None);
}

#[test]
fn requests_that_are_not_about_attempts_are_left_to_the_service() {
    let mut registry = Registry::new();
    for request in [
        Request::Hello { version: 1 },
        Request::SelectUser { login: login() },
        Request::Power {
            occasion: crate::dm::protocol::PowerOccasion::Shutdown,
        },
    ] {
        assert_eq!(registry.request(request), Err(Rejected::Unrelated));
    }
}

#[test]
fn losing_the_greeter_ends_attempts_still_proving_identity_but_not_a_starting_session() {
    let mut registry = Registry::new();
    registry.request(begin(1, Method::Fingerprint)).unwrap();
    registry.request(begin(2, Method::Password)).unwrap();
    assert_eq!(registry.greeter_gone(), [kill(1), kill(2)]);
    assert!(registry.is_empty());

    registry.request(begin(3, Method::Password)).unwrap();
    registry.helper(3, success());
    assert_eq!(
        registry.greeter_gone(),
        [kill(3)],
        "authenticated, not yet starting"
    );

    registry.request(begin(4, Method::Password)).unwrap();
    registry.helper(4, success());
    registry
        .request(Request::StartSession { generation: 4 })
        .unwrap();
    assert_eq!(registry.greeter_gone(), []);
    assert_eq!(registry.winner().unwrap().generation, 4);
    assert_eq!(registry.shutdown(), [kill(4)]);
    assert_eq!(registry.winner(), None);
}
