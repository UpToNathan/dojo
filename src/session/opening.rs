//! The opening transition as a pure reducer: events in, effects out.
//!
//! An opening starts from the closed state, shows the greeting, then opens the
//! desktop frame. The shell plays it for a login and the lock screen's unlock
//! sequence has the same shape, so both share this timeline. Time arrives in
//! events and nothing here reads a clock.
//!
//! The picture follows one timeline that an [`EventKind::Input`] can jump from
//! the greeting to the opening. The hard cap follows wall time from
//! [`Opening::begin`] and cannot be skipped, so a stalled shell can never leave
//! a permanent cover.
//!
//! An unlock opening is drawn by the shell over the live desktop after the
//! locker has gone. Its parked windows may not be visible yet when it begins, so
//! it holds the closed state until its first frame is presented
//! ([`EventKind::Presented`]) and then runs from that frame. A key press cuts it
//! short instead of cancelling it.
//!
//! Every timing the locker's unlock sequence shares with the shell is in the
//! table below, and `lock::model` takes its numbers from here.

use std::time::{Duration, Instant};

use super::closing::{CLOSING, HARD_CAP, ratio, smooth};
use super::settings::SessionSettings;
use super::transition::{self, Style};
use super::{Direction, Occasion};

pub const GREETING_IN: Duration = Duration::from_millis(150);
/// The greeting's hold when the opening that follows is this machine's own.
pub const GREETING_HOLD: Duration = Duration::from_millis(650);
/// The shorter hold when the shell draws the unlock opening after the locker: the
/// locker's greeting then costs 750 ms before the opening, not 950.
pub const GREETING_HOLD_HANDOFF: Duration = Duration::from_millis(450);
pub const GREETING_OUT: Duration = Duration::from_millis(150);
/// The frame opens out of the closed state. The same movement as a closing, in reverse.
pub const OPENING: Duration = CLOSING;
/// With no greeting to fade out, the locker lets its ring, date, hint and chips go over this
/// before it hands the closed disc to the shell.
pub const RELEASE: Duration = GREETING_OUT;

/// The unlock opening over the live desktop starts with a quick lead: the wallpaper
/// fades out in the first `LEAD`, which takes the progress from the closed state to
/// `LEAD_PROGRESS`, where the wallpaper is gone and the live desktop can show inside the
/// disc. Played at the closing's pace the lead is about 170 ms of a dark disc on a dark
/// surround that barely changes, so the opening has no visible shape until it is well under way.
/// After it the progress is linear again, so every part still eases in its own window.
pub const LEAD: Duration = Duration::from_millis(45);
pub const LEAD_PROGRESS: f32 = 0.62;

/// Closing progress `elapsed` into an opening of `total` that has a lead.
fn lead_progress(elapsed: Duration, total: Duration) -> f32 {
    let t = transition::progress(elapsed, total);
    let lead = ratio(LEAD, total);
    if t < lead {
        1. - (1. - LEAD_PROGRESS) * t / lead
    } else {
        LEAD_PROGRESS * (1. - (t - lead) / (1. - lead))
    }
}

/// The hold of the greeting, which depends on who draws the opening after it.
pub const fn greeting_hold(handoff: bool) -> Duration {
    if handoff {
        GREETING_HOLD_HANDOFF
    } else {
        GREETING_HOLD
    }
}

/// Offsets from `begin` at which the greeting changes. The opening starts at `end`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Greeting {
    fade_in: Duration,
    hold: Duration,
    end: Duration,
}

/// The whole timeline. `greeting` is `None` when no line is shown, in which
/// case the opening starts at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Timing {
    greeting: Option<Greeting>,
    opening: Duration,
}

impl Timing {
    /// When the opening starts.
    fn opening_start(self) -> Duration {
        self.greeting.map_or(Duration::ZERO, |g| g.end)
    }

    /// When the timeline is over.
    fn end(self) -> Duration {
        self.opening_start() + self.opening
    }
}

