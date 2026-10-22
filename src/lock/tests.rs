use std::time::{Duration, Instant};

use gpui::{px, size};

use super::pam::{Method, Notice, PamOutcome, Report, WorkerId};
use super::*;
use crate::session::transition::{Layout, Pose, dial_layout, shell_layout};
use crate::wall_clock;

const OUT: OutputId = OutputId(1);
const SIDE: OutputId = OutputId(2);
/// 2026-10-14 13:05:00 UTC, on a minute boundary.
const UNIX0: i64 = 1_791_983_100;

fn settings() -> Settings {
    Settings {
        reduced_motion: false,
        user: "alyssa".into(),
        greeting: Some(GreetingSet {
            short: "That was quick, Alyssa".into(),
            normal: "Welcome back, Alyssa".into(),
            long: "A new day, Alyssa".into(),
        }),
        controls_width: 300.,
        lock_age_origin: None,
        farewell: None,
        login: None,
    }
}

struct Rig {
    lock: LockModel,
    t0: Instant,
}

impl Rig {
    fn new(settings: Settings) -> (Rig, Vec<String>) {
        let t0 = Instant::now();
        let outputs = vec![
            Output {
                id: OUT,
                size: size(px(1920.), px(1080.)),
            },
            Output {
                id: SIDE,
                size: size(px(1280.), px(800.)),
            },
        ];
        let (lock, fx) = LockModel::new(settings, at(t0, 0), outputs);
        (Rig { lock, t0 }, show(&fx))
    }

    fn now(&self, ms: u64) -> Now {
        at(self.t0, ms)
    }

    fn send(&mut self, ms: u64, kind: EventKind) -> Vec<String> {
        let now = self.now(ms);
        show(&self.lock.update(Event { now, kind }))
    }

    fn key(&mut self, ms: u64, key: Key) -> Vec<String> {
        self.send(ms, EventKind::Key(OUT, key))
    }

    fn type_text(&mut self, ms: u64, text: &str) {
        for c in text.chars() {
            assert_eq!(self.key(ms, Key::Char(c)), none());
        }
    }

    fn scene(&self, output: OutputId, ms: u64) -> Scene {
        self.lock.scene(output, self.now(ms)).expect("known output")
    }

    fn panel(&self, ms: u64) -> Panel {
        match self.scene(OUT, ms).role {
            Role::Active(panel) => panel,
            Role::Passive => panic!("the active output is passive at {ms} ms"),
        }
    }

    fn pill(&self, ms: u64) -> Option<Pill> {
        self.panel(ms).pill
    }

    fn hint(&self, ms: u64) -> String {
        match self.panel(ms).line {
            Some(Line::Hint(text)) => text,
            other => panic!("expected a hint at {ms} ms, got {other:?}"),
        }
    }

    fn deadline(&self) -> Option<u64> {
        self.lock
            .next_deadline()
            .map(|d| d.duration_since(self.t0).as_millis() as u64)
    }
}

fn at(t0: Instant, ms: u64) -> Now {
    Now {
        mono: t0 + Duration::from_millis(ms),
        unix: UNIX0 + (ms / 1000) as i64,
    }
}

fn none() -> Vec<String> {
    Vec::new()
}

fn show(fx: &[Effect]) -> Vec<String> {
    let id = |id: &WorkerId| {
        let method = match id.method {
            Method::Password => "password",
            Method::Fingerprint => "fingerprint",
        };
        format!("{method} {}", id.generation)
    };
    fx.iter()
        .map(|effect| match effect {
            Effect::Spawn(w) => format!("spawn {}", id(w)),
            Effect::Kill(w) => format!("kill {}", id(w)),
            Effect::Answer { to, secret } => {
                format!("answer password {to} with {} chars", secret.char_count())
            }
            Effect::DisplayOff => "display off".into(),
            Effect::SampleChips => "sample chips".into(),
            Effect::ProbeShellOverlay => "probe shell overlay".into(),
            Effect::CloseShellSurfaces => "close shell surfaces".into(),
            Effect::SwapLook(user) => format!("swap look {user}"),
            Effect::Unlock { .. } => "unlock".into(),
            Effect::Power(leave) => format!("power {leave:?}"),
            Effect::Exit(reason) => format!("exit {reason:?}"),
        })
        .collect()
}

fn pw(generation: u32, report: Report) -> EventKind {
    EventKind::Worker(
        WorkerId {
            method: Method::Password,
            generation,
        },
        report,
    )
}

fn fp(generation: u32, report: Report) -> EventKind {
    EventKind::Worker(
        WorkerId {
            method: Method::Fingerprint,
            generation,
        },
        report,
    )
}

fn prompt() -> Report {
    Report::Prompt { echo: false }
}

fn hint(text: &str, error: bool) -> Report {
    Report::Notice(Notice::Hint {
        text: text.into(),
        error,
    })
}

fn finished(outcome: PamOutcome) -> Report {
    Report::Finished(outcome)
}

/// Locked at 0 ms with password worker 1 and fingerprint worker 2.
fn locked_with(settings: Settings) -> Rig {
    let (mut rig, fx) = Rig::new(settings);
    assert_eq!(fx, ["close shell surfaces", "sample chips"]);
    assert_eq!(
        rig.send(0, EventKind::Locked),
        ["spawn password 1", "spawn fingerprint 2"]
    );
    rig
}

fn locked() -> Rig {
    locked_with(settings())
}

/// Locked, with the password worker waiting in its prompt.
fn prompting() -> Rig {
    let mut rig = locked();
    assert_eq!(rig.send(10, pw(1, prompt())), none());
    rig
}

#[test]
fn enter_before_the_prompt_is_answered_on_the_prompt() {
    let mut rig = locked();
    rig.type_text(100, "hunter2");
    assert_eq!(rig.key(150, Key::Enter), none());
    let pill = rig.pill(150).unwrap();
    assert_eq!((pill.dots, pill.tone), (7, Tone::Muted));
    assert_eq!(
        rig.send(400, pw(1, prompt())),
        ["answer password 1 with 7 chars"]
    );
    assert_eq!(rig.pill(400).unwrap().dots, 7);
}

#[test]
fn enter_on_the_prompt_answers_at_once_and_ignores_keys_while_checking() {
    let mut rig = prompting();
    assert_eq!(rig.key(90, Key::Enter), none());
    rig.type_text(100, "hunter2");
    assert_eq!(rig.key(200, Key::Enter), ["answer password 1 with 7 chars"]);
    rig.type_text(300, "xyz");
    assert_eq!(rig.key(300, Key::Enter), none());
    assert_eq!(rig.pill(300).unwrap().dots, 7);
}

#[test]
fn a_rejection_shakes_clears_the_buffer_and_respawns_a_fresh_generation() {
    let mut rig = prompting();
    rig.type_text(100, "hunter2");
    assert_eq!(rig.key(200, Key::Enter), ["answer password 1 with 7 chars"]);
    assert_eq!(
        rig.send(2_300, pw(1, finished(PamOutcome::Rejected))),
        ["spawn password 3"]
    );

    let shaking = rig.scene(OUT, 2_350);
    assert!(shaking.animating);
    let Role::Active(panel) = shaking.role else {
        panic!("passive")
    };
    let pill = panel.pill.unwrap();
    assert_eq!((pill.dots, pill.tone), (0, Tone::Wrong));
    assert_ne!(pill.offset_x, px(0.));
    assert_eq!(panel.dial.ring.tone, Tone::Wrong);
    assert_eq!(
        panel.line,
        Some(Line::Hint("That wasn't it. Try again.".into()))
    );
    assert_eq!(rig.deadline(), Some(2_700));

    let rested = rig.scene(OUT, 2_700);
    assert!(!rested.animating);
    assert_eq!(rig.pill(2_700), None);

    assert_eq!(rig.send(2_800, pw(3, prompt())), none());
    assert_eq!(rig.hint(2_800), "That wasn't it. Try again.");
    rig.type_text(3_000, "right");
    assert_eq!(rig.hint(3_000), "The sensor still works");
    assert_eq!(
        rig.key(3_100, Key::Enter),
        ["answer password 3 with 5 chars"]
    );
}

#[test]
fn faillock_lock_then_remaining_time_pauses_with_no_worker_until_the_pause_ends() {
    let mut rig = locked();
    rig.type_text(50, "abc");
    assert_eq!(
        rig.send(100, pw(1, Report::Notice(Notice::PasswordLocked))),
        none()
    );
    let pill = rig.pill(100).unwrap();
    assert_eq!(
        (pill.dots, pill.dashed, pill.countdown_minutes),
        (0, true, None)
    );
    assert_eq!(rig.hint(100), "Password paused. The sensor still works");

    let remaining = Duration::from_secs(600);
    assert_eq!(
        rig.send(
            150,
            pw(1, Report::Notice(Notice::PasswordPaused { remaining }))
        ),
        ["kill password 1"]
    );
    assert_eq!(rig.pill(150).unwrap().countdown_minutes, Some(10));
    assert_eq!(rig.pill(60_149).unwrap().countdown_minutes, Some(10));
    assert_eq!(rig.pill(60_150).unwrap().countdown_minutes, Some(9));

    rig.type_text(200, "def");
    assert_eq!(rig.key(200, Key::Enter), none());
    assert_eq!(rig.pill(200).unwrap().dots, 0);
    assert_eq!(rig.send(300, pw(1, prompt())), none());

    // The display went off while paused; the pause end is still honoured.
    assert_eq!(
        rig.send(600_150, EventKind::Timer),
        ["display off", "kill fingerprint 2", "spawn password 3"]
    );
    assert_eq!(
        rig.key(600_200, Key::Char('x')),
        ["spawn fingerprint 4", "sample chips"]
    );
    assert_eq!(rig.send(600_300, pw(3, prompt())), none());
    assert_eq!(
        rig.key(600_400, Key::Enter),
        ["answer password 3 with 1 chars"]
    );
}

#[test]
fn fingerprint_stays_live_during_a_password_pause() {
    let mut rig = locked();
    let remaining = Duration::from_secs(60);
    assert_eq!(
        rig.send(
            100,
            pw(1, Report::Notice(Notice::PasswordPaused { remaining }))
        ),
        ["kill password 1"]
    );
    assert_eq!(rig.send(500, fp(2, finished(PamOutcome::Success))), ["probe shell overlay"]);
    assert!(matches!(rig.panel(500).line, Some(Line::Greeting { .. })));
}

#[test]
fn a_fingerprint_win_kills_the_password_worker() {
    let mut rig = prompting();
    rig.type_text(100, "hunt");
    assert_eq!(
        rig.send(1_000, fp(2, finished(PamOutcome::Success))),
        ["kill password 1", "probe shell overlay"]
    );
    assert_eq!(
        rig.send(1_100, pw(1, finished(PamOutcome::Success))),
        none()
    );
    assert_eq!(rig.pill(1_100), None);
}

#[test]
fn a_password_win_kills_the_fingerprint_worker() {
    let mut rig = prompting();
    rig.type_text(100, "hunter2");
    rig.key(200, Key::Enter);
    assert_eq!(
        rig.send(2_000, pw(1, finished(PamOutcome::Success))),
        ["kill fingerprint 2", "probe shell overlay"]
    );
    assert_eq!(
        rig.send(2_100, fp(2, finished(PamOutcome::Success))),
        none()
    );
}

#[test]
fn an_immediate_fingerprint_unavailable_is_never_retried() {
    let mut rig = locked();
    assert_eq!(
        rig.send(100, fp(2, finished(PamOutcome::Unavailable))),
        none()
    );
    assert_eq!(rig.hint(100), "Start typing to unlock");
    assert_eq!(rig.panel(100).dial.ring.tone, Tone::Muted);
    assert_eq!(rig.send(200, EventKind::Pointer(OUT)), none());
    assert_eq!(rig.send(35_200, EventKind::Timer), ["display off"]);
    assert_eq!(rig.key(40_000, Key::Char('a')), none());
    assert_eq!(rig.send(40_100, EventKind::Resumed), none());
}

#[test]
fn a_fingerprint_timeout_while_listening_is_rearmed() {
    let mut rig = locked();
    let place = "Place your right index finger on the fingerprint reader";
    assert_eq!(rig.send(100, fp(2, hint(place, false))), none());
    assert_eq!(rig.hint(100), "Touch the sensor, or just start typing");
    assert_eq!(
        rig.send(3_000, fp(2, hint("Verification timed out", false))),
        none()
    );
    assert!(!rig.scene(OUT, 3_000).animating);
    assert_eq!(
        rig.send(3_001, fp(2, finished(PamOutcome::Unavailable))),
        ["spawn fingerprint 3"]
    );
    assert_eq!(rig.send(3_100, fp(3, hint(place, false))), none());
}

#[test]
fn a_fingerprint_error_hint_flashes_the_ring_and_becomes_the_hint() {
    let mut rig = locked();
    rig.send(100, fp(2, hint("Place your finger", false)));
    assert_eq!(
        rig.send(1_000, fp(2, hint("Failed to match fingerprint", true))),
        none()
    );
    let flashing = rig.scene(OUT, 1_100);
    assert!(flashing.animating);
    assert_eq!(rig.panel(1_100).dial.ring.tone, Tone::Wrong);
    assert_eq!(rig.hint(1_100), "Failed to match fingerprint");
    assert_eq!(rig.deadline(), Some(1_300));
    assert_eq!(rig.panel(1_300).dial.ring.tone, Tone::Normal);
    assert!(!rig.scene(OUT, 1_300).animating);
}

