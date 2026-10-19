//! The shell's closing transition as a pure reducer: events in, effects out.
//!
//! The shell draws a cover over the workspace, plays the closing and the
//! farewell, then performs the occasion's action. Time arrives in events and
//! nothing here reads a clock. Every timing constant is in the table below.
//!
//! Two clocks run side by side. The picture (progress, farewell) follows a
//! timeline that an [`EventKind::Input`] can jump forward, and the locker is
//! started on that timeline so the compositor's swap to the lock screen lands on
//! a finished closing. The safety nets (the lock watchdog and the hard cap)
//! follow wall time from [`Closing::begin`] and cannot be skipped, so a key press
//! can never delay or lose them.
//!
//! A lock has no farewell here: the locker plays it (see `session::handoff`).
//!
//! A closing can also be a farewell only: [`Closing::farewell_only`] plays the
//! same picture for an occasion whose action something else performs (logind
//! after the power key, `systemctl poweroff`), and its effects never include a
//! [`Effect::Run`]. A sleeping closing, farewell or not, starts the locker when a
//! lock is asked for, so lock-before-suspend works behind it.

use std::time::{Duration, Instant};

use super::action::{Action, Departure};
use super::settings::SessionSettings;
use super::transition::{self, Style};
use super::Occasion;

/// The frame closes in over the workspace.
pub const CLOSING: Duration = Duration::from_millis(450);
/// Suspend and hibernate: the machine must not be held awake for the animation.
pub const SHORT_CLOSING: Duration = Duration::from_millis(150);
/// The farewell starts fading in at the middle of the closing.
pub const FAREWELL_IN: Duration = Duration::from_millis(150);
pub const FAREWELL_HOLD: Duration = Duration::from_millis(600);
pub const FAREWELL_OUT: Duration = Duration::from_millis(150);
/// A lock starts the locker this long before the closing ends. The locker was
/// measured at 140 ms from start to lock, so the compositor swaps the cover for
/// the lock screen when the closing is visually complete, not part way through.
pub const LOCKER_LEAD: Duration = Duration::from_millis(160);
/// After the closing ends, the locker is started once more if the lock is still unconfirmed.
pub const LOCK_WATCHDOG: Duration = Duration::from_millis(700);
/// From `begin`, the cover always comes down. A failed lock or action leaves no permanent cover.
pub const HARD_CAP: Duration = Duration::from_secs(5);

/// Offsets from `begin` at which the farewell changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Farewell {
    fade_in: Duration,
    hold: Duration,
    fade_out: Duration,
    end: Duration,
}

/// One row of the timeline for an occasion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Timing {
    closing: Duration,
    farewell: Option<Farewell>,
}

/// The table. `farewell` says whether the caller wants one shown.
const fn timing(occasion: Occasion, farewell: bool) -> Timing {
    let short = matches!(occasion, Occasion::Suspend | Occasion::Hibernate);
    if short {
        return Timing {
            closing: SHORT_CLOSING,
            farewell: None,
        };
    }
    if matches!(occasion, Occasion::Lock) {
        return Timing {
            closing: CLOSING,
            farewell: None,
        };
    }
    let farewell = if farewell {
        let fade_in = CLOSING.checked_div(2).unwrap();
        let hold = fade_in.saturating_add(FAREWELL_IN);
        let fade_out = hold.saturating_add(FAREWELL_HOLD);
        Some(Farewell {
            fade_in,
            hold,
            fade_out,
            end: fade_out.saturating_add(FAREWELL_OUT),
        })
    } else {
        None
    };
    Timing {
        closing: CLOSING,
        farewell,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// The frame is closing in.
    Closing,
    /// The frame is closed and the farewell is showing.
    Farewell,
    /// The timeline is over. A lock waits for the compositor; any other action
    /// has been handed to the runner. Only the end of the cover remains.
    Covering,
    Done,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    /// A deadline from [`Closing::next_deadline`] passed. A spurious one is a no-op.
    Tick,
    /// A key or pointer press. Skips ahead and never cancels.
    Input,
    /// The cover's first frame is on screen. Opening a window takes a while, so the
    /// timeline starts here and the first frame is the shell's own frame at progress 0.
    /// Only the first counts, and only while the frame is still closing in.
    Presented,
    /// The compositor reports the session locked.
    LockConfirmed,
}

#[derive(Clone, Copy, Debug)]
pub struct Event {
    pub now: Instant,
    pub kind: EventKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Start `dojo-lock`. Issued `LOCKER_LEAD` before a lock closing ends, and
    /// once more by the watchdog.
    StartLocker,
    /// The occasion's action, once per closing.
    Run(Action),
    /// Remove the cover. Idempotent.
    DismissOverlay,
    /// The machine has nothing left to do.
    Done,
}

impl Effect {
    /// The action an effect asks the runner to perform, if any.
    pub fn action(self) -> Option<Action> {
        match self {
            Effect::StartLocker => Some(Action::StartLocker),
            Effect::Run(action) => Some(action),
            Effect::DismissOverlay | Effect::Done => None,
        }
    }
}

/// Where the locker stands during a lock closing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Locker {
    /// Not started: it starts `LOCKER_LEAD` before the closing ends.
    Waiting,
    /// Started at this instant. The watchdog may start it once more.
    Started(Instant),
    /// The watchdog has fired.
    Retried,
}

/// What the closing is for. A lock starts the locker and waits for the
/// compositor; most others have one action to run; a farewell only plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Goal {
    Lock(Locker),
    Run(Action),
    /// Play the closing and leave the action to whatever started the occasion.
    Farewell,
    /// An opening occasion: there is nothing to close.
    Nothing,
}

#[derive(Clone, Debug)]
pub struct Closing {
    occasion: Occasion,
    goal: Goal,
    /// A lock was asked for during a sleeping closing, and the locker has been started.
    locker_asked: bool,
    style: Style,
    farewell: Option<String>,
    timing: Timing,
    /// When the timeline starts: the closing's begin, moved to the first presented frame.
    started: Instant,
    /// When the closing was requested. The hard cap counts from here and never moves.
    begun: Instant,
    presented: bool,
    /// Timeline time jumped over by input.
    skipped: Duration,
    phase: Phase,
}

