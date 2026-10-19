use super::*;
use crate::dm::protocol::SessionId;
use crate::lock::secret::Password;
use crate::session::snapshot::{Line, Palette};

fn login(text: &str) -> Login {
    Login::parse(text).unwrap()
}

fn entry(id: &str, name: &str) -> SessionEntry {
    SessionEntry {
        id: SessionId::parse(id).unwrap(),
        name: name.into(),
    }
}

fn bridge() -> Bridge {
    Bridge::new(vec![
        entry("niri-uwsm", "Niri (UWSM)"),
        entry("gnome", "GNOME"),
    ])
}

fn choice(login: &str, index: usize) -> Choice<'_> {
    Choice {
        login,
        session: "",
        index,
    }
}

fn worker(method: Method, generation: u32) -> WorkerId {
    WorkerId { method, generation }
}

fn secret(text: &str) -> Password {
    let mut password = Password::new();
    for c in text.chars() {
        assert!(password.push(c));
    }
    password
}

fn model(id: WorkerId, report: Report) -> Step {
    Step::Model(EventKind::Worker(id, report))
}

#[test]
fn a_spawned_worker_begins_a_conversation_for_the_current_user_and_session() {
    let mut bridge = bridge();
    assert_eq!(
        bridge.effect(
            Effect::Spawn(worker(Method::Password, 3)),
            Some(choice("nathan", 1))
        ),
        [Step::Send(Request::Begin {
            generation: 3,
            method: Method::Password,
            login: login("nathan"),
            session: SessionId::parse("gnome").unwrap(),
        })]
    );
    assert_eq!(
        bridge.effect(
            Effect::Spawn(worker(Method::Fingerprint, 4)),
            Some(choice("nathan", 0))
        ),
        [Step::Send(Request::Begin {
            generation: 4,
            method: Method::Fingerprint,
            login: login("nathan"),
            session: SessionId::parse("niri-uwsm").unwrap(),
        })]
    );
}

#[test]
fn the_other_conversation_effects_become_their_requests() {
    let mut bridge = bridge();
    assert_eq!(
        bridge.effect(Effect::Kill(worker(Method::Fingerprint, 4)), None),
        [Step::Send(Request::Cancel { generation: 4 })]
    );
    assert_eq!(
        bridge.effect(
            Effect::Answer {
                to: 3,
                secret: secret("hunter2")
            },
            None
        ),
        [Step::Send(Request::Answer {
            generation: 3,
            secret: Secret::new(secret("hunter2")),
        })]
    );
    // The fingerprint won: the session is started for that conversation.
    assert_eq!(
        bridge.effect(
            Effect::Unlock {
                by: worker(Method::Fingerprint, 4)
            },
            None
        ),
        [Step::Send(Request::StartSession { generation: 4 })]
    );
}

#[test]
fn each_power_occasion_maps_to_its_own() {
    let mut bridge = bridge();
    for (leave, expected) in [
        (Leave::Suspend, PowerOccasion::Suspend),
        (Leave::Hibernate, PowerOccasion::Hibernate),
        (Leave::Restart, PowerOccasion::Restart),
        (Leave::Shutdown, PowerOccasion::Shutdown),
    ] {
        assert_eq!(
            bridge.effect(Effect::Power(leave), None),
            [Step::Send(Request::Power { occasion: expected })]
        );
    }
}

#[test]
fn effects_that_are_the_edges_own_say_nothing_to_the_display_manager() {
    let mut bridge = bridge();
    for effect in [
        Effect::DisplayOff,
        Effect::SampleChips,
        Effect::ProbeShellOverlay,
        Effect::CloseShellSurfaces,
        Effect::Exit(crate::lock::ExitReason::Unlocked),
    ] {
        assert_eq!(bridge.effect(effect, None), []);
    }
}

#[test]
fn a_name_the_display_manager_cannot_hold_gets_a_conversation_that_always_refuses() {
    let mut bridge = bridge();
    let password = worker(Method::Password, 5);
    // Uppercase is outside the protocol's login rules, but the screen allows any name.
    assert_eq!(
        bridge.effect(Effect::Spawn(password), Some(choice("Bob", 0))),
        [model(password, Report::Prompt { echo: false })]
    );
    assert_eq!(
        bridge.effect(
            Effect::Answer {
                to: 5,
                secret: secret("anything")
            },
            None
        ),
        [model(password, Report::Finished(PamOutcome::Rejected))]
    );
    // The refused conversation was local all along: a kill has nobody to cancel.
    let again = worker(Method::Password, 6);
    bridge.effect(Effect::Spawn(again), Some(choice("Bob", 0)));
    assert_eq!(bridge.effect(Effect::Kill(again), None), []);
    let finger = worker(Method::Fingerprint, 7);
    assert_eq!(
        bridge.effect(Effect::Spawn(finger), Some(choice("Bob", 0))),
        [model(finger, Report::Finished(PamOutcome::Unavailable))]
    );
}