#[test]
fn fingerprint_exhaustion_waits_for_activity() {
    let mut rig = locked();
    assert_eq!(rig.send(100, fp(2, finished(PamOutcome::MaxTries))), none());
    assert_eq!(rig.panel(100).dial.ring.tone, Tone::Muted);
    assert_eq!(
        rig.send(4_000, EventKind::Pointer(OUT)),
        ["spawn fingerprint 3"]
    );
    assert_eq!(rig.send(4_100, EventKind::Pointer(OUT)), none());
}

#[test]
fn a_fingerprint_prompt_is_killed_and_never_answered() {
    let mut rig = locked();
    assert_eq!(rig.send(100, fp(2, prompt())), ["kill fingerprint 2"]);
    rig.type_text(200, "hunter2");
    assert_eq!(rig.key(300, Key::Enter), none());
    assert_eq!(
        rig.send(400, pw(1, prompt())),
        ["answer password 1 with 7 chars"]
    );
    assert_eq!(rig.send(500, EventKind::Pointer(OUT)), none());
}

#[test]
fn display_off_disarms_the_sensor_and_activity_rearms_it() {
    let mut rig = locked();
    rig.send(100, fp(2, hint("Place your finger", false)));
    assert_eq!(rig.deadline(), Some(5_000));
    assert_eq!(rig.send(5_000, EventKind::Timer), none());
    let settled = rig.scene(OUT, 5_000);
    assert_eq!((settled.dim, settled.animating), (0.45, false));
    assert_eq!(rig.deadline(), Some(35_000));
    assert_eq!(
        rig.send(35_000, EventKind::Timer),
        ["display off", "kill fingerprint 2"]
    );
    let dark = rig.scene(OUT, 35_000);
    assert_eq!(
        (dark.dim, dark.role, dark.clock.as_str()),
        (0., Role::Passive, "")
    );
    assert_eq!(
        rig.send(50_000, EventKind::Pointer(OUT)),
        ["spawn fingerprint 3"]
    );
    let lit = rig.scene(OUT, 50_000);
    assert_eq!((lit.dim, lit.clock), (1., wall_clock::dial_time(UNIX0)));
    assert_eq!(rig.deadline(), Some(55_000));
}

#[test]
fn the_display_off_deadline_set_has_no_minute_tick() {
    let mut rig = prompting();
    assert_eq!(rig.deadline(), Some(5_000));
    rig.send(35_000, EventKind::Timer);
    assert_eq!(rig.deadline(), None);

    let mut paused = locked();
    let remaining = Duration::from_secs(600);
    paused.send(
        100,
        pw(1, Report::Notice(Notice::PasswordPaused { remaining })),
    );
    paused.send(35_100, EventKind::Timer);
    assert_eq!(paused.deadline(), Some(600_100));
}

#[test]
fn the_minute_tick_is_a_deadline_while_lit_and_samples_chips() {
    let mut rig = prompting();
    rig.send(4_000, EventKind::Pointer(OUT));
    rig.send(8_000, EventKind::Pointer(OUT));
    rig.send(12_000, EventKind::Pointer(OUT));
    rig.send(16_000, EventKind::Pointer(OUT));
    rig.send(20_000, EventKind::Pointer(OUT));
    rig.send(24_000, EventKind::Pointer(OUT));
    rig.send(56_000, EventKind::Pointer(OUT));
    assert_eq!(rig.deadline(), Some(60_000));
    let before = rig.scene(OUT, 59_999);
    assert_eq!(rig.send(60_000, EventKind::Timer), ["sample chips"]);
    let after = rig.scene(OUT, 60_000);
    assert_eq!(before.clock, wall_clock::dial_time(UNIX0));
    assert_eq!(after.clock, wall_clock::dial_time(UNIX0 + 60));
    assert_ne!(before.clock, after.clock);
}

#[test]
fn a_stale_generation_is_ignored() {
    let mut rig = prompting();
    rig.type_text(100, "nope");
    rig.key(200, Key::Enter);
    assert_eq!(
        rig.send(2_000, pw(1, finished(PamOutcome::Rejected))),
        ["spawn password 3"]
    );
    rig.type_text(2_100, "again");
    rig.key(2_200, Key::Enter);
    assert_eq!(rig.send(2_300, pw(1, prompt())), none());
    assert_eq!(
        rig.send(2_300, pw(1, finished(PamOutcome::Success))),
        none()
    );
    assert_eq!(rig.pill(2_300).unwrap().dots, 5);
    assert_eq!(
        rig.send(2_400, pw(3, prompt())),
        ["answer password 3 with 5 chars"]
    );
}

#[test]
fn finished_before_locked_is_refused_and_after_locked_is_ended() {
    let (mut refused, _) = Rig::new(settings());
    assert_eq!(refused.send(50, EventKind::Finished), ["exit Refused"]);
    assert_eq!(refused.send(60, EventKind::Locked), none());
    assert_eq!(refused.deadline(), None);

    let mut ended = locked();
    assert_eq!(
        ended.send(50, EventKind::Finished),
        ["kill password 1", "kill fingerprint 2", "exit Ended"]
    );
    assert_eq!(ended.send(60, EventKind::Finished), none());
    assert_eq!(ended.key(70, Key::Enter), none());
}

#[test]
fn typing_before_locked_is_buffered_and_survives_the_lock() {
    let (mut rig, _) = Rig::new(settings());
    rig.type_text(20, "abc");
    assert_eq!(rig.pill(20).unwrap().dots, 3);
    assert_eq!(
        rig.send(100, EventKind::Locked),
        ["spawn password 1", "spawn fingerprint 2"]
    );
    assert_eq!(rig.pill(100).unwrap().dots, 3);
    assert_eq!(rig.send(200, pw(1, prompt())), none());
    assert_eq!(rig.key(300, Key::Enter), ["answer password 1 with 3 chars"]);
}

#[test]
fn a_timer_or_chips_sample_that_changes_nothing_leaves_every_scene_equal() {
    let mut rig = prompting();
    let chips = HostChips {
        battery: Some("82%".into()),
        network: Some("home".into()),
    };
    rig.send(100, EventKind::Chips(chips.clone()));
    rig.send(5_000, EventKind::Timer);
    let before = [rig.scene(OUT, 5_000), rig.scene(SIDE, 5_000)];
    assert_eq!(rig.send(5_500, EventKind::Timer), none());
    assert_eq!(rig.send(6_000, EventKind::Chips(chips)), none());
    assert_eq!(rig.send(6_500, pw(9, prompt())), none());
    assert_eq!([rig.scene(OUT, 7_000), rig.scene(SIDE, 7_000)], before);

    rig.send(
        7_000,
        EventKind::Chips(HostChips {
            battery: Some("81%".into()),
            network: Some("home".into()),
        }),
    );
    assert_ne!(rig.scene(OUT, 7_000), before[0]);
    assert_eq!(rig.scene(SIDE, 7_000), before[1]);
}

#[test]
fn unlock_runs_ring_fill_greeting_then_opening_then_unlocks() {
    let mut rig = prompting();
    rig.type_text(100, "hunter2");
    rig.key(200, Key::Enter);
    assert_eq!(
        rig.send(1_000, pw(1, finished(PamOutcome::Success))),
        ["kill fingerprint 2", "probe shell overlay"]
    );
    assert_eq!(rig.deadline(), Some(1_300));
    assert!(rig.panel(1_150).dial.ring.fill > 0. && rig.panel(1_150).dial.ring.fill < 1.);
    assert_eq!(rig.send(1_300, EventKind::Timer), none());
    assert_eq!(rig.panel(1_300).dial.ring.fill, 1.);
    assert_eq!(rig.deadline(), Some(1_950));
    assert_eq!(
        rig.panel(1_500).line,
        Some(Line::Greeting {
            text: "That was quick, Alyssa".into(),
            alpha: 1.
        })
    );
    assert_eq!(
        rig.scene(OUT, 1_500).pose,
        Pose::Moving(dial_layout(size(px(1920.), px(1080.))))
    );
    assert!(!rig.scene(SIDE, 1_500).animating);
    assert_eq!(rig.send(1_950, EventKind::Timer), none());
    assert_eq!(rig.deadline(), Some(2_400));
    assert!(rig.scene(SIDE, 2_175).animating);
    let Pose::Moving(middle) = rig.scene(OUT, 2_175).pose else {
        panic!("morph fades")
    };
    assert!(middle.shell_label > 0. && middle.dial_time > 0.);
    assert!(rig.scene(OUT, 2_175).shell_clock.is_some());
    assert_eq!(rig.panel(2_175).line, None);
    assert_eq!(
        rig.send(2_400, EventKind::Timer),
        ["unlock", "exit Unlocked"]
    );
    assert_eq!(rig.deadline(), None);
    assert_eq!(
        rig.scene(SIDE, 2_400).pose,
        Pose::Moving(shell_layout(size(px(1280.), px(800.)), 300.))
    );
}

#[test]
fn a_key_skips_the_greeting_and_never_cancels_the_unlock() {
    let mut rig = locked();
    assert_eq!(
        rig.send(1_000, fp(2, finished(PamOutcome::Success))),
        ["kill password 1", "probe shell overlay"]
    );
    assert_eq!(rig.key(1_100, Key::Char('q')), none());
    assert_eq!(rig.panel(1_100).line, None);
    assert_eq!(rig.pill(1_100), None);
    assert_eq!(rig.send(1_300, EventKind::Timer), none());
    assert_eq!(rig.deadline(), Some(1_550));
    assert_eq!(rig.key(1_400, Key::Escape), none());
    assert_eq!(rig.send(1_450, EventKind::Pointer(OUT)), none());
    assert_eq!(rig.deadline(), Some(1_550));
    assert_eq!(
        rig.send(1_550, EventKind::Timer),
        ["unlock", "exit Unlocked"]
    );
}

#[test]
fn without_a_greeting_the_opening_starts_at_once() {
    let mut rig = locked_with(Settings {
        greeting: None,
        ..settings()
    });
    rig.send(1_000, fp(2, finished(PamOutcome::Success)));
    assert_eq!(rig.panel(1_000).line, None);
    rig.send(1_300, EventKind::Timer);
    assert_eq!(rig.deadline(), Some(1_450));
    assert_eq!(
        rig.send(1_450, EventKind::Timer),
        ["unlock", "exit Unlocked"]
    );
}

#[test]
fn reduced_motion_fades_the_opening_and_keeps_wrong_to_colour() {
    let mut rig = locked_with(Settings {
        reduced_motion: true,
        ..settings()
    });
    rig.send(10, pw(1, prompt()));
    rig.type_text(100, "nope");
    assert_eq!(rig.panel(100).dial.scale, 0.72);
    assert!(!rig.scene(OUT, 100).animating);
    rig.key(200, Key::Enter);
    rig.send(2_000, pw(1, finished(PamOutcome::Rejected)));
    let wrong = rig.scene(OUT, 2_100);
    assert!(!wrong.animating);
    let pill = rig.pill(2_100).unwrap();
    assert_eq!((pill.tone, pill.offset_x), (Tone::Wrong, px(0.)));
    assert_eq!(rig.hint(2_100), "That wasn't it. Try again.");
    assert_eq!(rig.deadline(), Some(2_400));
    assert_eq!(rig.pill(2_400), None);

    rig.send(3_000, fp(2, finished(PamOutcome::Success)));
    assert_eq!(rig.panel(3_000).dial.ring.fill, 1.);
    rig.key(3_100, Key::Enter);
    let fading = rig.scene(OUT, 3_300);
    assert!(fading.animating);
    assert!(matches!(fading.pose, Pose::Fading { mix, .. } if mix > 0. && mix < 1.));
}

#[test]
fn passive_outputs_carry_no_dots_and_the_pointer_moves_the_active_output() {
    let mut rig = prompting();
    rig.type_text(300, "abc");
    assert_eq!(rig.scene(SIDE, 300).role, Role::Passive);
    let quiet = rig.scene(SIDE, 300);
    rig.type_text(350, "de");
    assert_eq!(rig.scene(SIDE, 350), quiet);

    assert_eq!(rig.send(400, EventKind::Pointer(SIDE)), none());
    assert_eq!(rig.scene(OUT, 500).role, Role::Passive);
    let Role::Active(side) = rig.scene(SIDE, 500).role else {
        panic!("side output should be active")
    };
    assert_eq!(side.pill.unwrap().dots, 5);
}

#[test]
fn passwords_never_reach_debug_output() {
    let now = at(Instant::now(), 0);
    let event = Event {
        now,
        kind: EventKind::Key(OUT, Key::Char('ž')),
    };
    let shown = format!("{event:?} {:?}", Key::Char('ž'));
    assert!(!shown.contains('ž'), "{shown}");
    assert!(
        shown.contains("Key(OutputId(1), Char(<redacted>))"),
        "{shown}"
    );

    let mut rig = prompting();
    rig.type_text(100, "žžžsecret");
    let fx = rig.lock.update(Event {
        now: rig.now(200),
        kind: EventKind::Key(OUT, Key::Enter),
    });
    assert_eq!(
        format!("{fx:?}"),
        "[Answer { to: 1, secret: Password(<redacted>) }]"
    );
}

#[test]
fn a_fingerprint_failure_after_a_wrong_password_replaces_the_wrong_hint() {
    let mut rig = prompting();
    rig.type_text(100, "nope");
    rig.key(200, Key::Enter);
    rig.send(2_000, pw(1, finished(PamOutcome::Rejected)));
    rig.send(2_100, fp(2, hint("Place your finger", false)));
    assert_eq!(rig.hint(2_500), "That wasn't it. Try again.");
    rig.send(3_000, fp(2, hint("Failed to match fingerprint", true)));
    assert_eq!(rig.hint(3_000), "Failed to match fingerprint");
}