impl Closing {
    /// Starts a closing for the occasion's own action. `farewell` is the line to
    /// show, if any; the `farewell` switch, the short closings of suspend and
    /// hibernate, and the `transition` switch can all drop it.
    ///
    /// With `transition` off there is no animation: the action is issued here
    /// together with `Done`, no cover is wanted and no `DismissOverlay` is sent.
    /// An opening occasion returns a finished machine, since it has no closing.
    pub fn begin(
        occasion: Occasion,
        settings: &SessionSettings,
        farewell: Option<String>,
        now: Instant,
    ) -> (Closing, Vec<Effect>) {
        match Departure::of(occasion) {
            Some(departure) => Closing::depart(departure, settings, farewell, now),
            None => Closing::start(occasion, Goal::Nothing, settings, farewell, now),
        }
    }

    /// Like [`Closing::begin`], for a departure that may end in a different action
    /// than its occasion's own.
    pub fn depart(
        departure: Departure,
        settings: &SessionSettings,
        farewell: Option<String>,
        now: Instant,
    ) -> (Closing, Vec<Effect>) {
        let goal = match departure.action() {
            Action::StartLocker => Goal::Lock(Locker::Waiting),
            action => Goal::Run(action),
        };
        Closing::start(departure.occasion(), goal, settings, farewell, now)
    }

    /// Plays the closing and the farewell and performs nothing. For an occasion
    /// that something else carries out, such as logind after the power key. A
    /// lock, which starts the locker itself, and an opening occasion have no
    /// farewell-only closing and return a finished machine.
    pub fn farewell_only(
        occasion: Occasion,
        settings: &SessionSettings,
        farewell: Option<String>,
        now: Instant,
    ) -> (Closing, Vec<Effect>) {
        let goal = match Departure::of(occasion).map(Departure::action) {
            Some(Action::StartLocker) | None => Goal::Nothing,
            Some(_) => Goal::Farewell,
        };
        Closing::start(occasion, goal, settings, farewell, now)
    }

    fn start(
        occasion: Occasion,
        goal: Goal,
        settings: &SessionSettings,
        farewell: Option<String>,
        now: Instant,
    ) -> (Closing, Vec<Effect>) {
        let style = if settings.reduced_motion {
            Style::Crossfade
        } else {
            Style::Morph
        };
        let timing = timing(occasion, settings.farewell && farewell.is_some());
        let mut closing = Closing {
            occasion,
            goal,
            locker_asked: false,
            style,
            farewell: farewell.filter(|_| timing.farewell.is_some()),
            timing,
            started: now,
            begun: now,
            presented: false,
            skipped: Duration::ZERO,
            phase: Phase::Closing,
        };
        let launch = match goal {
            Goal::Lock(_) if !settings.transition => vec![Effect::StartLocker],
            Goal::Run(action) if !settings.transition => vec![Effect::Run(action)],
            Goal::Lock(_) | Goal::Run(_) | Goal::Farewell | Goal::Nothing => vec![],
        };
        if !settings.transition || goal == Goal::Nothing {
            closing.timing = Timing {
                closing: Duration::ZERO,
                farewell: None,
            };
            closing.farewell = None;
            closing.phase = Phase::Done;
            return (closing, [launch, vec![Effect::Done]].concat());
        }
        (closing, launch)
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// The picture has played out. Only the cover is left, which holds the closed
    /// frame until the machine goes down or the hard cap removes it.
    pub fn timeline_over(&self) -> bool {
        matches!(self.phase, Phase::Covering | Phase::Done)
    }

    /// A lock was asked for while this closing is on screen. The frame is already
    /// closed or closing, so a sleeping closing only needs the locker, once; any
    /// other closing ignores the request, as it always has.
    pub fn lock_requested(&mut self) -> Vec<Effect> {
        let sleeping = self.occasion.sleeps() && matches!(self.goal, Goal::Run(_) | Goal::Farewell);
        if !sleeping || self.locker_asked || self.phase == Phase::Done {
            return vec![];
        }
        self.locker_asked = true;
        vec![Effect::StartLocker]
    }

    pub fn style(&self) -> Style {
        self.style
    }

    pub fn farewell(&self) -> Option<&str> {
        self.farewell.as_deref()
    }

    /// The next instant at which `update(Tick)` has something to do.
    pub fn next_deadline(&self) -> Option<Instant> {
        let boundary = match self.phase {
            Phase::Done => return None,
            Phase::Closing => Some(self.on_timeline(self.timing.closing)),
            Phase::Farewell => self.timing.farewell.map(|f| self.on_timeline(f.end)),
            Phase::Covering => None,
        };
        let locker = match self.goal {
            Goal::Lock(Locker::Waiting) => Some(self.on_timeline(self.locker_start())),
            Goal::Lock(Locker::Started(at)) => Some(self.watchdog_at(at)),
            Goal::Lock(Locker::Retried) | Goal::Run(_) | Goal::Farewell | Goal::Nothing => None,
        };
        [boundary, locker, Some(self.begun + HARD_CAP)]
            .into_iter()
            .flatten()
            .min()
    }

    /// Closing progress from 0 (the shell's frame) to 1 (the closed state), for
    /// `session::transition::layout`. It is linear in time: each part of the
    /// frame eases within its own window. It holds at 0 until the cover's first
    /// frame is presented, so that frame is the shell's own, whatever it took to open the window.
    pub fn progress(&self, now: Instant) -> f32 {
        if self.timing.closing.is_zero() {
            return 1.;
        }
        if !self.presented && self.phase == Phase::Closing && self.skipped.is_zero() {
            return 0.;
        }
        transition::progress(self.elapsed(now), self.timing.closing)
    }

    /// Alpha of the farewell line: 0 outside it, 1 through the hold.
    pub fn farewell_alpha(&self, now: Instant) -> f32 {
        let Some(f) = self.timing.farewell else {
            return 0.;
        };
        let elapsed = self.elapsed(now);
        if elapsed < f.fade_in || elapsed >= f.end {
            0.
        } else if elapsed < f.hold {
            smooth(ratio(elapsed - f.fade_in, FAREWELL_IN))
        } else if elapsed < f.fade_out {
            1.
        } else {
            1. - smooth(ratio(elapsed - f.fade_out, FAREWELL_OUT))
        }
    }

    /// Whether the picture changes at `now`: the frame closing in or the
    /// farewell fading. The shell asks for frames only while this holds.
    pub fn animating(&self, now: Instant) -> bool {
        let moving = |from: Duration, to: Duration| (from..to).contains(&self.elapsed(now));
        match self.phase {
            Phase::Done | Phase::Covering => false,
            Phase::Closing => true,
            Phase::Farewell => self
                .timing
                .farewell
                .is_some_and(|f| moving(f.fade_in, f.hold) || moving(f.fade_out, f.end)),
        }
    }

    /// When the picture starts changing again, if it is standing still now: the
    /// end of the farewell hold. Not a deadline for `update`, only for a redraw.
    pub fn next_motion(&self, now: Instant) -> Option<Instant> {
        let f = self.timing.farewell?;
        let holding = (f.hold..f.fade_out).contains(&self.elapsed(now));
        (self.phase == Phase::Farewell && holding).then(|| self.on_timeline(f.fade_out))
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
            EventKind::LockConfirmed => self.confirm(&mut out),
        }
        out
    }