#[test]
fn a_session_the_list_does_not_have_is_refused_instead_of_guessed() {
    let mut bridge = bridge();
    let password = worker(Method::Password, 1);
    assert_eq!(
        bridge.effect(Effect::Spawn(password), Some(choice("nathan", 9))),
        [model(password, Report::Prompt { echo: false })]
    );
}

#[test]
fn a_look_swap_asks_about_the_user_and_only_the_latest_answer_counts() {
    let mut bridge = bridge();
    let user = |name: &str| Username::parse(name).unwrap();
    assert_eq!(
        bridge.effect(Effect::SwapLook(user("alyssa")), None),
        [Step::Send(Request::SelectUser {
            login: login("alyssa")
        })]
    );
    bridge.effect(Effect::SwapLook(user("bob")), None);
    let reply = |name: &str| Event::User {
        login: login(name),
        display_name: Some(name.to_uppercase()),
        fingerprint: name == "bob",
    };
    // Alyssa's reply came late: she is no longer the user.
    assert_eq!(bridge.event(reply("alyssa")), Ok(vec![]));
    assert_eq!(
        bridge.event(reply("bob")),
        Ok(vec![Step::Look(Answer {
            login: "bob".into(),
            display_name: Some("BOB".into()),
            fingerprint: true,
        })])
    );
    // And a reply is only ever answered once.
    assert_eq!(bridge.event(reply("bob")), Ok(vec![]));
}

#[test]
fn the_first_user_is_asked_about_at_start_up() {
    let mut bridge = bridge();
    assert_eq!(
        bridge.ask(&login("nathan")),
        Request::SelectUser {
            login: login("nathan")
        }
    );
    let steps = bridge
        .event(Event::User {
            login: login("nathan"),
            display_name: None,
            fingerprint: false,
        })
        .unwrap();
    assert_eq!(steps.len(), 1);
}

#[test]
fn a_name_that_cannot_be_held_gets_the_neutral_look_at_once() {
    let mut bridge = bridge();
    assert_eq!(
        bridge.effect(Effect::SwapLook(Username::parse("Bob").unwrap()), None),
        [Step::Look(Answer {
            login: "Bob".into(),
            display_name: None,
            fingerprint: false,
        })]
    );
    // No reply is awaited for it.
    assert_eq!(
        bridge.event(Event::User {
            login: login("bob"),
            display_name: None,
            fingerprint: true
        }),
        Ok(vec![])
    );
}

#[test]
fn the_display_managers_conversation_events_reach_the_model() {
    let mut bridge = bridge();
    assert_eq!(
        bridge.event(Event::Report {
            generation: 8,
            method: Method::Fingerprint,
            report: Report::Finished(PamOutcome::Success),
        }),
        Ok(vec![model(
            worker(Method::Fingerprint, 8),
            Report::Finished(PamOutcome::Success)
        )])
    );
    assert_eq!(
        bridge.event(Event::SessionStarted),
        Ok(vec![Step::Model(EventKind::SessionStarted)])
    );
}

#[test]
fn every_failure_to_start_a_session_has_its_own_hint() {
    let mut bridge = bridge();
    let mut hints = Vec::new();
    for reason in [
        SessionFailure::Refused,
        SessionFailure::SpawnFailed,
        SessionFailure::UnknownSession,
        SessionFailure::HelperCrashed,
        SessionFailure::Exited,
    ] {
        let steps = bridge.event(Event::SessionFailed { reason }).unwrap();
        let [Step::Model(EventKind::SessionFailed(text))] = &steps[..] else {
            panic!("{steps:?}");
        };
        assert!(!text.is_empty());
        hints.push(text.clone());
    }
    hints.sort();
    hints.dedup();
    assert_eq!(hints.len(), 5);
}

#[test]
fn a_second_init_is_a_violation() {
    let mut bridge = bridge();
    assert_eq!(
        bridge.event(Event::Init {
            last_login: None,
            last_session: None,
            sessions: vec![],
        }),
        Err(Violation::SecondInit)
    );
}

#[test]
fn sessions_with_one_name_are_told_apart_by_their_id() {
    let sessions = [
        entry("niri-uwsm", "Niri"),
        entry("niri", "Niri"),
        entry("gnome", "GNOME"),
    ];
    assert_eq!(
        session_names(&sessions),
        ["Niri (niri-uwsm)", "Niri (niri)", "GNOME"]
    );
}