#[test]
fn lock_age_counts_whole_minutes_elapsed_from_the_takeover_marker() {
    let rig = locked_with(Settings {
        lock_age_origin: Some(UNIX0 - 2 * 3_600 - 5 * 60 - 30),
        ..settings()
    });
    assert_eq!(
        rig.panel(0).chips.lock_age.as_deref(),
        Some("Locked 2 h 5 min")
    );
    let mut own = locked();
    own.send(100, EventKind::Pointer(OUT));
    assert_eq!(
        own.panel(59_999).chips.lock_age.as_deref(),
        Some("Locked just now")
    );
    assert_eq!(
        own.panel(60_000).chips.lock_age.as_deref(),
        Some("Locked 1 min")
    );
}

#[test]
fn a_minute_boundary_seconds_after_locking_is_not_a_minute_of_lock() {
    let (mut rig, _) = Rig::new(settings());
    rig.send(50_000, EventKind::Locked);
    let age = |ms| rig.panel(ms).chips.lock_age;
    assert_eq!(age(61_000).as_deref(), Some("Locked just now"));
    assert_eq!(age(109_999).as_deref(), Some("Locked just now"));
    assert_eq!(age(110_000).as_deref(), Some("Locked 1 min"));
}

#[test]
fn a_scripted_lock_produces_exactly_these_effects() {
    let remaining = Duration::from_secs(120);
    let script: Vec<(u64, EventKind)> = vec![
        (40, EventKind::Key(OUT, Key::Char('a'))),
        (80, EventKind::Locked),
        (90, fp(2, hint("Place your finger", false))),
        (95, pw(1, prompt())),
        (300, EventKind::Key(OUT, Key::Enter)),
        (2_300, pw(1, finished(PamOutcome::Rejected))),
        (2_400, pw(3, Report::Notice(Notice::PasswordLocked))),
        (
            2_401,
            pw(3, Report::Notice(Notice::PasswordPaused { remaining })),
        ),
        (3_000, fp(2, hint("Failed to match fingerprint", true))),
        (8_000, EventKind::Timer),
        (38_000, EventKind::Timer),
        (60_000, EventKind::Timer),
        (70_000, EventKind::Pointer(SIDE)),
        (72_000, fp(4, hint("Place your finger", false))),
        (73_000, fp(4, finished(PamOutcome::Unavailable))),
        (73_500, fp(2, finished(PamOutcome::Success))),
        (75_000, EventKind::Timer),
        (105_000, EventKind::Timer),
        (122_401, EventKind::Timer),
        (122_500, pw(6, prompt())),
        (130_000, EventKind::Key(SIDE, Key::Char('b'))),
        (130_100, EventKind::Key(SIDE, Key::Enter)),
        (131_000, fp(7, finished(PamOutcome::MaxTries))),
        (131_500, pw(6, finished(PamOutcome::Success))),
        (131_600, EventKind::Key(SIDE, Key::Enter)),
        (131_800, EventKind::Timer),
        (132_050, EventKind::Timer),
        (132_200, EventKind::Finished),
    ];
    let (mut rig, start) = Rig::new(settings());
    let mut log: Vec<String> = start.into_iter().map(|e| format!("0 {e}")).collect();
    for (ms, kind) in script {
        for effect in rig.send(ms, kind) {
            log.push(format!("{ms} {effect}"));
        }
    }
    assert_eq!(
        log,
        [
            "0 close shell surfaces",
            "0 sample chips",
            "80 spawn password 1",
            "80 spawn fingerprint 2",
            "300 answer password 1 with 1 chars",
            "2300 spawn password 3",
            "2401 kill password 3",
            "38000 display off",
            "38000 kill fingerprint 2",
            "70000 spawn fingerprint 4",
            "70000 sample chips",
            "73000 spawn fingerprint 5",
            "105000 display off",
            "105000 kill fingerprint 5",
            "122401 spawn password 6",
            "130000 spawn fingerprint 7",
            "130000 sample chips",
            "130100 answer password 6 with 1 chars",
            "131500 probe shell overlay",
            "132050 unlock",
            "132050 exit Unlocked",
        ]
    );
}

#[test]
fn a_waking_key_after_a_missed_timer_never_turns_the_display_off() {
    let mut rig = prompting();
    assert_eq!(rig.key(40_000, Key::Char('a')), none());
    assert_eq!(rig.scene(OUT, 40_000).dim, 1.);
    assert_eq!(rig.deadline(), Some(40_200));
    assert_eq!(rig.send(40_200, EventKind::Timer), none());
    assert_eq!(rig.deadline(), Some(45_000));
}

#[test]
fn a_worker_that_dies_before_its_verdict_waits_for_enter_and_keeps_the_typing() {
    let mut rig = locked();
    rig.type_text(100, "abc");
    assert_eq!(rig.send(200, pw(1, Report::Crashed)), none());
    assert_eq!(rig.hint(200), "Press Enter to try again");
    assert_eq!(rig.pill(200).unwrap().dots, 3);
    assert_eq!(rig.send(300, EventKind::Timer), none());
    assert_eq!(rig.deadline(), Some(5_100));
    assert_eq!(rig.key(300, Key::Enter), ["spawn password 3"]);
    assert_eq!(
        rig.send(400, pw(3, prompt())),
        ["answer password 3 with 3 chars"]
    );
}

/// The greeting shown after a password unlock `ms` after the lock was confirmed.
fn greeting_after(settings: Settings, ms: u64) -> String {
    let mut rig = locked_with(settings);
    rig.send(10, pw(1, prompt()));
    rig.type_text(100, "hunter2");
    rig.key(200, Key::Enter);
    rig.send(ms, pw(1, finished(PamOutcome::Success)));
    match rig.panel(ms + 500).line {
        Some(Line::Greeting { text, .. }) => text,
        other => panic!("expected a greeting, got {other:?}"),
    }
}

#[test]
fn the_greeting_matches_how_long_the_session_was_locked() {
    assert_eq!(greeting_after(settings(), 90_000), "That was quick, Alyssa");
    assert_eq!(greeting_after(settings(), 120_000), "Welcome back, Alyssa");
    assert_eq!(
        greeting_after(settings(), 3 * 3_600_000),
        "Welcome back, Alyssa"
    );
    assert_eq!(
        greeting_after(settings(), 8 * 3_600_000),
        "A new day, Alyssa"
    );
}

#[test]
fn a_takeover_counts_the_lock_length_from_the_first_lock() {
    let settings = Settings {
        lock_age_origin: Some(UNIX0 - 9 * 3_600),
        ..settings()
    };
    assert_eq!(greeting_after(settings, 1_000), "A new day, Alyssa");
}

#[test]
fn greeting_buckets_follow_the_message_rules_at_their_edges() {
    let set = GreetingSet {
        short: "short".into(),
        normal: "normal".into(),
        long: "long".into(),
    };
    let pick = |seconds: u64| set.pick(Duration::from_secs(seconds));
    assert_eq!(pick(0), "short");
    assert_eq!(pick(119), "short");
    assert_eq!(pick(120), "normal");
    assert_eq!(pick(8 * 3_600 - 1), "normal");
    assert_eq!(pick(8 * 3_600), "long");
}

#[test]
fn chosen_greetings_come_from_their_own_bucket() {
    use crate::session::Occasion;
    use crate::session::message::{self, Condition};
    let lines = |locked_for: Option<Duration>| -> Vec<(String, Condition)> {
        let context = message::Context {
            hour: 9,
            locked_for,
            ..message::Context::default()
        };
        message::eligible(Occasion::Unlock, &context)
            .into_iter()
            .map(|entry| (entry.template.replace("{name}", "Alyssa"), entry.when))
            .collect()
    };
    let short = lines(Some(Duration::from_secs(60)));
    let normal = lines(Some(Duration::from_secs(3_600)));
    let long = lines(Some(Duration::from_secs(9 * 3_600)));
    let mut specific = (false, false);
    for seed in 0..64 {
        let base = message::Context {
            hour: 9,
            seed,
            ..message::Context::default()
        };
        let set = GreetingSet::choose(&base, "Alyssa");
        let condition = |pool: &[(String, Condition)], text: &str| {
            pool.iter()
                .find(|(line, _)| line == text)
                .map(|(_, when)| *when)
        };
        let short_when = condition(&short, &set.short).expect("short line is eligible when short");
        assert!(condition(&normal, &set.normal).is_some(), "{}", set.normal);
        let long_when = condition(&long, &set.long).expect("long line is eligible when long");
        assert_ne!(
            condition(&normal, &set.normal),
            Some(Condition::LockedUnder2Min)
        );
        specific.0 |= short_when == Condition::LockedUnder2Min;
        specific.1 |= long_when == Condition::LockedOver8h;
    }
    assert_eq!(specific, (true, true));
}

#[test]
fn the_pause_countdown_wakes_exactly_when_its_minute_changes() {
    let mut rig = locked();
    let remaining = Duration::from_secs(600);
    rig.send(
        150,
        pw(1, Report::Notice(Notice::PasswordPaused { remaining })),
    );
    rig.send(56_000, EventKind::Pointer(OUT));
    assert_eq!(rig.deadline(), Some(60_000));
    rig.send(60_000, EventKind::Timer);
    assert_eq!(rig.deadline(), Some(60_150));
    assert_eq!(rig.pill(60_149).unwrap().countdown_minutes, Some(10));
    assert_eq!(rig.send(60_150, EventKind::Timer), none());
    assert_eq!(rig.pill(60_150).unwrap().countdown_minutes, Some(9));
    assert_eq!(rig.deadline(), Some(61_000));
    rig.send(61_000, EventKind::Timer);
    assert_eq!(rig.deadline(), Some(91_000));

    let mut last = locked();
    let remaining = Duration::from_secs(60);
    last.send(
        150,
        pw(1, Report::Notice(Notice::PasswordPaused { remaining })),
    );
    last.send(4_000, EventKind::Pointer(OUT));
    assert_eq!(last.deadline(), Some(9_000));
    last.send(9_000, EventKind::Timer);
    assert_eq!(last.pill(9_000).unwrap().countdown_minutes, Some(1));
    assert_eq!(last.deadline(), Some(39_000));
}

/// The dial the login screen leaves for a power action: closed, with the ring and text gone.
fn released() -> Layout {
    Layout {
        ring: 0.,
        text: 0.,
        ..dial_layout(size(px(1920.), px(1080.)))
    }
}

/// The dial the login screen holds under the frame colour while the session starts: the closed
/// state with every one of its parts, because the whole scene fades together.
fn handed_over() -> Layout {
    dial_layout(size(px(1920.), px(1080.)))
}

fn closed_dial() -> Layout {
    Layout {
        ring: 0.,
        text: 0.,
        ..dial_layout(size(px(1920.), px(1080.)))
    }
}

#[test]
fn the_first_frame_before_locked_is_the_resting_dial_on_the_first_output() {
    let (rig, fx) = Rig::new(settings());
    assert_eq!(fx, ["close shell surfaces", "sample chips"]);
    let scene = rig.scene(OUT, 0);
    assert_eq!(scene.pose, Pose::Moving(closed_dial()));
    assert_eq!(scene.clock, wall_clock::dial_time(UNIX0));
    assert_eq!(
        (scene.shell_clock, scene.dim, scene.animating),
        (None, 1., false)
    );
    let panel = rig.panel(0);
    assert_eq!(panel.pill, None);
    assert_eq!(
        panel.line,
        Some(Line::Hint("Touch the sensor, or just start typing".into()))
    );
    assert_eq!(
        panel.dial,
        Dial {
            scale: 1.,
            ring: Ring {
                tone: Tone::Normal,
                fill: 0.,
                flash: 0.
            }
        }
    );
    assert_eq!(panel.chips.lock_age, None);
    assert_eq!(rig.scene(SIDE, 0).role, Role::Passive);
    assert_eq!(rig.deadline(), Some(60_000));
}

/// The shell's `Opening` and the reducer's unlock sequence must keep one timeline.
#[test]
fn the_opening_machine_keeps_the_unlock_sequence_timeline() {
    use crate::session::Occasion;
    use crate::session::opening::{
        self, Event as OpeningEvent, EventKind as OpeningKind, Opening,
    };
    use crate::session::settings::SessionSettings;

    assert_eq!(opening::GREETING_IN, model::GREETING_IN);
    assert_eq!(opening::GREETING_OUT, model::GREETING_OUT);
    assert_eq!(opening::OPENING, model::OPENING);
    assert_eq!(opening::RELEASE, model::RELEASE);
    for shell in [false, true] {
        assert_eq!(
            model::greeting(shell),
            opening::GREETING_IN + opening::greeting_hold(shell) + opening::GREETING_OUT
        );
    }

    let opening_deadlines = |greeting: bool, key_at: Option<u64>| {
        let t0 = Instant::now();
        let line = greeting.then(|| "Welcome back, Alyssa".to_owned());
        let (mut opening, _) =
            Opening::begin(Occasion::Unlock, &SessionSettings::default(), line, t0);
        opening.update(OpeningEvent {
            now: t0,
            kind: OpeningKind::Presented,
        });
        let mut out = Vec::new();
        if let Some(ms) = key_at {
            opening.update(OpeningEvent {
                now: t0 + Duration::from_millis(ms),
                kind: OpeningKind::Input,
            });
        }
        while let Some(at) = opening.next_deadline() {
            out.push(at.duration_since(t0).as_millis() as u64);
            opening.update(OpeningEvent {
                now: at,
                kind: OpeningKind::Tick,
            });
        }
        out
    };
    let model_deadlines = |greeting: bool, key_at: Option<u64>| {
        let mut rig = locked_with(Settings {
            greeting: greeting.then(|| settings().greeting).flatten(),
            ..settings()
        });
        rig.send(1_000, fp(2, finished(PamOutcome::Success)));
        if let Some(ms) = key_at {
            rig.key(1_000 + ms, Key::Char('q'));
        }
        let mut out = Vec::new();
        while let Some(deadline) = rig.deadline() {
            // The ring fill is the locker's own 300 ms step and is not part of the opening.
            if deadline != 1_300 {
                out.push(deadline - 1_000);
            }
            if rig.send(deadline, EventKind::Timer).iter().any(|e| e == "exit Unlocked") {
                break;
            }
        }
        out
    };
    for (greeting, key_at) in [(true, None), (false, None), (true, Some(400))] {
        assert_eq!(
            opening_deadlines(greeting, key_at),
            model_deadlines(greeting, key_at),
            "greeting {greeting}, key at {key_at:?}"
        );
    }
}