    fn present(&mut self, now: Instant) {
        if self.presented || self.phase != Phase::Closing {
            return;
        }
        self.presented = true;
        self.started = self.started.max(now);
    }

    fn elapsed(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.started) + self.skipped
    }

    /// Wall time at which the timeline reaches `offset`. Only asked about
    /// offsets still ahead, which are past `skipped`.
    fn on_timeline(&self, offset: Duration) -> Instant {
        self.started + offset.saturating_sub(self.skipped)
    }

    /// Jump to the end of the farewell hold, or to the end of the farewell when
    /// it has not started showing yet, so no text appears only to vanish.
    fn skip(&mut self, now: Instant) {
        if !matches!(self.phase, Phase::Closing | Phase::Farewell) {
            return;
        }
        let elapsed = self.elapsed(now);
        let target = match self.timing.farewell {
            Some(f) if elapsed >= f.fade_in => f.fade_out,
            Some(f) => f.end,
            None => self.timing.closing,
        };
        self.skipped += target.saturating_sub(elapsed);
    }

    /// Timeline offset at which a lock starts the locker.
    fn locker_start(&self) -> Duration {
        self.timing.closing.saturating_sub(LOCKER_LEAD)
    }

    /// Wall time of the watchdog for a locker started at `started`: 700 ms after
    /// the closing would have ended, and never sooner than 700 ms after the start.
    fn watchdog_at(&self, started: Instant) -> Instant {
        (self.started + self.timing.closing + LOCK_WATCHDOG).max(started + LOCK_WATCHDOG)
    }

    /// The compositor locked the session: the lock screen is showing, so the
    /// cover has nothing left to hide. It goes at once, in any phase.
    fn confirm(&mut self, out: &mut Vec<Effect>) {
        match self.goal {
            Goal::Lock(_) => self.finish(out),
            Goal::Farewell if self.locker_asked => self.finish(out),
            Goal::Run(_) if self.locker_asked => self.expire(out),
            Goal::Run(_) | Goal::Farewell | Goal::Nothing => {}
        }
    }

    /// Runs everything due at `now`, in order, until the machine waits again.
    fn settle(&mut self, now: Instant, out: &mut Vec<Effect>) {
        while self.phase != Phase::Done {
            if now >= self.begun + HARD_CAP {
                self.expire(out);
                return;
            }
            let elapsed = self.elapsed(now);
            match self.goal {
                Goal::Lock(Locker::Waiting) if elapsed >= self.locker_start() => {
                    self.goal = Goal::Lock(Locker::Started(now));
                    out.push(Effect::StartLocker);
                }
                Goal::Lock(Locker::Started(at)) if now >= self.watchdog_at(at) => {
                    self.goal = Goal::Lock(Locker::Retried);
                    out.push(Effect::StartLocker);
                }
                _ => {}
            }
            match (self.phase, self.timing.farewell) {
                (Phase::Closing, Some(_)) if elapsed >= self.timing.closing => {
                    self.phase = Phase::Farewell;
                }
                (Phase::Closing, None) if elapsed >= self.timing.closing => self.cover(out),
                (Phase::Farewell, Some(f)) if elapsed >= f.end => self.cover(out),
                _ => return,
            }
        }
    }

    /// The timeline is over: perform the action. A lock keeps the cover on the
    /// closed frame until the compositor confirms it.
    fn cover(&mut self, out: &mut Vec<Effect>) {
        self.phase = Phase::Covering;
        if let Goal::Run(action) = self.goal {
            out.push(Effect::Run(action));
        }
    }

    /// The hard cap. A stalled tick must not lose the action.
    fn expire(&mut self, out: &mut Vec<Effect>) {
        if let (Phase::Closing | Phase::Farewell, Goal::Run(action)) = (self.phase, self.goal) {
            out.push(Effect::Run(action));
        }
        self.finish(out);
    }

    fn finish(&mut self, out: &mut Vec<Effect>) {
        self.phase = Phase::Done;
        out.extend([Effect::DismissOverlay, Effect::Done]);
    }
}

pub(super) fn ratio(part: Duration, whole: Duration) -> f32 {
    (part.as_secs_f32() / whole.as_secs_f32()).clamp(0., 1.)
}