/// The table. `greeting` says whether a line is shown; `transition` off drops everything.
const fn timing(greeting: bool, transition: bool) -> Timing {
    if !transition {
        return Timing {
            greeting: None,
            opening: Duration::ZERO,
        };
    }
    let greeting = if greeting {
        let fade_in = GREETING_IN;
        let hold = fade_in.saturating_add(GREETING_HOLD);
        Some(Greeting {
            fade_in,
            hold,
            end: hold.saturating_add(GREETING_OUT),
        })
    } else {
        None
    };
    Timing {
        greeting,
        opening: OPENING,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// The frame is closed and the greeting is showing.
    Greeting,
    /// The frame is opening.
    Opening,
    Done,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    /// A deadline from [`Opening::next_deadline`] passed. A spurious one is a no-op.
    Tick,
    /// A key or pointer press. Skips the greeting and never cancels. Over the live
    /// desktop it also cuts the opening short.
    Input,
    /// The first frame of the opening is on screen. Only the opening over the live
    /// desktop waits for it; the first one counts.
    Presented,
}

#[derive(Clone, Copy, Debug)]
pub struct Event {
    pub now: Instant,
    pub kind: EventKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Remove the cover. Idempotent.
    DismissOverlay,
    /// The machine has nothing left to do. Issued once.
    Done,
}

#[derive(Clone, Debug)]
pub struct Opening {
    style: Style,
    greeting: Option<String>,
    timing: Timing,
    started: Instant,
    /// When the opening was requested. The hard cap counts from here and never moves.
    begun: Instant,
    /// An opening over the live desktop (unlock), which waits for its first frame
    /// and is cut short by input.
    live: bool,
    presented: bool,
    /// Timeline time jumped over by input.
    skipped: Duration,
    phase: Phase,
}

impl Opening {
    /// Starts an opening. `greeting` is the line to show, if any; the
    /// `greeting` switch and the `transition` switch can both drop it.
    ///
    /// With `transition` off there is no animation: the machine is finished and
    /// no cover is wanted, so no `DismissOverlay` is sent. A closing occasion
    /// has nothing to open and returns a finished machine too.
    pub fn begin(
        occasion: Occasion,
        settings: &SessionSettings,
        greeting: Option<String>,
        now: Instant,
    ) -> (Opening, Vec<Effect>) {
        let style = if settings.reduced_motion {
            Style::Crossfade
        } else {
            Style::Morph
        };
        let playing = settings.transition && occasion.direction() == Direction::Opening;
        let timing = timing(settings.greeting && greeting.is_some(), playing);
        let phase = match (playing, timing.greeting) {
            (false, _) => Phase::Done,
            (true, Some(_)) => Phase::Greeting,
            (true, None) => Phase::Opening,
        };
        let opening = Opening {
            style,
            greeting: greeting.filter(|_| timing.greeting.is_some()),
            timing,
            started: now,
            begun: now,
            live: occasion == Occasion::Unlock,
            presented: occasion != Occasion::Unlock,
            skipped: Duration::ZERO,
            phase,
        };
        let effects = if phase == Phase::Done {
            vec![Effect::Done]
        } else {
            vec![]
        };
        (opening, effects)
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// Whether the opening is running from its first frame. False only for an
    /// unlock opening that no frame has been shown for yet.
    pub fn presented(&self) -> bool {
        self.presented
    }

    pub fn style(&self) -> Style {
        self.style
    }

    pub fn greeting(&self) -> Option<&str> {
        self.greeting.as_deref()
    }

    /// The next instant at which `update(Tick)` has something to do.
    pub fn next_deadline(&self) -> Option<Instant> {
        let boundary = match self.phase {
            Phase::Done => return None,
            Phase::Greeting => self.timing.opening_start(),
            Phase::Opening => self.timing.end(),
        };
        let capped = self.begun + HARD_CAP;
        if !self.presented {
            return Some(capped);
        }
        Some(self.on_timeline(boundary).min(capped))
    }

    /// Closing progress for `session::transition::layout`, from 1 (the closed
    /// state) down to 0 (the shell's frame): the closing's linear progress run backwards.
    pub fn progress(&self, now: Instant) -> f32 {
        if self.timing.opening.is_zero() {
            return 0.;
        }
        if !self.presented {
            return 1.;
        }
        let into = self
            .elapsed(now)
            .saturating_sub(self.timing.opening_start());
        if self.live {
            return lead_progress(into, self.timing.opening);
        }
        1. - transition::progress(into, self.timing.opening)
    }

    /// Alpha of the greeting line: 0 outside it, 1 through the hold.
    pub fn greeting_alpha(&self, now: Instant) -> f32 {
        let Some(g) = self.timing.greeting else {
            return 0.;
        };
        let elapsed = self.elapsed(now);
        if elapsed >= g.end {
            0.
        } else if elapsed < g.fade_in {
            smooth(ratio(elapsed, GREETING_IN))
        } else if elapsed < g.hold {
            1.
        } else {
            1. - smooth(ratio(elapsed - g.hold, GREETING_OUT))
        }
    }

    /// Whether the picture changes at `now`: the greeting fading or the frame
    /// opening. The shell asks for frames only while this holds.
    pub fn animating(&self, now: Instant) -> bool {
        match (self.phase, self.timing.greeting) {
            (Phase::Done, _) => false,
            (Phase::Opening, _) => true,
            (Phase::Greeting, Some(g)) => !(g.fade_in..g.hold).contains(&self.elapsed(now)),
            (Phase::Greeting, None) => false,
        }
    }

    /// When the picture starts changing again, if it is standing still now: the
    /// end of the greeting hold. Not a deadline for `update`, only for a redraw.
    pub fn next_motion(&self, now: Instant) -> Option<Instant> {
        let g = self.timing.greeting?;
        let holding = (g.fade_in..g.hold).contains(&self.elapsed(now));
        (self.phase == Phase::Greeting && holding).then(|| self.on_timeline(g.hold))
    }

    pub fn update(&mut self, event: Event) -> Vec<Effect> {
        let mut out = Vec::new();
        self.settle(event.now, &mut out);
        if self.phase == Phase::Done {
            return out;
        }
        match event.kind {
            EventKind::Tick => {}
            EventKind::Input => {
                self.skip(event.now);
                self.settle(event.now, &mut out);
            }
            EventKind::Presented => self.present(event.now),
        }
        out
    }

    fn present(&mut self, now: Instant) {
        if !std::mem::replace(&mut self.presented, true) {
            self.started = self.started.max(now);
        }
    }

    fn elapsed(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.started) + self.skipped
    }

    /// Wall time at which the timeline reaches `offset`. Only asked about
    /// offsets still ahead, which are past `skipped`.
    fn on_timeline(&self, offset: Duration) -> Instant {
        self.started + offset.saturating_sub(self.skipped)
    }

    /// Jump from the greeting to the start of the opening. A login's opening is
    /// short and is never cut; the unlock's, over the live desktop, ends at once.
    fn skip(&mut self, now: Instant) {
        let target = match self.phase {
            Phase::Greeting => self.timing.opening_start(),
            Phase::Opening if self.live && self.presented => self.timing.end(),
            Phase::Opening | Phase::Done => return,
        };
        self.skipped += target.saturating_sub(self.elapsed(now));
    }

    /// Runs everything due at `now`, in order, until the machine waits again.
    fn settle(&mut self, now: Instant, out: &mut Vec<Effect>) {
        if self.phase == Phase::Done {
            return;
        }
        if now >= self.begun + HARD_CAP {
            return self.finish(out);
        }
        if !self.presented {
            return;
        }
        let elapsed = self.elapsed(now);
        if self.phase == Phase::Greeting && elapsed >= self.timing.opening_start() {
            self.phase = Phase::Opening;
        }
        if self.phase == Phase::Opening && elapsed >= self.timing.end() {
            self.finish(out);
        }
    }

    fn finish(&mut self, out: &mut Vec<Effect>) {
        self.phase = Phase::Done;
        out.extend([Effect::DismissOverlay, Effect::Done]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn line() -> Option<String> {
        Some("Welcome back, Nathan".into())
    }

    fn on() -> SessionSettings {
        SessionSettings::default()
    }

    fn event(t0: Instant, at: u64, kind: EventKind) -> Event {
        Event {
            now: t0 + ms(at),
            kind,
        }
    }

    /// Ticks at every deadline until done. Effects are paired with the
    /// millisecond at which they were issued.
    fn play(
        settings: SessionSettings,
        greeting: Option<String>,
        inputs: &[(u64, EventKind)],
    ) -> Vec<(u64, Effect)> {
        let t0 = Instant::now();
        let (mut opening, first) = Opening::begin(Occasion::Login, &settings, greeting, t0);
        let mut log: Vec<_> = first.into_iter().map(|e| (0, e)).collect();
        let mut pending: Vec<_> = inputs.to_vec();
        pending.sort_by_key(|(at, _)| *at);
        let mut pending = pending.into_iter().peekable();
        let millis = |at: Instant| at.duration_since(t0).as_millis() as u64;
        while opening.phase() != Phase::Done {
            let deadline = opening.next_deadline().map(millis).expect("a deadline");
            let (at, kind) = match pending.peek().copied() {
                Some((at, kind)) if at <= deadline => {
                    pending.next();
                    (at, kind)
                }
                _ => (deadline, EventKind::Tick),
            };
            log.extend(
                opening
                    .update(event(t0, at, kind))
                    .into_iter()
                    .map(|e| (at, e)),
            );
        }
        log
    }

    fn end(at: u64) -> Vec<(u64, Effect)> {
        vec![(at, Effect::DismissOverlay), (at, Effect::Done)]
    }

    #[test]
    fn the_timeline_follows_the_switches() {
        let table = [
            ("both on", on(), line(), end(1400)),
            (
                "greeting off",
                SessionSettings {
                    greeting: false,
                    ..on()
                },
                line(),
                end(450),
            ),
            ("no line", on(), None, end(450)),
            (
                "reduced motion keeps the timing",
                SessionSettings {
                    reduced_motion: true,
                    ..on()
                },
                line(),
                end(1400),
            ),
            (
                "transition off",
                SessionSettings {
                    transition: false,
                    ..on()
                },
                line(),
                vec![(0, Effect::Done)],
            ),
            (
                "transition off wins over greeting",
                SessionSettings {
                    transition: false,
                    greeting: true,
                    ..on()
                },
                None,
                vec![(0, Effect::Done)],
            ),
        ];
        for (name, settings, greeting, expected) in table {
            assert_eq!(play(settings, greeting, &[]), expected, "{name}");
        }
    }

    #[test]
    fn the_phases_change_at_their_boundaries() {
        let t0 = Instant::now();
        let (mut opening, first) = Opening::begin(Occasion::Login, &on(), line(), t0);
        assert_eq!(first, []);
        assert_eq!(opening.phase(), Phase::Greeting);
        assert_eq!(opening.next_deadline(), Some(t0 + ms(950)));
        assert_eq!(opening.update(event(t0, 949, EventKind::Tick)), []);
        assert_eq!(opening.phase(), Phase::Greeting);
        assert_eq!(opening.update(event(t0, 950, EventKind::Tick)), []);
        assert_eq!(opening.phase(), Phase::Opening);
        assert_eq!(opening.next_deadline(), Some(t0 + ms(1400)));
        assert_eq!(opening.update(event(t0, 1399, EventKind::Tick)), []);
        assert_eq!(
            opening.update(event(t0, 1400, EventKind::Tick)),
            [Effect::DismissOverlay, Effect::Done]
        );
        assert_eq!(opening.phase(), Phase::Done);
        assert_eq!(opening.next_deadline(), None);
    }

    #[test]
    fn without_a_greeting_the_opening_starts_at_once() {
        let t0 = Instant::now();
        let (opening, _) = Opening::begin(Occasion::Login, &on(), None, t0);
        assert_eq!(opening.phase(), Phase::Opening);
        assert_eq!(opening.greeting(), None);
        assert_eq!(opening.next_deadline(), Some(t0 + ms(450)));
    }

    #[test]
    fn a_closing_occasion_has_nothing_to_open() {
        for occasion in Occasion::ALL {
            let t0 = Instant::now();
            let (opening, effects) = Opening::begin(occasion, &on(), line(), t0);
            let plays = occasion.direction() == Direction::Opening;
            assert_eq!(opening.phase() != Phase::Done, plays, "{occasion:?}");
            assert_eq!(effects.is_empty(), plays, "{occasion:?}");
        }
    }

    #[test]
    fn a_spurious_tick_does_nothing() {
        let t0 = Instant::now();
        let (mut opening, _) = Opening::begin(Occasion::Login, &on(), line(), t0);
        assert_eq!(opening.update(event(t0, 10, EventKind::Tick)), []);
        assert_eq!(opening.next_deadline(), Some(t0 + ms(950)));
    }

    #[test]
    fn input_during_the_greeting_starts_the_opening_at_once() {
        for at in [0, 100, 400, 900] {
            let log = play(on(), line(), &[(at, EventKind::Input)]);
            assert_eq!(log, end(at + 450), "input at {at}");
        }
        let t0 = Instant::now();
        let (mut opening, _) = Opening::begin(Occasion::Login, &on(), line(), t0);
        assert_eq!(opening.update(event(t0, 400, EventKind::Input)), []);
        assert_eq!(opening.phase(), Phase::Opening);
        assert_eq!(opening.greeting_alpha(t0 + ms(400)), 0.);
        assert_eq!(opening.progress(t0 + ms(400)), 1.);
        assert_eq!(opening.next_deadline(), Some(t0 + ms(850)));
    }

    #[test]
    fn input_never_cancels_and_only_one_done_comes_out() {
        let inputs = [
            (10, EventKind::Input),
            (20, EventKind::Input),
            (300, EventKind::Input),
            (460, EventKind::Input),
        ];
        let log = play(on(), line(), &inputs);
        assert_eq!(log, end(460));
        let t0 = Instant::now();
        let (mut opening, _) = Opening::begin(Occasion::Login, &on(), line(), t0);
        let mut all = Vec::new();
        for (at, kind) in [
            (100, EventKind::Input),
            (200, EventKind::Input),
            (550, EventKind::Tick),
            (551, EventKind::Input),
            (5000, EventKind::Tick),
            (5001, EventKind::Input),
            (9000, EventKind::Tick),
        ] {
            all.extend(opening.update(event(t0, at, kind)));
        }
        assert_eq!(all, [Effect::DismissOverlay, Effect::Done]);
    }

    #[test]
    fn input_during_the_opening_changes_nothing() {
        let t0 = Instant::now();
        let (mut opening, _) = Opening::begin(Occasion::Login, &on(), line(), t0);
        opening.update(event(t0, 950, EventKind::Tick));
        let before = opening.next_deadline();
        assert_eq!(opening.update(event(t0, 1100, EventKind::Input)), []);
        assert_eq!(opening.next_deadline(), before);
        assert_eq!(opening.phase(), Phase::Opening);
    }

    #[test]
    fn updates_after_done_are_no_ops() {
        let t0 = Instant::now();
        let (mut opening, _) = Opening::begin(Occasion::Login, &on(), line(), t0);
        opening.update(event(t0, 2000, EventKind::Tick));
        assert_eq!(opening.phase(), Phase::Done);
        for kind in [EventKind::Tick, EventKind::Input] {
            assert_eq!(opening.update(event(t0, 3000, kind)), []);
        }
    }

    #[test]
    fn the_hard_cap_removes_the_cover_after_a_stall() {
        let t0 = Instant::now();
        let (mut opening, _) = Opening::begin(Occasion::Login, &on(), line(), t0);
        assert_eq!(
            opening.update(event(t0, 9000, EventKind::Tick)),
            [Effect::DismissOverlay, Effect::Done]
        );
    }

    #[test]
    fn progress_is_exact_at_the_ends_and_never_rises() {
        let t0 = Instant::now();
        let (opening, _) = Opening::begin(Occasion::Login, &on(), line(), t0);
        for at in [0, 500, 950] {
            assert_eq!(opening.progress(t0 + ms(at)), 1., "{at} ms");
        }
        assert!((opening.progress(t0 + ms(1175)) - 0.5).abs() < 1e-5);
        for at in [1400, 9000] {
            assert_eq!(opening.progress(t0 + ms(at)), 0., "{at} ms");
        }
        let mut previous = 1.;
        for at in 0..=1500 {
            let now = opening.progress(t0 + ms(at));
            assert!(now <= previous, "rose at {at} ms");
            previous = now;
        }
        let (plain, _) = Opening::begin(Occasion::Login, &on(), None, t0);
        assert_eq!(plain.progress(t0), 1.);
        assert_eq!(plain.progress(t0 + ms(450)), 0.);
    }

    #[test]
    fn a_finished_machine_is_open() {
        let t0 = Instant::now();
        let off = SessionSettings {
            transition: false,
            ..on()
        };
        let (opening, _) = Opening::begin(Occasion::Login, &off, line(), t0);
        assert_eq!(opening.progress(t0), 0.);
        assert_eq!(opening.greeting(), None);
        assert_eq!(opening.greeting_alpha(t0 + ms(300)), 0.);
        assert!(!opening.animating(t0));
        assert_eq!(opening.next_motion(t0), None);
    }

    #[test]
    fn the_greeting_fades_in_holds_and_fades_out() {
        let t0 = Instant::now();
        let (opening, _) = Opening::begin(Occasion::Login, &on(), line(), t0);
        assert_eq!(opening.greeting(), Some("Welcome back, Nathan"));
        let alpha = |at| opening.greeting_alpha(t0 + ms(at));
        assert_eq!(alpha(0), 0.);
        assert!((alpha(75) - 0.5).abs() < 1e-6);
        for at in [150, 500, 799] {
            assert_eq!(alpha(at), 1., "{at} ms");
        }
        assert!((alpha(875) - 0.5).abs() < 1e-6);
        for at in [950, 1200, 3000] {
            assert_eq!(alpha(at), 0., "{at} ms");
        }
        let rising: Vec<_> = (0..=150).map(alpha).collect();
        assert!(rising.windows(2).all(|pair| pair[0] <= pair[1]));
        let falling: Vec<_> = (800..=950).map(alpha).collect();
        assert!(falling.windows(2).all(|pair| pair[0] >= pair[1]));
    }

    #[test]
    fn the_greeting_switch_and_a_missing_line_remove_the_text() {
        let t0 = Instant::now();
        let off = SessionSettings {
            greeting: false,
            ..on()
        };
        for (settings, text) in [(off, line()), (on(), None)] {
            let (opening, _) = Opening::begin(Occasion::Login, &settings, text, t0);
            assert_eq!(opening.greeting(), None);
            assert_eq!(opening.greeting_alpha(t0 + ms(500)), 0.);
        }
    }

    #[test]
    fn reduced_motion_changes_the_style_only() {
        let t0 = Instant::now();
        let reduced = SessionSettings {
            reduced_motion: true,
            ..on()
        };
        let style = |settings| Opening::begin(Occasion::Login, &settings, line(), t0).0.style();
        assert_eq!(style(reduced), Style::Crossfade);
        assert_eq!(style(on()), Style::Morph);
    }

    #[test]
    fn the_picture_moves_when_fading_and_opening_but_not_in_the_hold() {
        let t0 = Instant::now();
        let (mut opening, _) = Opening::begin(Occasion::Login, &on(), line(), t0);
        let moving = |opening: &Opening, at| opening.animating(t0 + ms(at));
        assert!(moving(&opening, 0) && moving(&opening, 100));
        assert!(!moving(&opening, 150) && !moving(&opening, 500));
        assert_eq!(opening.next_motion(t0 + ms(150)), Some(t0 + ms(800)));
        assert_eq!(opening.next_motion(t0 + ms(800)), None);
        assert!(moving(&opening, 800) && moving(&opening, 900));
        opening.update(event(t0, 950, EventKind::Tick));
        assert!(moving(&opening, 950) && moving(&opening, 1300));
        assert_eq!(opening.next_motion(t0 + ms(1000)), None);
        opening.update(event(t0, 1400, EventKind::Tick));
        assert!(!moving(&opening, 1400));
    }

    #[test]
    fn skipping_moves_the_hold_end_with_the_timeline() {
        let t0 = Instant::now();
        let (mut opening, _) = Opening::begin(Occasion::Login, &on(), line(), t0);
        opening.update(event(t0, 400, EventKind::Input));
        assert_eq!(opening.next_motion(t0 + ms(400)), None);
        assert!(opening.animating(t0 + ms(400)));
    }

    #[test]
    fn unlock_has_the_login_timeline_once_its_first_frame_is_shown() {
        let t0 = Instant::now();
        let deadlines = |occasion| {
            let (mut opening, _) = Opening::begin(occasion, &on(), line(), t0);
            opening.update(event(t0, 0, EventKind::Presented));
            let mut out = Vec::new();
            while let Some(at) = opening.next_deadline() {
                out.push(at.duration_since(t0));
                opening.update(Event {
                    now: at,
                    kind: EventKind::Tick,
                });
            }
            out
        };
        assert_eq!(deadlines(Occasion::Unlock), deadlines(Occasion::Login));
        assert_eq!(deadlines(Occasion::Unlock), [ms(950), ms(1400)]);
    }

    fn unlock() -> (Opening, Instant) {
        let t0 = Instant::now();
        let (opening, effects) = Opening::begin(Occasion::Unlock, &on(), None, t0);
        assert_eq!(effects, []);
        (opening, t0)
    }

    #[test]
    fn an_unlock_opening_holds_the_closed_state_until_its_first_frame() {
        let (mut opening, t0) = unlock();
        assert_eq!(opening.phase(), Phase::Opening);
        for at in [0, 200, 450, 3000] {
            assert_eq!(opening.progress(t0 + ms(at)), 1., "{at} ms");
            assert_eq!(opening.update(event(t0, at, EventKind::Tick)), [], "{at} ms");
        }
        assert!(!opening.presented());
        assert_eq!(opening.next_deadline(), Some(t0 + ms(5000)));
        assert!(opening.animating(t0 + ms(100)));
        opening.update(event(t0, 700, EventKind::Presented));
        assert!(opening.presented());
        assert_eq!(opening.progress(t0 + ms(700)), 1.);
        assert!((opening.progress(t0 + ms(925)) - 0.344_44).abs() < 1e-4);
        assert_eq!(opening.progress(t0 + ms(1150)), 0.);
        assert_eq!(opening.next_deadline(), Some(t0 + ms(1150)));
        assert_eq!(
            opening.update(event(t0, 1150, EventKind::Tick)),
            [Effect::DismissOverlay, Effect::Done]
        );
    }

    #[test]
    fn only_the_first_presented_frame_starts_the_unlock_opening() {
        let (mut opening, t0) = unlock();
        opening.update(event(t0, 100, EventKind::Presented));
        opening.update(event(t0, 300, EventKind::Presented));
        assert_eq!(opening.next_deadline(), Some(t0 + ms(550)));
    }

    #[test]
    fn an_unlock_opening_that_is_never_shown_still_ends_at_the_hard_cap() {
        let (mut opening, t0) = unlock();
        assert_eq!(
            opening.update(event(t0, 5000, EventKind::Tick)),
            [Effect::DismissOverlay, Effect::Done]
        );
    }

    #[test]
    fn a_key_press_cuts_the_unlock_opening_short_without_cancelling_it() {
        let (mut opening, t0) = unlock();
        opening.update(event(t0, 0, EventKind::Presented));
        assert_eq!(opening.update(event(t0, 100, EventKind::Input)), [Effect::DismissOverlay, Effect::Done]);
        assert_eq!(opening.progress(t0 + ms(100)), 0.);
        assert_eq!(opening.update(event(t0, 110, EventKind::Input)), []);
        let (mut unseen, t1) = unlock();
        assert_eq!(unseen.update(event(t1, 100, EventKind::Input)), []);
        assert_eq!(unseen.progress(t1 + ms(100)), 1.);
    }

    #[test]
    fn an_unlock_without_transition_is_finished_at_once() {
        let t0 = Instant::now();
        let off = SessionSettings {
            transition: false,
            ..on()
        };
        let (opening, effects) = Opening::begin(Occasion::Unlock, &off, None, t0);
        assert_eq!(effects, [Effect::Done]);
        assert_eq!(opening.phase(), Phase::Done);
        assert_eq!(opening.progress(t0), 0.);
    }

    #[test]
    fn the_greeting_hold_is_shorter_when_the_shell_draws_the_opening() {
        assert_eq!(greeting_hold(false), ms(650));
        assert_eq!(greeting_hold(true), ms(450));
        assert_eq!(GREETING_IN + greeting_hold(true) + GREETING_OUT, ms(750));
        assert_eq!(RELEASE, GREETING_OUT);
    }

    #[test]
    fn the_unlock_lead_is_continuous_monotonic_and_exact_at_the_ends() {
        let (mut opening, t0) = unlock();
        opening.update(event(t0, 0, EventKind::Presented));
        let at = |n: u64| opening.progress(t0 + ms(n));
        assert_eq!(at(0), 1.);
        assert!((at(45) - LEAD_PROGRESS).abs() < 1e-5);
        assert!((at(15) - (1. - (1. - LEAD_PROGRESS) / 3.)).abs() < 1e-5);
        assert!((at(247) - LEAD_PROGRESS / 2.).abs() < 0.01);
        assert_eq!(at(450), 0.);
        assert_eq!(at(9000), 0.);
        let mut previous = 1.;
        for n in 0..=460 {
            let now = at(n);
            assert!(now <= previous, "rose at {n} ms");
            assert!(previous - now < 0.02, "jumped at {n} ms");
            previous = now;
        }
    }

    #[test]
    fn a_login_opening_has_no_lead() {
        let t0 = Instant::now();
        let (opening, _) = Opening::begin(Occasion::Login, &on(), None, t0);
        assert!((opening.progress(t0 + ms(45)) - 0.9).abs() < 1e-5);
    }
}