/// The farewell starts once the dial's parts have faded in.
const F: u64 = 250;

fn farewell_settings() -> Settings {
    Settings {
        farewell: Some("Securing the perimeter".into()),
        ..settings()
    }
}

fn farewell_alpha(rig: &Rig, ms: u64) -> Option<f32> {
    match rig.panel(ms).line {
        Some(Line::Farewell { text, alpha }) => {
            assert_eq!(text, "Securing the perimeter");
            Some(alpha)
        }
        _ => None,
    }
}

#[test]
fn the_farewell_fades_in_holds_and_fades_out_over_the_resting_dial() {
    let rig = locked_with(farewell_settings());
    // It starts once the dial's parts have faded in.
    for ms in [0, 125, 249] {
        assert_eq!(farewell_alpha(&rig, ms), Some(0.), "{ms} ms");
    }
    assert_eq!(farewell_alpha(&rig, F), Some(0.));
    assert!((farewell_alpha(&rig, F + 75).unwrap() - 0.5).abs() < 1e-5);
    for ms in [150, 300, 450, 600, 749] {
        assert_eq!(farewell_alpha(&rig, F + ms), Some(1.), "{ms} ms");
    }
    assert!((farewell_alpha(&rig, F + 825).unwrap() - 0.5).abs() < 1e-5);
    let last = farewell_alpha(&rig, F + 899).unwrap();
    assert!(last > 0. && last < 0.001, "{last}");
    assert_eq!(farewell_alpha(&rig, F + 900), None);
    assert_eq!(rig.hint(F + 900), "Touch the sensor, or just start typing");
    let rising: Vec<_> = (0..=150).map(|ms| farewell_alpha(&rig, F + ms).unwrap()).collect();
    assert!(rising.windows(2).all(|pair| pair[0] <= pair[1]));
    let falling: Vec<_> = (750..900).map(|ms| farewell_alpha(&rig, F + ms).unwrap()).collect();
    assert!(falling.windows(2).all(|pair| pair[0] >= pair[1]));
    let dial = rig.panel(F + 300).dial;
    assert_eq!((dial.scale, dial.ring.tone), (1., Tone::Normal));
    assert_eq!(rig.scene(OUT, F + 300).pose, Pose::Moving(dial_layout(size(px(1920.), px(1080.)))));
}

#[test]
fn the_farewell_animates_only_while_it_shows_and_ends_on_a_deadline() {
    let mut rig = locked_with(farewell_settings());
    assert!(rig.scene(OUT, 0).animating);
    assert!(rig.scene(OUT, F + 899).animating);
    assert!(!rig.scene(OUT, F + 900).animating);
    assert!(!rig.scene(SIDE, 300).animating);
    assert_eq!(rig.scene(SIDE, 300).role, Role::Passive);
    assert_eq!(rig.deadline(), Some(F + 900));
    assert_eq!(rig.send(F + 900, EventKind::Timer), none());
    assert_eq!(rig.deadline(), Some(5_000));
}

#[test]
fn without_a_farewell_the_resting_hint_shows_from_the_first_moment() {
    let rig = locked();
    assert_eq!(rig.hint(0), "Touch the sensor, or just start typing");
    assert!(!rig.scene(OUT, F).animating);
    assert_eq!(rig.deadline(), Some(5_000));
}

#[test]
fn any_key_skips_the_farewell_straight_to_the_resting_hint() {
    let mut rig = locked_with(farewell_settings());
    rig.send(10, pw(1, prompt()));
    assert_eq!(rig.send(F + 300, EventKind::Pointer(OUT)), none());
    assert_eq!(farewell_alpha(&rig, F + 300), Some(1.));
    assert_eq!(rig.key(F + 400, Key::Escape), none());
    assert_eq!(farewell_alpha(&rig, F + 400), None);
    assert_eq!(rig.hint(F + 400), "Touch the sensor, or just start typing");
    assert!(!rig.scene(OUT, F + 400).animating);
    assert_eq!(rig.deadline(), Some(5_000 + F + 400));
}

#[test]
fn a_key_before_the_lock_lands_skips_the_farewell_before_it_starts() {
    let (mut rig, _) = Rig::new(farewell_settings());
    assert_eq!(rig.key(5, Key::Char('q')), none());
    rig.send(10, EventKind::Locked);
    assert_eq!(farewell_alpha(&rig, 20), None);
    assert_eq!(rig.pill(20).unwrap().dots, 1);
}

#[test]
fn the_farewell_does_not_delay_the_workers_or_the_unlock() {
    let (mut rig, _) = Rig::new(farewell_settings());
    assert_eq!(
        rig.send(0, EventKind::Locked),
        ["spawn password 1", "spawn fingerprint 2"]
    );
    rig.send(10, pw(1, prompt()));
    assert_eq!(
        rig.send(100, fp(2, finished(PamOutcome::Success))),
        ["kill password 1", "probe shell overlay"]
    );
    assert_eq!(farewell_alpha(&rig, 100), None);
    assert_eq!(rig.panel(100).line.map(|l| matches!(l, Line::Greeting { .. })), Some(true));
}

#[test]
fn reduced_motion_shows_the_farewell_without_fades_or_frames() {
    let rig = locked_with(Settings {
        reduced_motion: true,
        ..farewell_settings()
    });
    for ms in [0, 75, 450, 899] {
        assert_eq!(farewell_alpha(&rig, ms), Some(1.), "{ms} ms");
        assert!(!rig.scene(OUT, ms).animating, "{ms} ms");
        assert_eq!(rig.scene(OUT, ms).pose, Pose::Moving(dial_layout(size(px(1920.), px(1080.)))));
    }
    assert_eq!(farewell_alpha(&rig, 900), None);
    assert_eq!(rig.deadline(), Some(900));
}

#[test]
fn the_dials_parts_fade_in_over_a_quarter_second_after_locked_and_not_before() {
    let (rig, _) = Rig::new(settings());
    let before = rig.scene(OUT, 5_000);
    assert_eq!(before.pose, Pose::Moving(closed_dial()));
    assert!(!before.animating, "the closed state waits without frames");
    assert_eq!(before.quiet, 1.);
    assert_eq!(rig.scene(SIDE, 5_000).quiet, 1.);

    let rig = locked();
    let ring_and_text = |ms| match rig.scene(OUT, ms).pose {
        Pose::Moving(layout) => (layout.ring, layout.text),
        Pose::Fading { .. } => panic!("morph never fades"),
    };
    assert_eq!(ring_and_text(0), (0., 0.));
    let (ring, text) = ring_and_text(125);
    assert!((ring - 0.5).abs() < 1e-5 && ring == text, "{ring} {text}");
    assert_eq!(ring_and_text(250), (1., 1.));
    let mut last = 0.;
    for ms in 0..=250 {
        let (ring, _) = ring_and_text(ms);
        assert!(ring >= last, "{ms} ms");
        last = ring;
    }
    assert!(rig.scene(OUT, 249).animating);
    assert!(!rig.scene(OUT, 250).animating);
    // Everything else about the picture is the closed state's from the first frame.
    let (mut now, closed) = (rig.scene(OUT, 100), rig.scene(OUT, 0));
    let Pose::Moving(layout) = &mut now.pose else {
        panic!("morph never fades")
    };
    (layout.ring, layout.text) = (0., 0.);
    assert_eq!(now.pose, closed.pose);
    assert_eq!(closed.clock, now.clock);
    // The quieter clock of the other outputs settles in with them.
    assert_eq!(rig.scene(SIDE, 0).quiet, 1.);
    assert!((rig.scene(SIDE, 250).quiet - 0.55).abs() < 1e-6);
    assert!(!rig.scene(SIDE, 100).animating);
}

#[test]
fn a_restarted_locker_and_reduced_motion_show_the_dial_at_once() {
    for restarted in [
        Settings {
            lock_age_origin: Some(UNIX0 - 3_600),
            ..settings()
        },
        Settings {
            reduced_motion: true,
            ..settings()
        },
    ] {
        let (rig, _) = Rig::new(restarted.clone());
        let full = Pose::Moving(dial_layout(size(px(1920.), px(1080.))));
        assert_eq!(rig.scene(OUT, 0).pose, full);
        assert!(!rig.scene(OUT, 0).animating);
        let rig = locked_with(restarted);
        assert_eq!(rig.scene(OUT, 0).pose, full);
        assert!(!rig.scene(OUT, 0).animating);
        assert_eq!(rig.scene(SIDE, 0).quiet, 0.55);
    }
}

/// An unlock at 1,000 ms in which the host answers the probe with `parked`.
fn unlocking_with(parked: Option<bool>, greeting: bool) -> Rig {
    let mut rig = locked_with(Settings {
        greeting: greeting.then(|| settings().greeting).flatten(),
        ..settings()
    });
    assert_eq!(
        rig.send(1_000, fp(2, finished(PamOutcome::Success))),
        ["kill password 1", "probe shell overlay"]
    );
    if let Some(parked) = parked {
        assert_eq!(rig.send(1_000, EventKind::ShellOverlay(parked)), none());
    }
    rig
}

#[test]
fn a_parked_shell_overlay_shortens_the_greeting_and_the_locker_unlocks_without_opening() {
    let mut rig = unlocking_with(Some(true), true);
    // The ring fill is the locker's own 300 ms step, then 150 in, 450 hold and 150 out.
    assert_eq!(rig.deadline(), Some(1_300));
    rig.send(1_300, EventKind::Timer);
    assert_eq!(rig.deadline(), Some(1_750));
    assert_eq!(rig.send(1_749, EventKind::Timer), none());
    assert_eq!(rig.send(1_750, EventKind::Timer), ["unlock", "exit Unlocked"]);
    assert_eq!(rig.deadline(), None);
}

#[test]
fn the_greeting_ends_with_the_parts_gone_and_the_disc_and_clock_left() {
    let mut rig = unlocking_with(Some(true), true);
    let closed = Pose::Moving(dial_layout(size(px(1920.), px(1080.))));
    let parts = |rig: &Rig, ms| match rig.scene(OUT, ms).pose {
        Pose::Moving(layout) => (layout.ring, layout.text),
        Pose::Fading { .. } => panic!("morph"),
    };
    // Held at progress 1 the whole time; parts full until the last 150 ms of the greeting.
    for ms in [1_000, 1_300, 1_500, 1_599] {
        assert_eq!(parts(&rig, ms), (1., 1.), "{ms} ms");
    }
    let (ring, text) = parts(&rig, 1_675);
    assert!((ring - 0.5).abs() < 1e-5 && (text - 0.5).abs() < 1e-5);
    let (ring, text) = parts(&rig, 1_749);
    assert!(ring < 0.001 && text < 0.001, "{ring} {text}");
    assert_eq!(rig.scene(OUT, 1_749).dim, 1.);
    assert!(rig.scene(OUT, 1_749).shell_clock.is_none());
    let mut dial = dial_layout(size(px(1920.), px(1080.)));
    dial.ring = 0.;
    dial.text = 0.;
    let Pose::Moving(last) = rig.scene(OUT, 1_749).pose else {
        panic!("morph")
    };
    assert_eq!((last.opening, last.clock, last.clock_px), (dial.opening, dial.clock, dial.clock_px));
    assert_eq!(rig.scene(OUT, 1_000).pose, closed);
    assert_eq!(rig.send(1_750, EventKind::Timer), ["unlock", "exit Unlocked"]);
}

#[test]
fn the_greeting_text_matches_the_shorter_hold_when_the_shell_opens() {
    let rig = unlocking_with(Some(true), true);
    let alpha = |ms| match rig.panel(ms).line {
        Some(Line::Greeting { alpha, .. }) => alpha,
        other => panic!("expected the greeting at {ms} ms, got {other:?}"),
    };
    assert_eq!(alpha(1_000), 0.);
    assert!((alpha(1_075) - 0.5).abs() < 1e-5);
    for ms in [1_150, 1_400, 1_599] {
        assert_eq!(alpha(ms), 1., "{ms} ms");
    }
    assert!((alpha(1_675) - 0.5).abs() < 1e-5);
}

#[test]
fn without_a_live_shell_overlay_the_locker_plays_its_own_opening_unchanged() {
    for answer in [None, Some(false)] {
        let mut rig = unlocking_with(answer, true);
        rig.send(1_300, EventKind::Timer);
        assert_eq!(rig.deadline(), Some(1_950), "{answer:?}");
        rig.send(1_950, EventKind::Timer);
        assert_eq!(rig.deadline(), Some(2_400), "{answer:?}");
        assert_eq!(rig.send(2_400, EventKind::Timer), ["unlock", "exit Unlocked"]);
    }
}