/// Smoothstep: exactly 0 at 0 and exactly 1 at 1.
pub(super) fn smooth(x: f32) -> f32 {
    x * x * (3. - 2. * x)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::action::{ActionRunner, RecordingRunner};

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn line() -> Option<String> {
        Some("Goodbye".into())
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
        occasion: Occasion,
        settings: SessionSettings,
        farewell: Option<String>,
        inputs: &[(u64, EventKind)],
    ) -> Vec<(u64, Effect)> {
        let t0 = Instant::now();
        drive(t0, Closing::begin(occasion, &settings, farewell, t0), inputs)
    }

    /// The same as [`play`], for a closing that is already begun.
    fn drive(
        t0: Instant,
        (mut closing, first): (Closing, Vec<Effect>),
        inputs: &[(u64, EventKind)],
    ) -> Vec<(u64, Effect)> {
        let mut log: Vec<_> = first.into_iter().map(|e| (0, e)).collect();
        let mut pending: Vec<_> = inputs.to_vec();
        pending.sort_by_key(|(at, _)| *at);
        let mut pending = pending.into_iter().peekable();
        let millis = |at: Instant| at.duration_since(t0).as_millis() as u64;
        while closing.phase() != Phase::Done {
            let deadline = closing.next_deadline().map(millis);
            let (at, kind) = match (pending.peek().copied(), deadline) {
                (Some((at, kind)), Some(d)) if at <= d => {
                    pending.next();
                    (at, kind)
                }
                (_, Some(d)) => (d, EventKind::Tick),
                (Some((at, kind)), None) => {
                    pending.next();
                    (at, kind)
                }
                (None, None) => panic!("running with no deadline"),
            };
            log.extend(
                closing
                    .update(event(t0, at, kind))
                    .into_iter()
                    .map(|e| (at, e)),
            );
        }
        log
    }

    fn on() -> SessionSettings {
        SessionSettings::default()
    }

    const CAP: [(u64, Effect); 2] = [(5000, Effect::DismissOverlay), (5000, Effect::Done)];

    fn then_cap(mut head: Vec<(u64, Effect)>) -> Vec<(u64, Effect)> {
        head.extend(CAP);
        head
    }

    #[test]
    fn each_closing_occasion_issues_its_effects_at_its_deadlines() {
        let run = |at, action| vec![(at, Effect::Run(action))];
        let table = [
            (
                Occasion::Lock,
                vec![(290, Effect::StartLocker), (1150, Effect::StartLocker)],
            ),
            (Occasion::Logout, run(1125, Action::Logout)),
            (Occasion::Restart, run(1125, Action::Restart)),
            (Occasion::Shutdown, run(1125, Action::Shutdown)),
            (Occasion::Suspend, run(150, Action::Suspend)),
            (Occasion::Hibernate, run(150, Action::Hibernate)),
        ];
        for (occasion, head) in table {
            assert_eq!(
                play(occasion, on(), line(), &[]),
                then_cap(head),
                "{occasion:?}"
            );
        }
    }

    #[test]
    fn without_a_farewell_the_action_runs_when_the_closing_ends() {
        let off = SessionSettings {
            farewell: false,
            ..on()
        };
        assert_eq!(
            play(Occasion::Logout, off, line(), &[]),
            then_cap(vec![(450, Effect::Run(Action::Logout))])
        );
        assert_eq!(
            play(Occasion::Logout, on(), None, &[]),
            then_cap(vec![(450, Effect::Run(Action::Logout))])
        );
        assert_eq!(
            play(Occasion::Lock, off, line(), &[]),
            then_cap(vec![(290, Effect::StartLocker), (1150, Effect::StartLocker)])
        );
    }

    #[test]
    fn suspend_and_hibernate_never_hold_a_farewell() {
        for occasion in [Occasion::Suspend, Occasion::Hibernate] {
            let t0 = Instant::now();
            let (closing, _) = Closing::begin(occasion, &on(), line(), t0);
            assert_eq!(closing.farewell(), None);
            for at in [0, 50, 150, 300, 600] {
                assert_eq!(closing.farewell_alpha(t0 + ms(at)), 0.);
            }
            assert_eq!(closing.next_deadline(), Some(t0 + ms(150)));
        }
    }

    #[test]
    fn with_the_transition_off_the_action_runs_at_once_and_nothing_animates() {
        let off = SessionSettings {
            transition: false,
            ..on()
        };
        assert_eq!(
            play(Occasion::Shutdown, off, line(), &[]),
            [(0, Effect::Run(Action::Shutdown)), (0, Effect::Done)]
        );
        assert_eq!(
            play(Occasion::Lock, off, line(), &[]),
            [(0, Effect::StartLocker), (0, Effect::Done)]
        );
        let t0 = Instant::now();
        let (closing, _) = Closing::begin(Occasion::Logout, &off, line(), t0);
        assert_eq!(closing.phase(), Phase::Done);
        assert_eq!(closing.next_deadline(), None);
        assert_eq!(closing.progress(t0), 1.);
        assert_eq!(closing.farewell(), None);
        assert_eq!(closing.farewell_alpha(t0 + ms(300)), 0.);
    }

    #[test]
    fn opening_occasions_have_nothing_to_close() {
        for occasion in [Occasion::Unlock, Occasion::Login] {
            assert_eq!(play(occasion, on(), line(), &[]), [(0, Effect::Done)]);
        }
    }

    #[test]
    fn reduced_motion_changes_the_style_and_not_the_timing() {
        let reduced = SessionSettings {
            reduced_motion: true,
            ..on()
        };
        let t0 = Instant::now();
        assert_eq!(
            Closing::begin(Occasion::Logout, &reduced, line(), t0)
                .0
                .style(),
            Style::Crossfade
        );
        assert_eq!(
            Closing::begin(Occasion::Logout, &on(), line(), t0)
                .0
                .style(),
            Style::Morph
        );
        assert_eq!(
            play(Occasion::Logout, reduced, line(), &[]),
            then_cap(vec![(1125, Effect::Run(Action::Logout))])
        );
    }

    #[test]
    fn phases_follow_the_timeline() {
        let t0 = Instant::now();
        let (mut closing, _) = Closing::begin(Occasion::Logout, &on(), line(), t0);
        assert_eq!(closing.phase(), Phase::Closing);
        closing.update(event(t0, 449, EventKind::Tick));
        assert_eq!(closing.phase(), Phase::Closing);
        closing.update(event(t0, 450, EventKind::Tick));
        assert_eq!(closing.phase(), Phase::Farewell);
        closing.update(event(t0, 1125, EventKind::Tick));
        assert_eq!(closing.phase(), Phase::Covering);
        closing.update(event(t0, 5000, EventKind::Tick));
        assert_eq!(closing.phase(), Phase::Done);
    }

    #[test]
    fn a_spurious_tick_does_nothing() {
        let t0 = Instant::now();
        let (mut closing, _) = Closing::begin(Occasion::Logout, &on(), line(), t0);
        assert_eq!(closing.update(event(t0, 10, EventKind::Tick)), []);
        assert_eq!(closing.next_deadline(), Some(t0 + ms(450)));
    }

    #[test]
    fn input_during_the_hold_jumps_to_the_fade_out_and_the_action_still_runs() {
        let log = play(
            Occasion::Logout,
            on(),
            line(),
            &[(600, EventKind::Input), (700, EventKind::Input)],
        );
        assert_eq!(log, then_cap(vec![(750, Effect::Run(Action::Logout))]));
    }

    #[test]
    fn input_before_the_farewell_shows_skips_to_the_action_at_once() {
        let log = play(Occasion::Logout, on(), line(), &[(100, EventKind::Input)]);
        assert_eq!(log, then_cap(vec![(100, Effect::Run(Action::Logout))]));
    }

    #[test]
    fn input_without_a_farewell_skips_the_closing() {
        let log = play(Occasion::Restart, on(), None, &[(100, EventKind::Input)]);
        assert_eq!(log, then_cap(vec![(100, Effect::Run(Action::Restart))]));
    }

    #[test]
    fn input_moves_the_picture_and_leaves_the_safety_nets_alone() {
        let t0 = Instant::now();
        let (mut closing, _) = Closing::begin(Occasion::Logout, &on(), line(), t0);
        assert_eq!(closing.update(event(t0, 300, EventKind::Input)), []);
        assert_eq!(closing.phase(), Phase::Farewell);
        assert_eq!(closing.progress(t0 + ms(300)), 1.);
        assert_eq!(closing.farewell_alpha(t0 + ms(300)), 1.);
        assert_eq!(closing.farewell_alpha(t0 + ms(450)), 0.);
        assert_eq!(closing.next_deadline(), Some(t0 + ms(450)));
        assert_eq!(
            closing.update(event(t0, 450, EventKind::Tick)),
            [Effect::Run(Action::Logout)]
        );
        assert_eq!(closing.phase(), Phase::Covering);
        assert_eq!(closing.next_deadline(), Some(t0 + ms(5000)));
    }

    #[test]
    fn input_during_a_lock_closing_starts_the_locker_at_once_and_keeps_the_watchdog() {
        let t0 = Instant::now();
        let (mut closing, _) = Closing::begin(Occasion::Lock, &on(), line(), t0);
        assert_eq!(
            closing.update(event(t0, 100, EventKind::Input)),
            [Effect::StartLocker]
        );
        assert_eq!(closing.phase(), Phase::Covering);
        assert_eq!(closing.progress(t0 + ms(100)), 1.);
        assert_eq!(closing.next_deadline(), Some(t0 + ms(1150)));
        assert_eq!(closing.update(event(t0, 200, EventKind::Input)), []);
        assert_eq!(
            closing.update(event(t0, 1150, EventKind::Tick)),
            [Effect::StartLocker]
        );
    }

    #[test]
    fn the_timeline_starts_at_the_first_presented_frame_and_the_cap_does_not_move() {
        let t0 = Instant::now();
        let (mut closing, _) = Closing::begin(Occasion::Lock, &on(), None, t0);
        assert_eq!(closing.next_deadline(), Some(t0 + ms(290)));
        assert_eq!(closing.update(event(t0, 140, EventKind::Presented)), []);
        assert_eq!(closing.progress(t0 + ms(140)), 0.);
        assert!(closing.progress(t0 + ms(365)) > 0.4);
        assert_eq!(closing.next_deadline(), Some(t0 + ms(430)));
        assert_eq!(closing.update(event(t0, 200, EventKind::Presented)), []);
        assert_eq!(closing.next_deadline(), Some(t0 + ms(430)));
        assert_eq!(
            closing.update(event(t0, 430, EventKind::Tick)),
            [Effect::StartLocker]
        );
        assert_eq!(closing.next_deadline(), Some(t0 + ms(590)));
        assert_eq!(closing.update(event(t0, 590, EventKind::Tick)), []);
        assert_eq!(closing.next_deadline(), Some(t0 + ms(1290)));
        assert_eq!(
            closing.update(event(t0, 5000, EventKind::Tick)),
            [Effect::DismissOverlay, Effect::Done]
        );
    }

    #[test]
    fn a_frame_presented_after_the_closing_ended_changes_nothing() {
        let t0 = Instant::now();
        let (mut closing, _) = Closing::begin(Occasion::Restart, &on(), None, t0);
        closing.update(event(t0, 450, EventKind::Tick));
        assert_eq!(closing.phase(), Phase::Covering);
        assert_eq!(closing.update(event(t0, 600, EventKind::Presented)), []);
        assert_eq!(closing.progress(t0 + ms(600)), 1.);
    }

    #[test]
    fn a_lock_closing_has_no_farewell_and_starts_the_locker_a_lead_before_its_end() {
        let t0 = Instant::now();
        let (mut closing, first) = Closing::begin(Occasion::Lock, &on(), line(), t0);
        assert_eq!(first, []);
        assert_eq!(closing.farewell(), None);
        for at in [0, 300, 450, 600, 1000] {
            assert_eq!(closing.farewell_alpha(t0 + ms(at)), 0., "{at} ms");
        }
        assert_eq!(CLOSING - LOCKER_LEAD, ms(290));
        assert_eq!(closing.next_deadline(), Some(t0 + ms(290)));
        assert_eq!(closing.update(event(t0, 289, EventKind::Tick)), []);
        assert_eq!(
            closing.update(event(t0, 290, EventKind::Tick)),
            [Effect::StartLocker]
        );
        assert_eq!(closing.phase(), Phase::Closing);
        assert_eq!(closing.next_deadline(), Some(t0 + ms(450)));
        assert_eq!(closing.update(event(t0, 450, EventKind::Tick)), []);
        assert_eq!(closing.phase(), Phase::Covering);
        assert!(!closing.animating(t0 + ms(450)));
        assert_eq!(closing.next_deadline(), Some(t0 + ms(1150)));
    }

    #[test]
    fn input_never_cancels_a_lock_or_runs_a_second_action() {
        let log = play(
            Occasion::Lock,
            on(),
            line(),
            &[(10, EventKind::Input), (20, EventKind::Input)],
        );
        let starts = log.iter().filter(|(_, e)| *e == Effect::StartLocker).count();
        assert_eq!(starts, 2);
        assert!(!log.iter().any(|(_, e)| matches!(e, Effect::Run(_))));
    }

    #[test]
    fn input_after_the_action_changes_nothing() {
        let t0 = Instant::now();
        let (mut closing, _) = Closing::begin(Occasion::Suspend, &on(), None, t0);
        assert_eq!(
            closing.update(event(t0, 150, EventKind::Tick)),
            [Effect::Run(Action::Suspend)]
        );
        assert_eq!(closing.update(event(t0, 160, EventKind::Input)), []);
        assert_eq!(closing.update(event(t0, 170, EventKind::Tick)), []);
    }

    #[test]
    fn a_confirmed_lock_drops_the_cover_at_once_even_mid_closing() {
        let log = play(
            Occasion::Lock,
            on(),
            line(),
            &[(300, EventKind::LockConfirmed)],
        );
        assert_eq!(
            log,
            [
                (290, Effect::StartLocker),
                (300, Effect::DismissOverlay),
                (300, Effect::Done)
            ]
        );
    }

    #[test]
    fn a_lock_confirmed_before_the_locker_starts_never_starts_one() {
        let log = play(
            Occasion::Lock,
            on(),
            line(),
            &[(100, EventKind::LockConfirmed)],
        );
        assert_eq!(
            log,
            [(100, Effect::DismissOverlay), (100, Effect::Done)]
        );
    }

    #[test]
    fn a_lock_confirmed_after_the_timeline_drops_the_cover_at_once_and_skips_the_watchdog() {
        let log = play(
            Occasion::Lock,
            on(),
            line(),
            &[(1130, EventKind::LockConfirmed)],
        );
        assert_eq!(
            log,
            [
                (290, Effect::StartLocker),
                (1130, Effect::DismissOverlay),
                (1130, Effect::Done)
            ]
        );
    }

    #[test]
    fn the_watchdog_reissues_the_locker_exactly_once() {
        let t0 = Instant::now();
        let (mut closing, _) = Closing::begin(Occasion::Lock, &on(), None, t0);
        assert_eq!(
            closing.update(event(t0, 290, EventKind::Tick)),
            [Effect::StartLocker]
        );
        assert_eq!(closing.update(event(t0, 450, EventKind::Tick)), []);
        assert_eq!(closing.next_deadline(), Some(t0 + ms(1150)));
        assert_eq!(closing.update(event(t0, 1149, EventKind::Tick)), []);
        assert_eq!(
            closing.update(event(t0, 1150, EventKind::Tick)),
            [Effect::StartLocker]
        );
        assert_eq!(closing.update(event(t0, 2000, EventKind::Tick)), []);
        assert_eq!(closing.next_deadline(), Some(t0 + ms(5000)));
    }

    #[test]
    fn a_failed_lock_cannot_leave_a_permanent_cover() {
        let t0 = Instant::now();
        let (mut closing, first) = Closing::begin(Occasion::Lock, &on(), line(), t0);
        assert_eq!(first, []);
        let mut all = first;
        for at in [290, 450, 1125, 1150, 4999] {
            all.extend(closing.update(event(t0, at, EventKind::Tick)));
            assert_ne!(closing.phase(), Phase::Done, "dismissed early at {at}");
        }
        all.extend(closing.update(event(t0, 5000, EventKind::Tick)));
        assert_eq!(
            all,
            [
                Effect::StartLocker,
                Effect::StartLocker,
                Effect::DismissOverlay,
                Effect::Done
            ]
        );
        assert_eq!(closing.phase(), Phase::Done);
        assert_eq!(closing.next_deadline(), None);
    }

    #[test]
    fn a_stalled_shell_starts_the_locker_once_and_gives_it_the_full_watchdog() {
        let t0 = Instant::now();
        let (mut closing, _) = Closing::begin(Occasion::Lock, &on(), line(), t0);
        assert_eq!(
            closing.update(event(t0, 3000, EventKind::Tick)),
            [Effect::StartLocker]
        );
        assert_eq!(closing.next_deadline(), Some(t0 + ms(3700)));
        assert_eq!(
            closing.update(event(t0, 3700, EventKind::Tick)),
            [Effect::StartLocker]
        );
    }

    #[test]
    fn a_stalled_shell_still_dismisses_a_lock_without_another_locker() {
        let t0 = Instant::now();
        let (mut closing, _) = Closing::begin(Occasion::Lock, &on(), line(), t0);
        assert_eq!(
            closing.update(event(t0, 9000, EventKind::Tick)),
            [Effect::DismissOverlay, Effect::Done]
        );
    }

    #[test]
    fn a_stalled_shell_still_runs_the_action_before_the_cover_comes_down() {
        let t0 = Instant::now();
        let (mut closing, _) = Closing::begin(Occasion::Shutdown, &on(), line(), t0);
        assert_eq!(
            closing.update(event(t0, 9000, EventKind::Tick)),
            [
                Effect::Run(Action::Shutdown),
                Effect::DismissOverlay,
                Effect::Done
            ]
        );
    }

    #[test]
    fn duplicate_lock_confirmations_are_idempotent() {
        let t0 = Instant::now();
        let (mut closing, _) = Closing::begin(Occasion::Lock, &on(), None, t0);
        assert_eq!(
            closing.update(event(t0, 100, EventKind::LockConfirmed)),
            [Effect::DismissOverlay, Effect::Done]
        );
        assert_eq!(closing.update(event(t0, 110, EventKind::LockConfirmed)), []);
        assert_eq!(closing.update(event(t0, 450, EventKind::Tick)), []);
        assert_eq!(closing.update(event(t0, 460, EventKind::LockConfirmed)), []);
    }

    #[test]
    fn lock_confirmation_means_nothing_to_other_occasions() {
        let t0 = Instant::now();
        let (mut closing, _) = Closing::begin(Occasion::Logout, &on(), None, t0);
        assert_eq!(closing.update(event(t0, 100, EventKind::LockConfirmed)), []);
        assert_eq!(closing.phase(), Phase::Closing);
    }

    #[test]
    fn updates_after_done_are_no_ops() {
        for occasion in [Occasion::Lock, Occasion::Logout, Occasion::Suspend] {
            let t0 = Instant::now();
            let (mut closing, _) = Closing::begin(occasion, &on(), line(), t0);
            closing.update(event(t0, 6000, EventKind::Tick));
            assert_eq!(closing.phase(), Phase::Done);
            for kind in [
                EventKind::Tick,
                EventKind::Input,
                EventKind::LockConfirmed,
            ] {
                assert_eq!(closing.update(event(t0, 7000, kind)), [], "{occasion:?}");
            }
            assert_eq!(closing.next_deadline(), None);
        }
    }

    #[test]
    fn progress_is_exact_at_the_ends_and_never_decreases() {
        let t0 = Instant::now();
        let (mut closing, _) = Closing::begin(Occasion::Logout, &on(), line(), t0);
        closing.update(event(t0, 0, EventKind::Presented));
        assert_eq!(closing.progress(t0), 0.);
        assert_eq!(closing.progress(t0 + ms(450)), 1.);
        assert_eq!(closing.progress(t0 + ms(9000)), 1.);
        assert!((closing.progress(t0 + ms(225)) - 0.5).abs() < 1e-5);
        let mut previous = 0.;
        for at in 0..=500 {
            let now = closing.progress(t0 + ms(at));
            assert!(now >= previous, "fell at {at} ms");
            previous = now;
        }
    }

    #[test]
    fn progress_is_linear_and_starts_at_the_presented_frame_whatever_came_before_it() {
        let t0 = Instant::now();
        let (mut closing, _) = Closing::begin(Occasion::Lock, &on(), None, t0);
        for at in [0, 90, 400] {
            assert_eq!(closing.progress(t0 + ms(at)), 0., "{at} ms, before the first frame");
        }
        closing.update(event(t0, 100, EventKind::Presented));
        assert_eq!(closing.progress(t0 + ms(100)), 0.);
        for fraction in 1..=9 {
            let at = 100 + 45 * fraction;
            let want = fraction as f32 / 10.;
            assert!((closing.progress(t0 + ms(at)) - want).abs() < 1e-5, "{at} ms");
        }
        assert_eq!(closing.progress(t0 + ms(550)), 1.);
    }

    #[test]
    fn short_closings_finish_their_progress_in_150_ms() {
        let t0 = Instant::now();
        let (mut closing, _) = Closing::begin(Occasion::Hibernate, &on(), None, t0);
        closing.update(event(t0, 0, EventKind::Presented));
        assert_eq!(closing.progress(t0), 0.);
        assert_eq!(closing.progress(t0 + ms(150)), 1.);
        assert!(closing.progress(t0 + ms(75)) < 1.);
    }

    #[test]
    fn farewell_fades_in_holds_and_fades_out() {
        let t0 = Instant::now();
        let (closing, _) = Closing::begin(Occasion::Logout, &on(), line(), t0);
        assert_eq!(closing.farewell(), Some("Goodbye"));
        let alpha = |at| closing.farewell_alpha(t0 + ms(at));
        for at in [0, 100, 225] {
            assert_eq!(alpha(at), 0., "{at} ms");
        }
        assert!((alpha(300) - 0.5).abs() < 1e-6);
        for at in [375, 600, 974] {
            assert_eq!(alpha(at), 1., "{at} ms");
        }
        assert!((alpha(1050) - 0.5).abs() < 1e-6);
        for at in [1125, 3000] {
            assert_eq!(alpha(at), 0., "{at} ms");
        }
        let rising: Vec<_> = (225..=375).map(alpha).collect();
        assert!(rising.windows(2).all(|pair| pair[0] <= pair[1]));
        let falling: Vec<_> = (975..=1125).map(alpha).collect();
        assert!(falling.windows(2).all(|pair| pair[0] >= pair[1]));
    }

    #[test]
    fn the_farewell_switch_and_a_missing_line_remove_the_text() {
        let t0 = Instant::now();
        let off = SessionSettings {
            farewell: false,
            ..on()
        };
        for (settings, text) in [(off, line()), (on(), None)] {
            let (closing, _) = Closing::begin(Occasion::Shutdown, &settings, text, t0);
            assert_eq!(closing.farewell(), None);
            assert_eq!(closing.farewell_alpha(t0 + ms(500)), 0.);
        }
    }

    #[test]
    fn the_recording_runner_sees_each_action_once_however_often_the_machine_is_poked() {
        let t0 = Instant::now();
        let (mut closing, first) = Closing::begin(Occasion::Logout, &on(), line(), t0);
        let mut runner = RecordingRunner::new();
        let mut effects = first;
        for (at, kind) in [
            (100, EventKind::Input),
            (120, EventKind::Input),
            (450, EventKind::Tick),
            (450, EventKind::Tick),
            (800, EventKind::Input),
            (5000, EventKind::Tick),
            (5001, EventKind::Tick),
        ] {
            effects.extend(closing.update(event(t0, at, kind)));
        }
        for action in effects.into_iter().filter_map(Effect::action) {
            runner.run(action).unwrap();
        }
        assert_eq!(runner.actions, [Action::Logout]);
    }

    #[test]
    fn the_recording_runner_sees_the_locker_twice_when_the_watchdog_fires() {
        let log = play(Occasion::Lock, on(), None, &[]);
        let mut runner = RecordingRunner::new();
        for action in log.into_iter().filter_map(|(_, e)| e.action()) {
            runner.run(action).unwrap();
        }
        assert_eq!(runner.actions, [Action::StartLocker, Action::StartLocker]);
    }

    #[test]
    fn the_picture_moves_while_closing_and_while_the_farewell_fades_but_not_in_the_hold() {
        let t0 = Instant::now();
        let (closing, _) = Closing::begin(Occasion::Logout, &on(), line(), t0);
        let moving = |at| closing.animating(t0 + ms(at));
        assert!(moving(0) && moving(300));
        let mut closing = closing;
        closing.update(event(t0, 450, EventKind::Tick));
        assert_eq!(closing.phase(), Phase::Farewell);
        let moving = |closing: &Closing, at| closing.animating(t0 + ms(at));
        assert!(!moving(&closing, 450) && !moving(&closing, 900));
        assert_eq!(closing.next_motion(t0 + ms(450)), Some(t0 + ms(975)));
        assert!(moving(&closing, 975) && moving(&closing, 1100));
        assert_eq!(closing.next_motion(t0 + ms(975)), None);
        closing.update(event(t0, 1125, EventKind::Tick));
        assert_eq!(closing.phase(), Phase::Covering);
        assert!(!moving(&closing, 1200));
        assert_eq!(closing.next_motion(t0 + ms(1200)), None);
    }

    #[test]
    fn input_during_the_closing_jumps_to_the_fade_out() {
        let t0 = Instant::now();
        let (mut closing, _) = Closing::begin(Occasion::Logout, &on(), line(), t0);
        closing.update(event(t0, 300, EventKind::Input));
        assert_eq!(closing.phase(), Phase::Farewell);
        assert!(closing.animating(t0 + ms(300)));
        assert_eq!(closing.next_motion(t0 + ms(300)), None);
        assert_eq!(closing.next_deadline(), Some(t0 + ms(450)));
    }

    #[test]
    fn a_finished_machine_never_animates() {
        let t0 = Instant::now();
        let off = SessionSettings {
            transition: false,
            ..on()
        };
        let (closing, _) = Closing::begin(Occasion::Shutdown, &off, line(), t0);
        assert!(!closing.animating(t0));
        assert_eq!(closing.next_motion(t0), None);
    }

    fn farewell_log(occasion: Occasion, settings: SessionSettings, inputs: &[(u64, EventKind)]) -> Vec<(u64, Effect)> {
        let t0 = Instant::now();
        drive(t0, Closing::farewell_only(occasion, &settings, line(), t0), inputs)
    }

    #[test]
    fn a_farewell_only_closing_plays_the_same_picture_and_never_runs_an_action() {
        for occasion in [
            Occasion::Logout,
            Occasion::Restart,
            Occasion::Shutdown,
            Occasion::Suspend,
            Occasion::Hibernate,
        ] {
            assert_eq!(farewell_log(occasion, on(), &[]), CAP, "{occasion:?}");
            let t0 = Instant::now();
            let (own, _) = Closing::begin(occasion, &on(), line(), t0);
            let (only, _) = Closing::farewell_only(occasion, &on(), line(), t0);
            assert_eq!(only.next_deadline(), own.next_deadline(), "{occasion:?}");
            assert_eq!(only.farewell(), own.farewell(), "{occasion:?}");
            for at in [0, 100, 300, 700, 1000] {
                assert_eq!(only.progress(t0 + ms(at)), own.progress(t0 + ms(at)), "{occasion:?}");
                assert_eq!(
                    only.farewell_alpha(t0 + ms(at)),
                    own.farewell_alpha(t0 + ms(at)),
                    "{occasion:?}"
                );
            }
        }
    }

    #[test]
    fn a_farewell_only_closing_ends_its_timeline_when_the_action_would_have_run() {
        let table = [
            (Occasion::Shutdown, 1125),
            (Occasion::Restart, 1125),
            (Occasion::Logout, 1125),
            (Occasion::Suspend, 150),
            (Occasion::Hibernate, 150),
        ];
        for (occasion, end) in table {
            let t0 = Instant::now();
            let (mut closing, _) = Closing::farewell_only(occasion, &on(), line(), t0);
            assert!(!closing.timeline_over(), "{occasion:?}");
            assert!(closing.update(event(t0, end - 1, EventKind::Tick)).is_empty());
            assert!(!closing.timeline_over(), "{occasion:?}");
            assert!(closing.update(event(t0, end, EventKind::Tick)).is_empty());
            assert!(closing.timeline_over(), "{occasion:?}");
            assert_eq!(closing.phase(), Phase::Covering, "{occasion:?}");
        }
    }

    #[test]
    fn the_timeline_of_an_acting_closing_is_over_once_the_action_has_run() {
        let t0 = Instant::now();
        let (mut closing, _) = Closing::begin(Occasion::Shutdown, &on(), line(), t0);
        assert!(!closing.timeline_over());
        assert_eq!(
            closing.update(event(t0, 1125, EventKind::Tick)),
            [Effect::Run(Action::Shutdown)]
        );
        assert!(closing.timeline_over());
    }

    #[test]
    fn a_farewell_only_closing_survives_input_stalls_and_the_cap_without_an_action() {
        let off = SessionSettings {
            transition: false,
            ..on()
        };
        assert_eq!(farewell_log(Occasion::Shutdown, off, &[]), [(0, Effect::Done)]);
        assert_eq!(
            farewell_log(
                Occasion::Shutdown,
                on(),
                &[(100, EventKind::Input), (200, EventKind::Input), (900, EventKind::Input)]
            ),
            CAP
        );
        let t0 = Instant::now();
        let (mut stalled, _) = Closing::farewell_only(Occasion::Restart, &on(), line(), t0);
        assert_eq!(
            stalled.update(event(t0, 7000, EventKind::Tick)),
            [Effect::DismissOverlay, Effect::Done]
        );
        assert!(stalled.timeline_over());
    }

    #[test]
    fn a_farewell_only_closing_ignores_a_lock_confirmation_and_the_locker_unless_asked() {
        let t0 = Instant::now();
        let (mut closing, _) = Closing::farewell_only(Occasion::Shutdown, &on(), line(), t0);
        assert!(closing.update(event(t0, 100, EventKind::LockConfirmed)).is_empty());
        assert_eq!(closing.phase(), Phase::Closing);
        assert!(closing.lock_requested().is_empty());
    }

    #[test]
    fn locks_and_openings_have_no_farewell_only_closing() {
        for occasion in [Occasion::Lock, Occasion::Unlock, Occasion::Login] {
            assert_eq!(farewell_log(occasion, on(), &[]), [(0, Effect::Done)], "{occasion:?}");
        }
    }

    #[test]
    fn a_sleeping_closing_starts_the_locker_once_when_a_lock_is_asked_for() {
        for occasion in [Occasion::Suspend, Occasion::Hibernate] {
            let t0 = Instant::now();
            let (mut farewell, _) = Closing::farewell_only(occasion, &on(), line(), t0);
            assert_eq!(farewell.lock_requested(), [Effect::StartLocker], "{occasion:?}");
            assert!(farewell.lock_requested().is_empty(), "{occasion:?}");
            let (mut acting, _) = Closing::begin(occasion, &on(), line(), t0);
            acting.update(event(t0, 150, EventKind::Tick));
            assert_eq!(acting.lock_requested(), [Effect::StartLocker], "{occasion:?}");
            assert!(acting.lock_requested().is_empty(), "{occasion:?}");
        }
    }

    #[test]
    fn other_closings_ignore_a_lock_request() {
        for occasion in [Occasion::Logout, Occasion::Restart, Occasion::Shutdown, Occasion::Lock] {
            let t0 = Instant::now();
            let (mut acting, _) = Closing::begin(occasion, &on(), line(), t0);
            assert!(acting.lock_requested().is_empty(), "{occasion:?}");
        }
        let t0 = Instant::now();
        let off = SessionSettings {
            transition: false,
            ..on()
        };
        let (mut done, _) = Closing::begin(Occasion::Suspend, &off, line(), t0);
        assert!(done.lock_requested().is_empty());
    }

    #[test]
    fn a_confirmed_lock_drops_a_sleeping_closing_without_losing_its_action() {
        let t0 = Instant::now();
        let (mut closing, _) = Closing::begin(Occasion::Suspend, &on(), None, t0);
        closing.lock_requested();
        assert_eq!(
            closing.update(event(t0, 100, EventKind::LockConfirmed)),
            [Effect::Run(Action::Suspend), Effect::DismissOverlay, Effect::Done]
        );
        let (mut after, _) = Closing::begin(Occasion::Suspend, &on(), None, t0);
        after.update(event(t0, 150, EventKind::Tick));
        after.lock_requested();
        assert_eq!(
            after.update(event(t0, 300, EventKind::LockConfirmed)),
            [Effect::DismissOverlay, Effect::Done]
        );
        let (mut farewell, _) = Closing::farewell_only(Occasion::Hibernate, &on(), None, t0);
        farewell.lock_requested();
        assert_eq!(
            farewell.update(event(t0, 100, EventKind::LockConfirmed)),
            [Effect::DismissOverlay, Effect::Done]
        );
        let (mut unasked, _) = Closing::begin(Occasion::Suspend, &on(), None, t0);
        assert!(unasked.update(event(t0, 100, EventKind::LockConfirmed)).is_empty());
    }

    #[test]
    fn a_departure_with_another_action_runs_that_action_on_its_occasions_timeline() {
        let t0 = Instant::now();
        let departure = Departure::parse("suspend-then-hibernate").unwrap();
        assert_eq!(
            drive(t0, Closing::depart(departure, &on(), line(), t0), &[]),
            then_cap(vec![(150, Effect::Run(Action::SuspendThenHibernate))])
        );
    }

    #[test]
    fn the_recording_runner_sees_a_farewell_only_closing_do_nothing_and_a_departure_once() {
        let mut runner = RecordingRunner::new();
        for (_, effect) in farewell_log(Occasion::Shutdown, on(), &[]) {
            if let Some(action) = effect.action() {
                runner.run(action).unwrap();
            }
        }
        assert!(runner.actions.is_empty());
    }
}