#[test]
fn the_first_login_is_the_first_human_with_a_real_shell() {
    let passwd = "\
root:x:0:0:root:/root:/bin/bash
daemon:x:1:1::/:/usr/bin/nologin
build:x:999:999::/var/lib/build:/bin/bash
robot:x:1000:1000::/home/robot:/usr/bin/nologin
Mixed:x:1001:1001::/home/Mixed:/bin/bash
broken line
nathan:x:1002:1002:Nathan,,,:/home/nathan:/bin/zsh
alyssa:x:1003:1003::/home/alyssa:/bin/bash
nobody:x:65534:65534::/:/usr/bin/nologin
";
    assert_eq!(first_human(passwd), Some(login("nathan")));
    assert_eq!(first_human("root:x:0:0::/root:/bin/sh\n"), None);
    assert_eq!(first_human(""), None);
}

fn palette_bytes() -> Vec<u8> {
    let preset = &crate::theme::presets().unwrap()[0];
    serde_json::to_vec_pretty(&preset.palette).unwrap()
}

fn snapshot_of(name: &str) -> snapshot::Look {
    let mut look = snapshot::Look::empty(login(name));
    look.full_name = Line::parse("Alyssa Park");
    look.palette = Some(Palette::parse(palette_bytes()).unwrap());
    look
}

fn base() -> message::Context {
    message::Context {
        hour: 9,
        seed: 7,
        ..message::Context::default()
    }
}

fn answer(display_name: Option<&str>, fingerprint: bool) -> Answer {
    Answer {
        login: "alyssa".into(),
        display_name: display_name.map(Into::into),
        fingerprint,
    }
}

#[test]
fn a_snapshot_gives_the_users_palette_name_greeting_and_farewells() {
    let look = snapshot_of("alyssa");
    let user = user_look(&answer(None, true), Some(&look), &base());
    assert_eq!(user.login, "alyssa");
    assert_eq!(user.display.as_deref(), Some("Alyssa Park"));
    assert!(user.fingerprint);
    let greeting = user.greeting.expect("greetings are on by default");
    assert!(!greeting.normal.is_empty());
    let farewells = user.farewells.expect("farewells are on by default");
    assert!(!farewells.shutdown.is_empty() && !farewells.restart.is_empty());
    let (colors, error) = palette(Some(&look));
    let expected = look.palette.as_ref().unwrap();
    assert_eq!((colors, error), (expected.colors(), expected.error_color()));
}

#[test]
fn the_display_managers_name_wins_over_the_snapshots() {
    let look = snapshot_of("alyssa");
    let user = user_look(&answer(Some("A. Park"), false), Some(&look), &base());
    assert_eq!(user.display.as_deref(), Some("A. Park"));
}

#[test]
fn the_snapshots_settings_switch_the_greeting_and_the_farewells_off() {
    let mut look = snapshot_of("alyssa");
    look.settings = Some(crate::session::settings::SessionSettings {
        greeting: false,
        farewell: false,
        ..Default::default()
    });
    let user = user_look(&answer(None, false), Some(&look), &base());
    assert_eq!((user.greeting, user.farewells), (None, None));
    assert!(!user.fingerprint);
}

#[test]
fn the_greeting_never_repeats_the_users_last_one() {
    let mut look = snapshot_of("alyssa");
    // Whatever was shown last is the line this user's next login screen avoids.
    let first = user_look(&answer(None, false), Some(&look), &base())
        .greeting
        .unwrap()
        .normal;
    look.last_greeting = Line::parse(&first);
    for seed in 0..64 {
        let context = message::Context { seed, ..base() };
        let next = user_look(&answer(None, false), Some(&look), &context)
            .greeting
            .unwrap()
            .normal;
        assert_ne!(next, first, "seed {seed}");
    }
}

#[test]
fn a_user_with_no_snapshot_gets_the_default_theme_and_the_neutral_look() {
    let user = user_look(&answer(Some("Alyssa"), true), None, &base());
    assert_eq!(
        user,
        UserLook {
            login: "alyssa".into(),
            display: Some("Alyssa".into()),
            greeting: None,
            farewells: None,
            fingerprint: true,
        }
    );
    let (colors, error) = palette(None);
    assert_eq!(colors, crate::config::ThemeColors::default());
    assert_eq!(error, crate::session::host::FALLBACK_ERROR);
    // A snapshot without a palette paints the default theme too.
    let bare = snapshot::Look::empty(login("alyssa"));
    assert_eq!(palette(Some(&bare)), palette(None));
}