#[test]
fn a_shell_overlay_with_no_greeting_releases_the_parts_then_unlocks() {
    let mut rig = unlocking_with(Some(true), false);
    assert_eq!(rig.panel(1_000).line, None);
    assert_eq!(rig.deadline(), Some(1_300));
    rig.send(1_300, EventKind::Timer);
    assert_eq!(rig.deadline(), Some(1_450));
    let parts = |rig: &Rig, ms| match rig.scene(OUT, ms).pose {
        Pose::Moving(layout) => layout.ring,
        Pose::Fading { .. } => panic!("morph"),
    };
    assert_eq!(parts(&rig, 1_300), 1.);
    assert!((parts(&rig, 1_375) - 0.5).abs() < 1e-5);
    assert_eq!(rig.send(1_450, EventKind::Timer), ["unlock", "exit Unlocked"]);
    let mut plain = unlocking_with(Some(false), false);
    plain.send(1_300, EventKind::Timer);
    assert_eq!(plain.deadline(), Some(1_450));
    assert_eq!(plain.send(1_450, EventKind::Timer), ["unlock", "exit Unlocked"]);
}

#[test]
fn a_key_skips_the_greeting_into_the_release_when_the_shell_opens() {
    let mut rig = unlocking_with(Some(true), true);
    assert_eq!(rig.key(1_100, Key::Char('q')), none());
    assert_eq!(rig.panel(1_100).line, None);
    assert_eq!(rig.deadline(), Some(1_250));
    assert_eq!(rig.key(1_200, Key::Escape), none());
    assert_eq!(rig.deadline(), Some(1_250));
    assert_eq!(rig.send(1_250, EventKind::Timer), ["unlock", "exit Unlocked"]);
}

#[test]
fn the_shell_overlay_answer_is_ignored_outside_an_unlock() {
    let mut rig = locked();
    let before = rig.deadline();
    assert_eq!(rig.send(500, EventKind::ShellOverlay(true)), none());
    assert_eq!(rig.deadline(), before);
    let mut finished = locked();
    finished.send(600, EventKind::Finished);
    assert_eq!(finished.send(700, EventKind::ShellOverlay(true)), none());
}

#[test]
fn a_reduced_motion_handoff_keeps_the_parts_until_the_unlock() {
    let mut rig = locked_with(Settings {
        reduced_motion: true,
        ..settings()
    });
    rig.send(1_000, fp(2, finished(PamOutcome::Success)));
    rig.send(1_000, EventKind::ShellOverlay(true));
    let Pose::Moving(layout) = rig.scene(OUT, 1_749).pose else {
        panic!("closed")
    };
    assert_eq!((layout.ring, layout.text), (1., 1.));
    assert_eq!(rig.send(1_750, EventKind::Timer), ["unlock", "exit Unlocked"]);
}

// Login mode: the same reducer with a user chip, a session chip and a handoff to the shell.

const SESSIONS: [&str; 3] = ["Niri (UWSM)", "Niri", "GNOME"];

fn login_settings(fingerprint: bool) -> Settings {
    Settings {
        user: "Alyssa Duarte".into(),
        login: Some(
            Login::new(
                "alyssa",
                SESSIONS.iter().map(|s| s.to_string()).collect(),
                "Niri",
            )
            .unwrap()
            .with_fingerprint(fingerprint),
        ),
        ..settings()
    }
}

/// A login screen locked at 0 ms. Without the fingerprint method only password worker 1 runs.
fn login_screen() -> Rig {
    let (mut rig, fx) = Rig::new(login_settings(false));
    assert_eq!(fx, ["close shell surfaces", "sample chips"]);
    assert_eq!(rig.send(0, EventKind::Locked), ["spawn password 1"]);
    rig
}

fn login_screen_with_fingerprint() -> Rig {
    let (mut rig, _) = Rig::new(login_settings(true));
    assert_eq!(
        rig.send(0, EventKind::Locked),
        ["spawn password 1", "spawn fingerprint 2"]
    );
    rig
}

fn login_prompting() -> Rig {
    let mut rig = login_screen();
    assert_eq!(rig.send(10, pw(1, prompt())), none());
    rig
}

fn login_chips(rig: &Rig, ms: u64) -> LoginChips {
    rig.panel(ms).chips.login.expect("login chips")
}

fn editing(rig: &Rig, ms: u64) -> Option<NameEdit> {
    login_chips(rig, ms).editing
}

fn type_name(rig: &mut Rig, ms: u64, name: &str) {
    for c in name.chars() {
        assert_eq!(rig.key(ms, Key::Char(c)), none());
    }
}

fn look(login: &str, display: Option<&str>) -> EventKind {
    EventKind::LookReady(UserLook {
        login: login.into(),
        display: display.map(Into::into),
        farewells: None,
        fingerprint: false,
    })
}

/// `look` with the display manager's fingerprint answer.
fn look_with(login: &str, fingerprint: bool) -> EventKind {
    match look(login, None) {
        EventKind::LookReady(look) => EventKind::LookReady(UserLook {
            fingerprint,
            ..look
        }),
        _ => unreachable!("look builds a LookReady"),
    }
}

fn look_mix(rig: &Rig, ms: u64) -> LookMix {
    rig.scene(OUT, ms).look.expect("login look")
}

#[test]
fn the_login_screen_carries_the_user_its_display_name_and_the_selected_session() {
    let rig = login_screen();
    let panel = rig.panel(0);
    assert_eq!(panel.chips.user, "Alyssa Duarte");
    assert_eq!(panel.chips.lock_age, None);
    assert_eq!(
        panel.chips.login,
        Some(LoginChips {
            session: "Niri".into(),
            editing: None,
            power_open: false,
        })
    );
    assert_eq!(
        rig.lock.choice(),
        Some(Choice {
            login: "alyssa",
            session: "Niri",
            index: 1,
        })
    );
    assert_eq!(
        rig.scene(OUT, 0).look,
        Some(LookMix {
            login: "alyssa".into(),
            from: None,
            mix: 1.
        })
    );
}

#[test]
fn login_settings_reject_a_bad_user_and_an_empty_session_list_and_default_the_selection() {
    let sessions = || SESSIONS.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(
        Login::new("", sessions(), "Niri"),
        Err(LoginError::User(NameError::Empty))
    );
    assert_eq!(
        Login::new("a b", sessions(), "Niri"),
        Err(LoginError::User(NameError::Forbidden))
    );
    assert_eq!(
        Login::new("alyssa", vec![], ""),
        Err(LoginError::NoSessions)
    );
    let (rig, _) = Rig::new(Settings {
        login: Some(Login::new("alyssa", sessions(), "Sway").unwrap()),
        ..settings()
    });
    assert_eq!(rig.lock.choice().unwrap().session, "Niri (UWSM)");
}

#[test]
fn usernames_are_validated_at_the_boundary() {
    assert!(Username::parse("alyssa").is_ok());
    assert!(Username::parse(&"a".repeat(MAX_LOGIN_BYTES)).is_ok());
    assert_eq!(Username::parse(""), Err(NameError::Empty));
    assert_eq!(
        Username::parse(&"a".repeat(MAX_LOGIN_BYTES + 1)),
        Err(NameError::TooLong)
    );
    // Bytes, not characters.
    assert_eq!(
        Username::parse(&"é".repeat(MAX_LOGIN_BYTES / 2 + 1)),
        Err(NameError::TooLong)
    );
    for bad in ["a b", " a", "a\t", "a\nb", "a\0", "a\u{1b}", "a\u{a0}b"] {
        assert_eq!(Username::parse(bad), Err(NameError::Forbidden), "{bad:?}");
    }
}

#[test]
fn tab_edits_the_name_and_enter_commits_it_with_a_new_password_worker() {
    let mut rig = login_prompting();
    assert_eq!(rig.key(100, Key::Tab), none());
    assert_eq!(
        editing(&rig, 100),
        Some(NameEdit {
            text: String::new(),
            refused: false
        })
    );
    type_name(&mut rig, 200, "bob");
    assert_eq!(editing(&rig, 200).unwrap().text, "bob");
    assert_eq!(rig.panel(200).chips.user, "Alyssa Duarte");
    assert_eq!(
        rig.key(300, Key::Enter),
        ["swap look bob", "kill password 1", "spawn password 2"]
    );
    assert_eq!(editing(&rig, 300), None);
    assert_eq!(rig.panel(300).chips.user, "bob");
    assert_eq!(rig.lock.choice().unwrap().login, "bob");
    // The new worker is a new generation: the old one's reports are stale.
    assert_eq!(rig.send(400, pw(1, finished(PamOutcome::Success))), none());
    assert_eq!(rig.send(410, pw(2, prompt())), none());
    rig.type_text(500, "secret");
    assert_eq!(rig.key(600, Key::Enter), ["answer password 2 with 6 chars"]);
}

#[test]
fn a_click_on_the_user_chip_edits_the_name_like_tab() {
    let mut rig = login_prompting();
    assert_eq!(rig.send(100, EventKind::UserChip(OUT)), none());
    assert!(editing(&rig, 100).is_some());
    type_name(&mut rig, 150, "bob");
    // A second click while editing changes nothing.
    assert_eq!(rig.send(160, EventKind::UserChip(OUT)), none());
    assert_eq!(editing(&rig, 160).unwrap().text, "bob");
    assert_eq!(
        rig.key(200, Key::Enter),
        ["swap look bob", "kill password 1", "spawn password 2"]
    );
}

#[test]
fn escape_cancels_the_edit_and_leaves_the_user_workers_and_look_alone() {
    let mut rig = login_prompting();
    rig.key(100, Key::Tab);
    type_name(&mut rig, 150, "bob");
    assert_eq!(rig.key(200, Key::Escape), none());
    assert_eq!(editing(&rig, 200), None);
    assert_eq!(rig.panel(200).chips.user, "Alyssa Duarte");
    assert_eq!(rig.lock.choice().unwrap().login, "alyssa");
    assert_eq!(look_mix(&rig, 200).login, "alyssa");
    // The worker still answers for the old user.
    rig.type_text(300, "pw");
    assert_eq!(rig.key(400, Key::Enter), ["answer password 1 with 2 chars"]);
}

#[test]
fn committing_the_same_name_changes_nothing() {
    let mut rig = login_prompting();
    rig.key(100, Key::Tab);
    type_name(&mut rig, 150, "alyssa");
    assert_eq!(rig.key(200, Key::Enter), none());
    assert_eq!(editing(&rig, 200), None);
    assert_eq!(look_mix(&rig, 200).from, None);
}

#[test]
fn the_name_editor_refuses_empty_long_whitespace_and_control_input() {
    let mut rig = login_prompting();
    rig.key(100, Key::Tab);
    assert_eq!(rig.key(110, Key::Enter), none());
    assert_eq!(
        editing(&rig, 110),
        Some(NameEdit {
            text: String::new(),
            refused: true
        })
    );
    for c in [' ', '\t', '\n', '\u{7}', '\u{a0}'] {
        assert_eq!(rig.key(120, Key::Char(c)), none());
        assert_eq!(
            editing(&rig, 120),
            Some(NameEdit {
                text: String::new(),
                refused: true
            })
        );
    }
    type_name(&mut rig, 130, "a");
    assert!(!editing(&rig, 130).unwrap().refused);
    type_name(&mut rig, 140, &"b".repeat(MAX_LOGIN_BYTES - 1));
    assert_eq!(editing(&rig, 140).unwrap().text.len(), MAX_LOGIN_BYTES);
    assert!(!editing(&rig, 140).unwrap().refused);
    assert_eq!(rig.key(150, Key::Char('c')), none());
    let edit = editing(&rig, 150).unwrap();
    assert_eq!((edit.text.len(), edit.refused), (MAX_LOGIN_BYTES, true));
    // A multi-byte character that would cross the limit is refused whole.
    rig.key(160, Key::Backspace);
    assert!(!editing(&rig, 160).unwrap().refused);
    assert_eq!(rig.key(170, Key::Char('é')), none());
    assert_eq!(editing(&rig, 170).unwrap().text.len(), MAX_LOGIN_BYTES - 1);
    assert!(editing(&rig, 170).unwrap().refused);
    // Nothing was committed and no worker was touched; ClearLine empties the editor.
    assert_eq!(rig.key(180, Key::ClearLine), none());
    assert_eq!(editing(&rig, 180).unwrap().text, "");
    assert_eq!(rig.lock.choice().unwrap().login, "alyssa");
}

#[test]
fn committing_drops_the_password_typed_for_the_old_user() {
    let mut rig = login_prompting();
    rig.type_text(100, "hunter2");
    assert_eq!(rig.pill(100).unwrap().dots, 7);
    rig.key(200, Key::Tab);
    type_name(&mut rig, 250, "bob");
    assert_eq!(
        rig.key(300, Key::Enter),
        ["swap look bob", "kill password 1", "spawn password 2"]
    );
    assert_eq!(rig.pill(300), None);
    // Enter before the new prompt submits nothing.
    assert_eq!(rig.key(310, Key::Enter), none());
    assert_eq!(rig.send(320, pw(2, prompt())), none());
}

#[test]
fn a_commit_before_the_lock_lands_swaps_the_look_and_starts_the_new_users_worker_on_lock() {
    let (mut rig, _) = Rig::new(login_settings(false));
    rig.type_text(10, "old");
    rig.key(20, Key::Tab);
    type_name(&mut rig, 30, "bob");
    assert_eq!(rig.key(40, Key::Enter), ["swap look bob"]);
    assert_eq!(rig.send(50, EventKind::Locked), ["spawn password 1"]);
    assert_eq!(rig.pill(60), None);
}

#[test]
fn keys_outside_editing_reach_the_password_pill_and_keys_while_editing_do_not() {
    let mut rig = login_prompting();
    rig.type_text(100, "ab");
    assert_eq!(rig.pill(100).unwrap().dots, 2);
    rig.key(150, Key::Tab);
    type_name(&mut rig, 200, "xyz");
    assert_eq!(rig.pill(200).unwrap().dots, 2);
    assert_eq!(editing(&rig, 200).unwrap().text, "xyz");
    rig.key(250, Key::Escape);
    rig.key(260, Key::Char('c'));
    assert_eq!(rig.pill(300).unwrap().dots, 3);
    assert_eq!(rig.key(400, Key::Enter), ["answer password 1 with 3 chars"]);
}

#[test]
fn the_first_key_on_a_fresh_login_screen_grows_the_pill_as_on_the_lock_screen() {
    let mut rig = login_prompting();
    assert_eq!(rig.pill(0), None);
    assert_eq!(rig.panel(0).dial.scale, 1.);
    rig.type_text(100, "p");
    assert_eq!(rig.pill(100).unwrap().dots, 1);
    assert!(rig.scene(OUT, 150).animating);
    assert_eq!(rig.panel(400).dial.scale, 0.72);
}

#[test]
fn the_session_key_and_chip_cycle_the_selection_and_restart_the_conversation_with_the_typing_kept() {
    let mut rig = login_prompting();
    rig.type_text(50, "pw");
    // A conversation begun for a session would start that one, so it restarts for the new one.
    assert_eq!(
        rig.key(100, Key::NextSession),
        ["kill password 1", "spawn password 2"]
    );
    assert_eq!(login_chips(&rig, 100).session, "GNOME");
    assert_eq!(rig.lock.choice().unwrap().index, 2);
    assert_eq!(
        rig.send(110, EventKind::SessionChip(OUT)),
        ["kill password 2", "spawn password 3"]
    );
    assert_eq!(login_chips(&rig, 110).session, "Niri (UWSM)");
    assert_eq!(rig.lock.choice().unwrap().index, 0);
    // The typed password, the user and the look are untouched, and a stale prompt is ignored.
    assert_eq!(rig.pill(120).unwrap().dots, 2);
    assert_eq!(rig.lock.choice().unwrap().login, "alyssa");
    assert_eq!(look_mix(&rig, 120).from, None);
    assert_eq!(rig.send(130, pw(1, prompt())), none());
    assert_eq!(rig.send(140, pw(3, prompt())), none());
    assert_eq!(rig.key(200, Key::Enter), ["answer password 3 with 2 chars"]);
}

#[test]
fn a_session_change_restarts_a_live_sensor_and_leaves_a_released_one_alone() {
    let mut rig = login_screen_with_fingerprint();
    rig.send(10, pw(1, prompt()));
    assert_eq!(
        rig.key(100, Key::NextSession),
        [
            "kill password 1",
            "spawn password 3",
            "kill fingerprint 2",
            "spawn fingerprint 4"
        ]
    );
    rig.send(110, fp(4, finished(PamOutcome::Unavailable)));
    rig.send(120, fp(4, finished(PamOutcome::Unavailable)));
    assert_eq!(rig.key(200, Key::NextSession), ["kill password 3", "spawn password 5"]);
}

#[test]
fn a_session_change_after_the_verdict_is_refused() {
    let mut rig = login_prompting();
    rig.type_text(100, "pw");
    rig.key(150, Key::Enter);
    assert_eq!(rig.key(200, Key::NextSession), none());
    assert_eq!(rig.send(210, EventKind::SessionChip(OUT)), none());
    assert_eq!(rig.lock.choice().unwrap().session, "Niri");
}

#[test]
fn the_session_key_is_not_a_password_character_while_editing_the_name() {
    let mut rig = login_prompting();
    rig.key(100, Key::Tab);
    assert_eq!(rig.key(110, Key::NextSession), none());
    assert_eq!(login_chips(&rig, 110).session, "Niri");
    assert_eq!(editing(&rig, 110).unwrap().text, "");
}

#[test]
fn the_lock_screen_ignores_login_keys_and_chip_clicks_and_has_no_login_scene() {
    let mut rig = prompting();
    let before = rig.scene(OUT, 100);
    assert_eq!(before.look, None);
    assert_eq!(rig.panel(100).chips.login, None);
    assert_eq!(rig.lock.choice(), None);
    assert_eq!(rig.key(100, Key::Tab), none());
    assert_eq!(rig.key(100, Key::NextSession), none());
    assert_eq!(rig.send(100, EventKind::UserChip(OUT)), none());
    assert_eq!(rig.send(100, EventKind::SessionChip(OUT)), none());
    assert_eq!(rig.send(100, look("bob", Some("Bob"))), none());
    assert_eq!(rig.scene(OUT, 100), before);
    assert_eq!(rig.panel(100).chips.user, "alyssa");
}

#[test]
fn the_hint_asks_for_the_password_when_it_is_the_only_method() {
    let mut rig = login_prompting();
    assert_eq!(rig.hint(0), "Type your password");
    rig.type_text(100, "p");
    assert_eq!(rig.hint(100), "Type your password");
    rig.key(150, Key::Enter);
    rig.send(2_000, pw(1, finished(PamOutcome::Rejected)));
    assert_eq!(rig.hint(2_100), "That wasn't it. Try again.");
}

#[test]
fn without_the_fingerprint_method_no_sensor_is_spawned_armed_or_shown() {
    let mut rig = login_prompting();
    assert_eq!(rig.panel(0).dial.ring.tone, Tone::Normal);
    // Display off and the next activity touch no sensor.
    assert_eq!(rig.send(36_000, EventKind::Timer), ["display off"]);
    assert_eq!(rig.key(40_000, Key::Char('a')), none());
    // A stale fingerprint report is ignored: there is no worker.
    assert_eq!(
        rig.send(40_100, fp(2, hint("Place your finger", true))),
        none()
    );
    assert_eq!(rig.hint(40_100), "Type your password");
}

#[test]
fn with_the_fingerprint_method_both_workers_race_as_on_the_lock_screen() {
    let mut rig = login_screen_with_fingerprint();
    rig.send(10, pw(1, prompt()));
    assert_eq!(rig.hint(10), "Touch the sensor, or just start typing");
    assert_eq!(rig.panel(10).dial.ring.tone, Tone::Normal);
    assert_eq!(
        rig.send(500, fp(2, finished(PamOutcome::Success))),
        ["kill password 1"]
    );
}

#[test]
fn a_fingerprint_that_is_unavailable_for_the_user_leaves_the_password_hint() {
    let mut rig = login_screen_with_fingerprint();
    rig.send(10, pw(1, prompt()));
    rig.send(20, fp(2, finished(PamOutcome::Unavailable)));
    assert_eq!(rig.hint(20), "Type your password");
    assert_eq!(rig.panel(20).dial.ring.tone, Tone::Muted);
}

#[test]
fn committing_a_user_drops_the_old_users_sensor_until_the_new_look_says_otherwise() {
    let mut rig = login_screen_with_fingerprint();
    rig.send(10, pw(1, prompt()));
    rig.key(100, Key::Tab);
    type_name(&mut rig, 110, "bob");
    assert_eq!(
        rig.key(200, Key::Enter),
        ["swap look bob", "kill password 1", "spawn password 3", "kill fingerprint 2"]
    );
    // The old user's finger no longer counts.
    assert_eq!(rig.send(210, fp(2, finished(PamOutcome::Success))), none());
    assert_eq!(rig.panel(210).dial.ring.tone, Tone::Normal);
    assert_eq!(rig.send(220, look_with("bob", true)), ["spawn fingerprint 4"]);
    assert_eq!(
        rig.send(300, fp(4, finished(PamOutcome::Success))),
        ["kill password 3"]
    );
}

#[test]
fn editing_the_user_is_refused_while_an_answer_is_being_checked() {
    let mut rig = login_prompting();
    rig.type_text(100, "pw");
    rig.key(150, Key::Enter);
    assert_eq!(rig.key(200, Key::Tab), none());
    assert_eq!(rig.send(210, EventKind::UserChip(OUT)), none());
    assert_eq!(editing(&rig, 210), None);
    assert_eq!(rig.key(220, Key::Char('x')), none());
    assert_eq!(rig.pill(220).unwrap().dots, 2);
}

#[test]
fn a_password_pause_belongs_to_the_old_user_and_ends_when_the_user_changes() {
    let mut rig = login_prompting();
    rig.send(
        20,
        pw(
            1,
            Report::Notice(Notice::PasswordPaused {
                remaining: Duration::from_secs(600),
            }),
        ),
    );
    assert!(rig.pill(30).unwrap().dashed);
    rig.key(100, Key::Tab);
    type_name(&mut rig, 110, "bob");
    assert_eq!(
        rig.key(200, Key::Enter),
        ["swap look bob", "spawn password 2"]
    );
    assert!(rig.pill(210).is_none());
}

#[test]
fn a_login_success_fills_the_ring_then_dips_to_the_frame_colour_and_asks_for_the_session() {
    let mut rig = login_prompting();
    rig.type_text(100, "hunter2");
    rig.key(200, Key::Enter);
    // No fingerprint worker to kill, no overlay to ask about and no greeting to play.
    assert_eq!(rig.send(1_000, pw(1, finished(PamOutcome::Success))), none());
    assert_eq!(rig.panel(1_000).line, None);
    // The ring's motion ends first, then the fade.
    assert_eq!(rig.deadline(), Some(1_300));
    assert!(rig.panel(1_150).dial.ring.fill > 0.);
    // The ring fills for 300 ms with the scene whole, then the whole scene fades over 200.
    for ms in [1_000, 1_150, 1_300] {
        assert_eq!(rig.scene(OUT, ms).veil, 0., "{ms} ms");
    }
    assert!((rig.scene(OUT, 1_400).veil - 0.5).abs() < 1e-5);
    assert!(rig.scene(OUT, 1_400).animating);
    assert_eq!(rig.send(1_300, EventKind::Timer), none());
    assert_eq!(rig.deadline(), Some(1_500));
    assert_eq!(rig.send(1_499, EventKind::Timer), none());
    assert_eq!(rig.send(1_500, EventKind::Timer), ["unlock"]);
    // The frame colour stays up, still, while the display manager starts the session.
    let held = rig.scene(OUT, 1_500);
    assert_eq!(held.veil, 1.);
    assert!(!held.animating);
    assert_eq!(held.pose, Pose::Moving(handed_over()));
    assert_eq!(rig.deadline(), None);
}

#[test]
fn every_output_dips_not_only_the_active_one() {
    let mut rig = login_prompting();
    rig.send(1_000, pw(1, finished(PamOutcome::Success)));
    let side = |ms| rig.scene(SIDE, ms);
    assert!(matches!(side(1_400).role, Role::Passive));
    assert!((side(1_400).veil - 0.5).abs() < 1e-5);
    assert!(side(1_400).animating, "a passive output must keep asking for frames while it fades");
    assert_eq!(side(1_500).veil, 1.);
    assert!(!side(1_500).animating);
}

#[test]
fn the_login_screen_plays_no_greeting_even_when_one_is_set() {
    let mut rig = login_prompting();
    rig.send(1_000, pw(1, finished(PamOutcome::Success)));
    for ms in [1_000, 1_100, 1_300, 1_450, 1_500] {
        assert_eq!(rig.panel(ms).line, None, "{ms} ms");
    }
}

#[test]
fn a_key_during_the_fade_changes_nothing_and_never_cancels_the_login() {
    let mut rig = login_prompting();
    rig.send(1_000, pw(1, finished(PamOutcome::Success)));
    assert_eq!(rig.key(1_100, Key::Char('q')), none());
    assert_eq!(rig.panel(1_100).line, None);
    assert_eq!(rig.key(1_120, Key::Tab), none());
    assert_eq!(rig.key(1_130, Key::NextSession), none());
    assert_eq!(rig.send(1_140, EventKind::UserChip(OUT)), none());
    assert_eq!(editing(&rig, 1_140), None);
    assert_eq!(rig.lock.choice().unwrap().session, "Niri");
    assert_eq!(rig.send(1_150, EventKind::Pointer(OUT)), none());
    assert_eq!(rig.deadline(), Some(1_300));
    assert_eq!(rig.send(1_300, EventKind::Timer), none());
    assert_eq!(rig.deadline(), Some(1_500));
    assert_eq!(rig.send(1_500, EventKind::Timer), ["unlock"]);
}

#[test]
fn under_reduced_motion_the_ring_is_shown_full_then_the_screen_cuts_to_the_frame_colour() {
    let (mut rig, _) = Rig::new(Settings {
        reduced_motion: true,
        ..login_settings(false)
    });
    rig.send(0, EventKind::Locked);
    rig.send(10, pw(1, prompt()));
    rig.send(1_000, pw(1, finished(PamOutcome::Success)));
    assert_eq!(rig.panel(1_000).dial.ring.fill, 1.);
    assert_eq!(rig.deadline(), Some(1_300));
    assert_eq!(rig.scene(OUT, 1_299).veil, 0.);
    assert_eq!(rig.send(1_300, EventKind::Timer), ["unlock"]);
    assert_eq!(rig.scene(OUT, 1_300).veil, 1.);
}

#[test]
fn a_fingerprint_login_dips_to_the_frame_colour_too() {
    let mut rig = login_screen_with_fingerprint();
    assert_eq!(
        rig.send(1_000, fp(2, finished(PamOutcome::Success))),
        ["kill password 1"]
    );
    assert_eq!(rig.deadline(), Some(1_300));
    rig.send(1_300, EventKind::Timer);
    assert_eq!(rig.deadline(), Some(1_500));
    assert_eq!(rig.send(1_500, EventKind::Timer), ["unlock"]);
}

#[test]
fn an_editor_open_when_the_verdict_arrives_is_dropped() {
    let mut rig = login_prompting();
    rig.type_text(100, "pw");
    rig.key(150, Key::Enter);
    rig.send(160, pw(1, prompt()));
    rig.key(200, Key::Tab);
    assert_eq!(editing(&rig, 200).map(|e| e.text), Some(String::new()));
    rig.send(300, pw(1, finished(PamOutcome::Success)));
    assert_eq!(editing(&rig, 310), None);
}

#[test]
fn a_look_swap_waits_for_the_edge_then_cross_fades_over_the_reveal() {
    let mut rig = login_prompting();
    rig.key(100, Key::Tab);
    type_name(&mut rig, 110, "bob");
    rig.key(200, Key::Enter);
    // Until the edge has the look, the old one stays and nothing animates.
    assert_eq!(
        look_mix(&rig, 300),
        LookMix {
            login: "alyssa".into(),
            from: None,
            mix: 1.
        }
    );
    assert!(!rig.scene(OUT, 300).animating);
    assert_eq!(rig.send(1_000, look("bob", Some("Bob Builder"))), none());
    assert_eq!(rig.panel(1_000).chips.user, "Bob Builder");
    let start = look_mix(&rig, 1_000);
    assert_eq!(
        (start.login.as_str(), start.from.as_deref(), start.mix),
        ("bob", Some("alyssa"), 0.)
    );
    assert!(rig.scene(OUT, 1_000).animating);
    assert!(rig.scene(SIDE, 1_000).animating);
    let middle = look_mix(&rig, 1_125);
    assert!((middle.mix - 0.5).abs() < 1e-5);
    assert!(rig.scene(OUT, 1_125).animating);
    assert_eq!(rig.deadline(), Some(1_250));
    let done = look_mix(&rig, 1_250);
    assert_eq!(
        done,
        LookMix {
            login: "bob".into(),
            from: None,
            mix: 1.
        }
    );
    assert!(!rig.scene(OUT, 1_250).animating);
    assert_eq!(rig.send(1_250, EventKind::Timer), none());
    // Only the power timeline is left: settling 5 s after the last key.
    assert_eq!(rig.deadline(), Some(5_200));
}

#[test]
fn a_settled_login_screen_draws_no_frame_for_the_passing_time() {
    let mut rig = login_prompting();
    rig.key(100, Key::Tab);
    type_name(&mut rig, 110, "bob");
    rig.key(200, Key::Enter);
    rig.send(300, look("bob", None));
    rig.send(550, EventKind::Timer);
    assert_eq!(rig.scene(OUT, 600), rig.scene(OUT, 2_000));
    assert_eq!(rig.scene(SIDE, 600), rig.scene(SIDE, 2_000));
}

#[test]
fn reduced_motion_cuts_the_look_swap() {
    let (mut rig, _) = Rig::new(Settings {
        reduced_motion: true,
        ..login_settings(false)
    });
    rig.send(0, EventKind::Locked);
    rig.send(10, pw(1, prompt()));
    rig.key(100, Key::Tab);
    type_name(&mut rig, 110, "bob");
    rig.key(200, Key::Enter);
    assert_eq!(look_mix(&rig, 250).login, "alyssa");
    rig.send(300, look("bob", Some("Bob")));
    assert_eq!(
        look_mix(&rig, 300),
        LookMix {
            login: "bob".into(),
            from: None,
            mix: 1.
        }
    );
    assert!(!rig.scene(OUT, 300).animating);
    // No fade deadline: only the power timeline, 5 s after the last key.
    assert_eq!(rig.deadline(), Some(5_200));
}

#[test]
fn a_look_for_another_user_is_stale_and_ignored() {
    let mut rig = login_prompting();
    rig.key(100, Key::Tab);
    type_name(&mut rig, 110, "bob");
    rig.key(200, Key::Enter);
    assert_eq!(rig.send(300, look("alyssa", Some("Alyssa"))), none());
    assert_eq!(rig.panel(300).chips.user, "bob");
    assert_eq!(look_mix(&rig, 300).login, "alyssa");
    assert!(!rig.scene(OUT, 300).animating);
}

#[test]
fn a_second_commit_before_the_first_look_arrives_fades_from_what_was_on_screen() {
    let mut rig = login_prompting();
    rig.key(100, Key::Tab);
    type_name(&mut rig, 110, "bob");
    rig.key(200, Key::Enter);
    rig.key(300, Key::Tab);
    type_name(&mut rig, 310, "carol");
    assert_eq!(
        rig.key(400, Key::Enter),
        ["swap look carol", "kill password 2", "spawn password 3"]
    );
    rig.send(450, look("bob", None));
    assert_eq!(look_mix(&rig, 450).login, "alyssa");
    rig.send(500, look("carol", None));
    let start = look_mix(&rig, 500);
    assert_eq!(
        (start.login.as_str(), start.from.as_deref()),
        ("carol", Some("alyssa"))
    );
}

#[test]
fn a_user_with_no_look_keeps_the_typed_name_and_the_same_hint_and_skips_the_greeting() {
    let mut rig = login_prompting();
    let hint_before = rig.hint(0);
    rig.key(100, Key::Tab);
    type_name(&mut rig, 110, "ghost");
    rig.key(200, Key::Enter);
    rig.send(
        300,
        EventKind::LookReady(UserLook {
            login: "ghost".into(),
            display: None,
            farewells: None,
            fingerprint: false,
        }),
    );
    assert_eq!(rig.panel(300).chips.user, "ghost");
    assert_eq!(rig.hint(300), hint_before);
    rig.send(310, pw(2, prompt()));
    rig.type_text(400, "pw");
    rig.key(500, Key::Enter);
    rig.send(1_000, pw(2, finished(PamOutcome::Success)));
    assert_eq!(rig.panel(1_000).line, None);
}

fn unlocked_by(fx: &[Effect]) -> Option<WorkerId> {
    fx.iter().find_map(|effect| match effect {
        Effect::Unlock { by } => Some(*by),
        _ => None,
    })
}

fn raw(rig: &mut Rig, ms: u64, kind: EventKind) -> Vec<Effect> {
    let now = rig.now(ms);
    rig.lock.update(Event { now, kind })
}

fn farewells() -> FarewellSet {
    FarewellSet {
        restart: "Back soon, Alyssa".into(),
        shutdown: "Goodnight, Alyssa".into(),
    }
}

fn look_full(login: &str, fingerprint: bool, farewells: Option<FarewellSet>) -> EventKind {
    match look(login, None) {
        EventKind::LookReady(look) => EventKind::LookReady(UserLook {
            fingerprint,
            farewells,
            ..look
        }),
        _ => unreachable!("look builds a LookReady"),
    }
}

/// A login screen with the sensor armed, farewells known and the password prompt waiting.
fn leaving_screen() -> Rig {
    let mut rig = login_screen_with_fingerprint();
    assert_eq!(rig.send(5, look_full("alyssa", true, Some(farewells()))), none());
    rig.send(10, pw(1, prompt()));
    rig
}

fn farewell_line(rig: &Rig, ms: u64) -> Option<(String, f32)> {
    match rig.panel(ms).line {
        Some(Line::Farewell { text, alpha }) => Some((text, alpha)),
        _ => None,
    }
}

#[test]
fn a_users_fingerprint_answer_arms_the_sensor_once_the_screen_is_locked() {
    let mut rig = login_screen();
    assert_eq!(rig.panel(0).dial.ring.tone, Tone::Normal);
    // The display manager says the first user has a print on a reader that exists.
    assert_eq!(rig.send(20, look_with("alyssa", true)), ["spawn fingerprint 2"]);
    assert_eq!(rig.hint(20), "Touch the sensor, or just start typing");
    assert_eq!(
        rig.send(30, fp(2, finished(PamOutcome::Success))),
        ["kill password 1"]
    );
}

#[test]
fn an_answer_that_arrives_before_the_lock_decides_which_workers_the_lock_starts() {
    let (mut rig, _) = Rig::new(login_settings(false));
    assert_eq!(rig.send(5, look_with("alyssa", true)), none());
    assert_eq!(
        rig.send(10, EventKind::Locked),
        ["spawn password 1", "spawn fingerprint 2"]
    );
    let (mut rig, _) = Rig::new(login_settings(true));
    assert_eq!(rig.send(5, look_with("alyssa", false)), none());
    assert_eq!(rig.send(10, EventKind::Locked), ["spawn password 1"]);
}

#[test]
fn a_user_without_a_print_shows_no_sensor_and_runs_no_fingerprint_worker() {
    let mut rig = login_screen_with_fingerprint();
    rig.send(10, pw(1, prompt()));
    rig.key(100, Key::Tab);
    type_name(&mut rig, 110, "bob");
    rig.key(200, Key::Enter);
    assert_eq!(rig.send(300, look_with("bob", false)), none());
    assert_eq!(rig.hint(300), "Type your password");
    assert_eq!(rig.panel(300).dial.ring.tone, Tone::Normal);
    // Activity never arms a sensor the user does not have.
    assert_eq!(rig.send(400, EventKind::Pointer(OUT)), none());
}

#[test]
fn a_look_that_withdraws_the_print_releases_a_live_sensor_and_a_released_one_stays_released() {
    let mut rig = login_screen_with_fingerprint();
    rig.send(10, pw(1, prompt()));
    assert_eq!(rig.send(20, look_with("alyssa", false)), ["kill fingerprint 2"]);
    assert_eq!(rig.hint(20), "Type your password");
    // Nothing re-arms it, not even the display coming back on.
    assert_eq!(rig.send(35_000, EventKind::Timer), ["display off"]);
    assert_eq!(rig.send(40_000, EventKind::Pointer(OUT)), none());
}

#[test]
fn a_fingerprint_answer_for_another_user_is_stale() {
    let mut rig = login_screen();
    assert_eq!(rig.send(20, look_with("bob", true)), none());
    assert_eq!(rig.send(30, look_with("alyssa", false)), none());
    assert_eq!(rig.panel(30).dial.ring.tone, Tone::Normal);
}

#[test]
fn the_lock_screen_ignores_a_look_it_never_asked_for() {
    let mut rig = locked();
    assert_eq!(rig.send(20, look_with("alyssa", false)), none());
    assert_eq!(rig.panel(20).dial.ring.tone, Tone::Normal);
    assert_eq!(
        rig.send(30, fp(2, finished(PamOutcome::Success))),
        ["kill password 1", "probe shell overlay"]
    );
}

#[test]
fn the_login_unlock_names_the_conversation_that_won() {
    let mut rig = login_screen_with_fingerprint();
    rig.send(10, pw(1, prompt()));
    let fx = raw(&mut rig, 20, fp(2, finished(PamOutcome::Success)));
    assert_eq!(unlocked_by(&fx), None);
    assert_eq!(show(&fx), ["kill password 1"]);
    let fx = raw(&mut rig, 520, EventKind::Timer);
    assert_eq!(
        unlocked_by(&fx),
        Some(WorkerId {
            method: Method::Fingerprint,
            generation: 2
        })
    );
}

/// Authenticated at 1 000 ms, faded to the frame colour and handed over at 1 500 ms.
fn handed_over_screen(fingerprint: bool) -> Rig {
    let mut rig = if fingerprint {
        login_screen_with_fingerprint()
    } else {
        login_screen()
    };
    rig.send(
        5,
        EventKind::LookReady(UserLook {
            login: "alyssa".into(),
            display: None,
            farewells: None,
            fingerprint,
        }),
    );
    rig.send(10, pw(1, prompt()));
    rig.send(1_000, pw(1, finished(PamOutcome::Success)));
    assert_eq!(rig.send(1_500, EventKind::Timer), ["unlock"]);
    rig
}

#[test]
fn the_handover_holds_one_still_closed_frame_until_the_display_manager_answers() {
    let mut rig = handed_over_screen(false);
    assert_eq!(rig.deadline(), None);
    for ms in [1_500, 2_000, 60_000] {
        let scene = rig.scene(OUT, ms);
        assert!(!scene.animating, "{ms} ms");
        assert_eq!(scene.veil, 1., "{ms} ms");
        assert_eq!(scene.pose, Pose::Moving(handed_over()), "{ms} ms");
        assert_eq!(scene.dim, 1.);
    }
    assert_eq!(rig.send(60_000, EventKind::Timer), none());
    assert_eq!(rig.key(61_000, Key::Char('q')), none());
    assert_eq!(rig.send(61_000, EventKind::UserChip(OUT)), none());
    assert_eq!(rig.send(61_000, EventKind::PowerChip(OUT)), none());
    let before = rig.scene(OUT, 62_000);
    assert_eq!(rig.send(62_000, EventKind::SessionStarted), ["exit Unlocked"]);
    // The last frame is the closed state, exactly as it was.
    assert_eq!(rig.scene(OUT, 62_000), before);
    assert_eq!(rig.send(62_100, EventKind::SessionStarted), none());
    assert_eq!(rig.send(62_100, EventKind::SessionFailed("late".into())), none());
}

#[test]
fn a_session_that_does_not_start_asks_for_the_password_again_with_the_reason() {
    let mut rig = handed_over_screen(false);
    assert_eq!(
        rig.send(3_000, EventKind::SessionFailed("The session could not start".into())),
        ["spawn password 2"]
    );
    assert_eq!(rig.hint(3_000), "The session could not start");
    // The screen is not veiled any more, and the parts fade in over the dial.
    assert_eq!(rig.scene(OUT, 3_000).veil, 0.);
    let Pose::Moving(layout) = rig.scene(OUT, 3_000).pose else {
        panic!("morph")
    };
    assert_eq!((layout.ring, layout.text), (0., 0.));
    let Pose::Moving(layout) = rig.scene(OUT, 3_250).pose else {
        panic!("morph")
    };
    assert_eq!((layout.ring, layout.text), (1., 1.));
    assert_eq!(rig.panel(3_250).chips.user, "alyssa");
    assert_eq!(rig.pill(3_250), None);
    // The next key clears the reason and goes to the new conversation.
    rig.send(3_300, pw(2, prompt()));
    assert_eq!(rig.key(3_400, Key::Char('x')), none());
    assert_eq!(rig.hint(3_400), "Type your password");
    assert_eq!(rig.pill(3_400).unwrap().dots, 1);
    assert_eq!(rig.key(3_500, Key::Enter), ["answer password 2 with 1 chars"]);
    rig.send(3_600, pw(2, finished(PamOutcome::Success)));
    assert_eq!(rig.send(4_100, EventKind::Timer), ["unlock"]);
    assert_eq!(rig.send(4_200, EventKind::SessionStarted), ["exit Unlocked"]);
}

#[test]
fn a_helper_that_dies_after_the_verdict_asks_for_the_password_again_at_once() {
    let mut rig = login_prompting();
    rig.send(1_000, pw(1, finished(PamOutcome::Success)));
    assert_eq!(rig.scene(OUT, 1_100).veil, 0.);
    // The ring is still filling when the display manager says the start cannot happen.
    assert_eq!(
        rig.send(1_100, EventKind::SessionFailed("The login stopped unexpectedly".into())),
        ["spawn password 2"]
    );
    assert_eq!(rig.hint(1_100), "The login stopped unexpectedly");
    // Nothing of the abandoned fade is left to unlock.
    assert_eq!(rig.send(1_300, EventKind::Timer), none());
    assert_eq!(rig.send(2_000, EventKind::Timer), none());
    assert_eq!(rig.send(2_100, EventKind::SessionStarted), none());
}

#[test]
fn a_failed_start_rearms_the_sensor_too_and_a_stale_answer_changes_nothing() {
    let mut rig = handed_over_screen(true);
    assert_eq!(
        rig.send(3_000, EventKind::SessionFailed("Refused".into())),
        ["spawn password 3", "spawn fingerprint 4"]
    );
    // The old conversations are over: their reports are stale.
    assert_eq!(rig.send(3_010, pw(1, finished(PamOutcome::Success))), none());
    assert_eq!(rig.send(3_020, fp(2, finished(PamOutcome::Success))), none());
    // Neither answer means anything while the screen asks for the password.
    assert_eq!(rig.send(3_030, EventKind::SessionFailed("again".into())), none());
    assert_eq!(rig.send(3_040, EventKind::SessionStarted), none());
    assert_eq!(rig.hint(3_040), "Refused");
}

#[test]
fn a_reason_told_before_the_lock_lands_is_the_hint_when_it_does() {
    let (mut rig, _) = Rig::new(login_settings(false));
    assert_eq!(
        rig.send(0, EventKind::SessionFailed("The session could not be started".into())),
        none()
    );
    assert_eq!(rig.send(5, EventKind::Locked), ["spawn password 1"]);
    assert_eq!(rig.hint(10), "The session could not be started");
    rig.send(20, pw(1, prompt()));
    assert_eq!(rig.key(100, Key::Char('x')), none());
    assert_eq!(rig.hint(110), "Type your password");
}

#[test]
fn a_reason_told_after_the_lock_is_the_hint_until_a_key_unless_another_stands() {
    let mut rig = login_prompting();
    assert_eq!(
        rig.send(50, EventKind::SessionFailed("That login was refused".into())),
        none()
    );
    assert_eq!(rig.hint(60), "That login was refused");
    assert_eq!(rig.send(70, EventKind::SessionFailed("again".into())), none());
    assert_eq!(rig.hint(80), "That login was refused");
    assert_eq!(rig.key(100, Key::Char('x')), none());
    assert_eq!(rig.hint(110), "Type your password");
    // Told once the lock has landed, the reason does not come back by itself.
    assert_eq!(rig.hint(5_000), "Type your password");
}

#[test]
fn the_lock_screen_has_no_handover_and_no_power_chip() {
    let mut rig = locked();
    assert_eq!(rig.send(50, EventKind::SessionStarted), none());
    assert_eq!(rig.send(60, EventKind::SessionFailed("x".into())), none());
    assert_eq!(rig.send(70, EventKind::PowerChip(OUT)), none());
    assert_eq!(rig.send(80, EventKind::LeaveChip(OUT, Leave::Shutdown)), none());
    assert_eq!(rig.key(90, Key::Leave(Leave::Shutdown)), none());
    assert_eq!(rig.pill(90), None);
    assert_eq!(rig.panel(90).chips.login, None);
}

#[test]
fn a_shutdown_fades_the_parts_plays_the_farewell_then_hands_the_occasion_over() {
    let mut rig = leaving_screen();
    assert_eq!(
        rig.key(100, Key::Leave(Leave::Shutdown)),
        ["kill password 1", "kill fingerprint 2"]
    );
    assert_eq!(rig.deadline(), Some(1_150));
    let Pose::Moving(layout) = rig.scene(OUT, 175).pose else {
        panic!("morph")
    };
    assert!(layout.ring > 0. && layout.ring < 1., "{}", layout.ring);
    assert_eq!(rig.scene(OUT, 175).pose, rig.scene(OUT, 175).pose);
    assert_eq!(rig.scene(OUT, 250).pose, Pose::Moving(released()));
    assert_eq!(farewell_line(&rig, 250), Some(("Goodnight, Alyssa".into(), 0.)));
    assert_eq!(farewell_line(&rig, 700), Some(("Goodnight, Alyssa".into(), 1.)));
    assert!(rig.scene(OUT, 700).animating);
    assert_eq!(rig.pill(700), None);
    assert_eq!(rig.send(1_149, EventKind::Timer), none());
    assert_eq!(rig.send(1_150, EventKind::Timer), ["power Shutdown"]);
    assert_eq!(rig.panel(1_150).line, None);
    assert!(!rig.scene(OUT, 1_150).animating);
    assert_eq!(rig.scene(OUT, 1_150).pose, Pose::Moving(released()));
    assert_eq!(rig.deadline(), Some(16_150));
    // Once is enough.
    assert_eq!(rig.send(1_200, EventKind::Timer), none());
}

#[test]
fn a_suspend_has_no_farewell_and_leaves_after_the_release() {
    let mut rig = leaving_screen();
    rig.key(100, Key::Leave(Leave::Suspend));
    assert_eq!(rig.deadline(), Some(250));
    assert_eq!(farewell_line(&rig, 200), None);
    assert_eq!(rig.send(250, EventKind::Timer), ["power Suspend"]);
}

#[test]
fn a_user_with_farewells_off_leaves_without_one() {
    let mut rig = login_screen();
    rig.send(10, pw(1, prompt()));
    assert_eq!(rig.key(100, Key::Leave(Leave::Restart)), ["kill password 1"]);
    assert_eq!(farewell_line(&rig, 300), None);
    assert_eq!(rig.send(250, EventKind::Timer), ["power Restart"]);
}

#[test]
fn reduced_motion_shows_the_farewell_at_once_with_no_frames() {
    let (mut rig, _) = Rig::new(Settings {
        reduced_motion: true,
        ..login_settings(false)
    });
    rig.send(0, EventKind::Locked);
    rig.send(5, look_full("alyssa", false, Some(farewells())));
    rig.send(10, pw(1, prompt()));
    rig.key(100, Key::Leave(Leave::Restart));
    assert_eq!(farewell_line(&rig, 100), Some(("Back soon, Alyssa".into(), 1.)));
    assert_eq!(farewell_line(&rig, 900), Some(("Back soon, Alyssa".into(), 1.)));
    assert!(!rig.scene(OUT, 500).animating);
    assert_eq!(rig.deadline(), Some(1_000));
    assert_eq!(rig.send(1_000, EventKind::Timer), ["power Restart"]);
}

#[test]
fn input_cannot_cancel_a_departure_but_wakes_the_screen_once_the_machine_had_its_moment() {
    let mut rig = leaving_screen();
    rig.key(100, Key::Leave(Leave::Hibernate));
    assert_eq!(rig.key(120, Key::Char('q')), none());
    assert_eq!(rig.send(130, EventKind::UserChip(OUT)), none());
    assert_eq!(rig.send(140, EventKind::Pointer(OUT)), none());
    assert_eq!(rig.pill(140), None);
    assert_eq!(rig.send(250, EventKind::Timer), ["power Hibernate"]);
    // Too soon after the hand-over: a mouse that is still moving is not a wake.
    assert_eq!(rig.send(1_000, EventKind::Pointer(OUT)), none());
    assert_eq!(rig.panel(1_000).line, None);
    // After a suspend the first key wakes the login screen and is the first character.
    assert_eq!(
        rig.key(1_800, Key::Char('p')),
        ["spawn password 3", "spawn fingerprint 4"]
    );
    assert_eq!(rig.pill(1_800).unwrap().dots, 1);
    assert_eq!(rig.hint(1_800), "The sensor still works");
    let Pose::Moving(layout) = rig.scene(OUT, 1_800 + 250).pose else {
        panic!("morph")
    };
    assert_eq!(layout.ring, 1.);
}

#[test]
fn a_power_action_that_never_happens_brings_the_login_screen_back() {
    let mut rig = leaving_screen();
    rig.key(100, Key::Leave(Leave::Shutdown));
    assert_eq!(rig.send(1_150, EventKind::Timer), ["power Shutdown"]);
    assert_eq!(rig.send(16_149, EventKind::Timer), none());
    assert_eq!(
        rig.send(16_150, EventKind::Timer),
        ["spawn password 3", "spawn fingerprint 4"]
    );
    assert_eq!(rig.hint(16_150), "Touch the sensor, or just start typing");
}

#[test]
fn the_power_chip_opens_offers_the_occasions_and_closes_with_escape() {
    let mut rig = login_prompting();
    assert!(!login_chips(&rig, 20).power_open);
    assert_eq!(rig.send(30, EventKind::PowerChip(OUT)), none());
    assert!(login_chips(&rig, 30).power_open);
    rig.type_text(40, "pw");
    assert_eq!(rig.key(50, Key::Escape), none());
    assert!(!login_chips(&rig, 50).power_open);
    // The password was not cleared by the Escape that closed the menu.
    assert_eq!(rig.pill(50).unwrap().dots, 2);
    rig.send(60, EventKind::PowerChip(OUT));
    assert_eq!(rig.send(70, EventKind::PowerChip(OUT)), none());
    assert!(!login_chips(&rig, 70).power_open);
}

#[test]
fn choosing_an_occasion_from_the_chip_leaves_like_its_shortcut() {
    let mut rig = leaving_screen();
    rig.send(30, EventKind::PowerChip(OUT));
    assert_eq!(
        rig.send(40, EventKind::LeaveChip(OUT, Leave::Restart)),
        ["kill password 1", "kill fingerprint 2"]
    );
    assert_eq!(rig.deadline(), Some(1_090));
    assert_eq!(farewell_line(&rig, 400).unwrap().0, "Back soon, Alyssa");
    assert_eq!(rig.send(1_090, EventKind::Timer), ["power Restart"]);
}

#[test]
fn a_departure_closes_the_menu_and_the_name_editor() {
    let mut rig = login_prompting();
    rig.key(100, Key::Tab);
    type_name(&mut rig, 110, "bo");
    // The editor owns the keys while it is open, the leave shortcuts included.
    assert_eq!(rig.key(200, Key::Leave(Leave::Shutdown)), none());
    assert_eq!(rig.panel(200).line.is_some(), true);
    rig.key(210, Key::Escape);
    rig.send(220, EventKind::PowerChip(OUT));
    assert!(login_chips(&rig, 220).power_open);
    assert_eq!(rig.key(230, Key::Leave(Leave::Shutdown)), ["kill password 1"]);
    let chips = login_chips(&rig, 240);
    assert_eq!((chips.power_open, chips.editing), (false, None));
}

#[test]
fn an_unlocking_lock_screen_ignores_the_display_managers_answers() {
    let mut rig = prompting();
    rig.send(1_000, pw(1, finished(PamOutcome::Success)));
    assert_eq!(rig.send(1_050, EventKind::SessionFailed("x".into())), none());
    assert_eq!(rig.send(1_060, EventKind::SessionStarted), none());
    assert!(rig.deadline().is_some());
}

#[test]
fn a_fingerprint_login_says_nothing_about_the_keyring_it_leaves_locked() {
    let mut rig = login_screen_with_fingerprint();
    rig.send(5, look_with("alyssa", true));
    rig.send(10, pw(1, prompt()));
    rig.send(500, fp(2, finished(PamOutcome::Success)));
    for ms in [500, 700, 1_200, 1_500, 1_800, 3_000] {
        rig.send(ms, EventKind::Timer);
        let panel = rig.panel(ms);
        let words = match &panel.line {
            Some(Line::Hint(text) | Line::Greeting { text, .. } | Line::Farewell { text, .. }) => {
                text.to_lowercase()
            }
            None => String::new(),
        };
        assert!(!words.contains("keyring") && !words.contains("secret"), "{ms} ms: {words}");
        assert!(!panel.chips.user.to_lowercase().contains("keyring"));
    }
}
