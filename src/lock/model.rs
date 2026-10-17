//! Every lock rule in one reducer: stages, the password flow, the fingerprint
//! state, the power timeline, the active output and the unlock sequence.
//! Time arrives in events; nothing here reads a clock.

use std::collections::BTreeMap;
use std::fmt;
use std::mem;
use std::time::{Duration, Instant};

use gpui::{Pixels, Size};

use super::pam::{Method, Notice, PamOutcome, Report, WorkerId};
use super::power::{Display, PowerPhase, Sensor, rules};
use super::secret::Password;
use crate::session::Occasion;
use crate::session::message::{self, Condition};
use crate::session::opening;

/// One reading of both clocks. `unix` is only for the minute and the labels.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Now {
    pub mono: Instant,
    pub unix: i64,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Debug)]
pub struct OutputId(pub u64);

/// An output that should have a lock window. `size` is in logical pixels.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Output {
    pub id: OutputId,
    pub size: Size<Pixels>,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Settings {
    pub reduced_motion: bool,
    /// The name in the user chip.
    pub user: String,
    /// The unlock greetings, chosen by the edge before the lock. The line for
    /// the actual lock length is picked when authentication succeeds.
    /// `None` skips straight to the opening.
    pub greeting: Option<GreetingSet>,
    /// The shell's top-bar controls width, so the opening ends on the shell's own clock bounds.
    pub controls_width: f32,
    /// When the session was first locked, from a takeover marker left by an
    /// earlier locker. `None` counts the lock age from this locker's `Locked`.
    pub lock_age_origin: Option<i64>,
    /// The farewell the shell chose, played over the resting dial from `Locked`.
    /// `None` for no farewell: the shell left none, or the setting is off.
    pub farewell: Option<String>,
}

/// One unlock greeting per lock-length bucket of the greeting rules. The lock
/// length is only known at unlock, so all three are selected up front.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct GreetingSet {
    /// Locked under two minutes.
    pub short: String,
    /// Neither short nor long.
    pub normal: String,
    /// Locked eight hours or more.
    pub long: String,
}

impl GreetingSet {
    /// Selects each bucket's line from `base` (hour, previous line, seed).
    /// The lock-length thresholds stay in `session::message`: `ZERO` falls in
    /// the short bucket, `MAX` in the long one, and no length in neither.
    pub fn choose(base: &message::Context, name: &str) -> GreetingSet {
        let with = |locked_for| {
            let context = message::Context {
                locked_for,
                ..base.clone()
            };
            message::select(Occasion::Unlock, &context, name)
        };
        GreetingSet {
            short: with(Some(Duration::ZERO)),
            normal: with(None),
            long: with(Some(Duration::MAX)),
        }
    }

    /// The line for a lock that lasted `locked_for`, bucketed by the greeting rules themselves.
    pub(super) fn pick(&self, locked_for: Duration) -> &str {
        let context = message::Context {
            locked_for: Some(locked_for),
            ..message::Context::default()
        };
        let eligible = message::eligible(Occasion::Unlock, &context);
        let holds = |condition| eligible.iter().any(|entry| entry.when == condition);
        if holds(Condition::LockedUnder2Min) {
            &self.short
        } else if holds(Condition::LockedOver8h) {
            &self.long
        } else {
            &self.normal
        }
    }
}

/// Host status for the corner chips, already formatted by the edge.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct HostChips {
    pub battery: Option<String>,
    pub network: Option<String>,
}

#[derive(Debug)]
pub struct Event {
    pub now: Now,
    pub kind: EventKind,
}

#[derive(Debug)]
pub enum EventKind {
    /// The compositor confirmed the lock.
    Locked,
    /// The compositor ended the lock: refused before `Locked`, ended after it.
    Finished,
    /// The full current set; repeats are harmless.
    Outputs(Vec<Output>),
    /// The answer to `Effect::ProbeShellOverlay`: whether a live shell holds a parked
    /// overlay over the lock and will draw the unlock opening. Ignored outside an unlock.
    ShellOverlay(bool),
    /// The pointer entered or moved on that output's lock surface.
    Pointer(OutputId),
    Key(OutputId, Key),
    Worker(WorkerId, Report),
    /// `next_deadline` passed. A spurious one is a no-op.
    Timer,
    /// The machine woke from sleep.
    Resumed,
    Chips(HostChips),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Backspace,
    Enter,
    Escape,
    ClearLine,
}

impl fmt::Debug for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Key::Char(_) => f.write_str("Char(<redacted>)"),
            Key::Backspace => f.write_str("Backspace"),
            Key::Enter => f.write_str("Enter"),
            Key::Escape => f.write_str("Escape"),
            Key::ClearLine => f.write_str("ClearLine"),
        }
    }
}

/// What the edge must do. `Password`'s Debug is redacted, so this one is too.
#[derive(Debug)]
pub enum Effect {
    /// Start a PAM worker. Spawning the fingerprint worker arms the sensor.
    Spawn(WorkerId),
    /// Kill a worker's process tree; a no-op for one already gone.
    Kill(WorkerId),
    /// Answer the password worker of generation `to`. No other worker can be addressed.
    Answer {
        to: u32,
        secret: Password,
    },
    DisplayOff,
    SampleChips,
    /// Ask the host whether a live shell has a parked overlay, and answer with
    /// `EventKind::ShellOverlay`. Issued once, when authentication succeeds.
    ProbeShellOverlay,
    CloseShellSurfaces,
    Unlock,
    Exit(ExitReason),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExitReason {
    Unlocked,
    /// `Finished` before `Locked`: another locker holds the session.
    Refused,
    /// `Finished` after `Locked`.
    Ended,
}

const DIAL_SHRINK: Duration = Duration::from_millis(200);
const SHAKE: Duration = Duration::from_millis(400);
const RING_FLASH: Duration = Duration::from_millis(300);
const RING_FILL: Duration = Duration::from_millis(300);
/// The unlock timeline is `session::opening`'s table; the locker only reads it.
pub(super) const GREETING_IN: Duration = opening::GREETING_IN;
pub(super) const GREETING_OUT: Duration = opening::GREETING_OUT;
pub(super) const OPENING: Duration = opening::OPENING;
pub(super) const RELEASE: Duration = opening::RELEASE;

/// The greeting's length for an unlock whose opening the shell does (`shell`) or the locker does.
pub(super) const fn greeting(shell: bool) -> Duration {
    GREETING_IN
        .saturating_add(opening::greeting_hold(shell))
        .saturating_add(GREETING_OUT)
}
/// The ring, date, hint and chips fade in over this after `Locked`, on the dial
/// the shell's closing left, which they are not part of.
pub(super) const REVEAL: Duration = Duration::from_millis(250);
pub(super) const FAREWELL_IN: Duration = Duration::from_millis(150);
pub(super) const FAREWELL_HOLD: Duration = Duration::from_millis(600);
pub(super) const FAREWELL_OUT: Duration = Duration::from_millis(150);
pub(super) const FAREWELL: Duration = FAREWELL_IN
    .saturating_add(FAREWELL_HOLD)
    .saturating_add(FAREWELL_OUT);

pub struct LockModel {
    pub(super) settings: Settings,
    pub(super) outputs: BTreeMap<OutputId, Size<Pixels>>,
    /// The last output with pointer or key input.
    active: Option<OutputId>,
    pub(super) stage: Stage,
    /// The one short animation in flight.
    pub(super) motion: Option<Motion>,
    pub(super) chips: HostChips,
    /// The latest event time, for the next wall-clock minute.
    clock: Now,
    sampled_minute: i64,
    next_generation: u32,
}

pub(super) enum Stage {
    /// Lock requested, compositor not yet confirmed. No workers run, so nothing
    /// can authenticate before the session is really locked.
    Requested {
        typed: Typed,
    },
    Locked(Locked),
    /// Authenticated: irreversible. Both workers are already dead.
    Unlocking(Unlocking),
    Exiting,
}

pub(super) struct Locked {
    pub(super) since_unix: i64,
    pub(super) password: PasswordFlow,
    pub(super) fingerprint: Fingerprint,
    pub(super) power: Power,
    /// The last password was rejected and nothing newer has been said: no key
    /// since, and no fingerprint message since.
    pub(super) rejected: bool,
    /// The shell's farewell, until it has played or a key skipped it.
    pub(super) farewell: Option<Farewell>,
    /// The `Locked` event, while the dial's parts fade in. `None`: they are there from the start.
    pub(super) reveal_from: Option<Instant>,
}

pub(super) struct Farewell {
    pub(super) text: String,
    /// When it starts fading in: after the dial's parts have, so it may lie ahead.
    pub(super) since: Instant,
}

pub(super) enum Typed {
    Editing(Password),
    /// Enter came before the prompt; the prompt answers it.
    Submitted(Password),
}

pub(super) enum PasswordFlow {
    /// Spawned; faillock's preauth runs before any prompt.
    Starting { worker: u32, typed: Typed },
    /// Blocked in the conversation; Enter answers at once.
    Prompting { worker: u32, typed: Password },
    /// Answer sent. Keys are ignored until the verdict.
    Checking { worker: u32, dots: usize },
    /// Password pause. Its worker, if any, only delivers the remaining time.
    Paused(PauseEnd),
    /// The worker died without a verdict; Enter starts a new one.
    Broken { typed: Password },
}

pub(super) enum PauseEnd {
    /// faillock said the account is locked; its next notice gives the time left.
    Unknown {
        worker: u32,
    },
    At(Instant),
}

pub(super) enum Fingerprint {
    /// Spawned, no PAM message yet.
    Starting { worker: u32 },
    /// The sensor is armed. `hint` is the latest error message from PAM.
    Listening { worker: u32, hint: Option<String> },
    /// Out of tries or failed; the next activity re-arms it.
    Exhausted,
    /// Released because the display is off; the next activity re-arms it.
    Disarmed,
    /// No reader or no enrolled print. Never retried this lock.
    Unavailable,
}

#[derive(Clone, Copy)]
pub(super) struct Power {
    pub(super) phase: PowerPhase,
    pub(super) since: Instant,
}

pub(super) struct Unlocking {
    pub(super) since_unix: i64,
    pub(super) phase: UnlockPhase,
    /// A live shell holds a parked overlay and draws the opening: the locker leaves
    /// the closed disc and clock and unlocks as soon as the rest has faded. False
    /// until the host says so, so a shell that is absent or stale means the locker's own opening.
    pub(super) shell_opens: bool,
}

pub(super) enum UnlockPhase {
    Greeting { since: Instant, text: String },
    /// The locker's own opening, back to the shell's frame.
    Opening { since: Instant },
    /// No greeting (or it has been skipped) and the shell opens: the ring, date,
    /// hint and chips fade out over `RELEASE`, leaving the closed disc.
    Release { since: Instant },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct Motion {
    pub(super) kind: MotionKind,
    pub(super) since: Instant,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum MotionKind {
    DialShrink,
    Shake,
    RingFlash,
    RingFill,
}

impl MotionKind {
    fn duration(self) -> Duration {
        match self {
            MotionKind::DialShrink => DIAL_SHRINK,
            MotionKind::Shake => SHAKE,
            MotionKind::RingFlash => RING_FLASH,
            MotionKind::RingFill => RING_FILL,
        }
    }
}

impl Motion {
    pub(super) fn end(self) -> Instant {
        self.since + self.kind.duration()
    }

    /// Progress in [0, 1), or `None` once it has ended.
    pub(super) fn progress(self, now: Instant) -> Option<f32> {
        (now < self.end()).then(|| {
            now.saturating_duration_since(self.since).as_secs_f32()
                / self.kind.duration().as_secs_f32()
        })
    }
}

impl Typed {
    pub(super) fn password(&self) -> &Password {
        match self {
            Typed::Editing(password) | Typed::Submitted(password) => password,
        }
    }

    fn into_password(self) -> Password {
        match self {
            Typed::Editing(password) | Typed::Submitted(password) => password,
        }
    }
}

impl PasswordFlow {
    fn worker(&self) -> Option<u32> {
        match self {
            PasswordFlow::Starting { worker, .. }
            | PasswordFlow::Prompting { worker, .. }
            | PasswordFlow::Checking { worker, .. }
            | PasswordFlow::Paused(PauseEnd::Unknown { worker }) => Some(*worker),
            PasswordFlow::Paused(PauseEnd::At(_)) | PasswordFlow::Broken { .. } => None,
        }
    }
}

impl Fingerprint {
    fn worker(&self) -> Option<u32> {
        match self {
            Fingerprint::Starting { worker } | Fingerprint::Listening { worker, .. } => {
                Some(*worker)
            }
            Fingerprint::Exhausted | Fingerprint::Disarmed | Fingerprint::Unavailable => None,
        }
    }

    pub(super) fn armed(&self) -> bool {
        self.worker().is_some()
    }
}

impl Power {
    fn deadline(self) -> Option<Instant> {
        rules(self.phase).next.map(|(after, _)| self.since + after)
    }
}

fn password(generation: u32) -> WorkerId {
    WorkerId {
        method: Method::Password,
        generation,
    }
}

fn fingerprint(generation: u32) -> WorkerId {
    WorkerId {
        method: Method::Fingerprint,
        generation,
    }
}

/// Whole minutes left in a password pause, rounded up, as the pill shows them.
pub(super) fn minutes_left(until: Instant, now: Instant) -> u32 {
    until
        .saturating_duration_since(now)
        .as_millis()
        .div_ceil(60_000) as u32
}

/// When `minutes_left` next drops, if that is before the pause ends.
fn countdown_change(until: Instant, now: Instant) -> Option<Instant> {
    let shown = minutes_left(until, now);
    (shown > 1).then(|| until - Duration::from_secs(60 * u64::from(shown - 1)))
}

/// Worker generations are unique across both methods, so a stale report never matches.
fn issue(next: &mut u32) -> u32 {
    *next += 1;
    *next - 1
}

/// Applies an editing key. True for an Enter on a non-empty password.
fn edit(password: &mut Password, key: Key) -> bool {
    match key {
        Key::Char(c) => {
            password.push(c);
        }
        Key::Backspace => password.pop(),
        Key::Escape | Key::ClearLine => *password = Password::new(),
        Key::Enter => return password.char_count() > 0,
    }
    false
}

fn edit_typed(typed: &mut Typed, key: Key) {
    match typed {
        Typed::Editing(password) => {
            if edit(password, key) {
                *typed = Typed::Submitted(mem::take(password));
            }
        }
        Typed::Submitted(_) => {
            if matches!(key, Key::Escape | Key::ClearLine) {
                *typed = Typed::Editing(Password::new());
            }
        }
    }
}

impl LockModel {
    /// Lock requested, nothing confirmed yet.
    pub fn new(settings: Settings, now: Now, outputs: Vec<Output>) -> (Self, Vec<Effect>) {
        let model = LockModel {
            settings,
            outputs: outputs.into_iter().map(|o| (o.id, o.size)).collect(),
            active: None,
            stage: Stage::Requested {
                typed: Typed::Editing(Password::new()),
            },
            motion: None,
            chips: HostChips::default(),
            clock: now,
            sampled_minute: now.unix.div_euclid(60),
            next_generation: 1,
        };
        (model, vec![Effect::CloseShellSurfaces, Effect::SampleChips])
    }

    /// The single transition function. Every (stage, event) pair is defined.
    pub fn update(&mut self, Event { now, kind }: Event) -> Vec<Effect> {
        let mut fx = Vec::new();
        self.clock = now;
        // Activity resets the power timeline before anything overdue runs, so a
        // late timer can never turn the display off under the key that woke it.
        if matches!(
            kind,
            EventKind::Pointer(_) | EventKind::Key(..) | EventKind::Resumed
        ) && let Stage::Locked(locked) = &mut self.stage
        {
            locked.power = Power {
                phase: PowerPhase::Active,
                since: now.mono,
            };
        }
        self.advance(now.mono, &mut fx);
        match kind {
            EventKind::Locked => self.locked(now, &mut fx),
            EventKind::Finished => self.finished(&mut fx),
            EventKind::Outputs(set) => {
                self.outputs = set.into_iter().map(|o| (o.id, o.size)).collect();
            }
            EventKind::Pointer(output) => {
                self.active = Some(output);
                self.activity(now.mono, &mut fx);
            }
            EventKind::Key(output, key) => {
                self.active = Some(output);
                self.skip_farewell();
                self.activity(now.mono, &mut fx);
                self.key(key, now.mono, &mut fx);
            }
            EventKind::Worker(id, report) => self.report(id, report, now, &mut fx),
            EventKind::Timer => {}
            EventKind::Resumed => self.activity(now.mono, &mut fx),
            EventKind::Chips(chips) => self.chips = chips,
            EventKind::ShellOverlay(parked) => self.shell_overlay(parked),
        }
        self.sample_minute(now, &mut fx);
        fx
    }

    /// The earliest moment the model needs a `Timer`. `None`: sleep until input.
    pub fn next_deadline(&self) -> Option<Instant> {
        let minute =
            self.clock.mono + Duration::from_secs((60 - self.clock.unix.rem_euclid(60)) as u64);
        let motion = self.motion.map(Motion::end);
        let stage = match &self.stage {
            Stage::Requested { .. } => [Some(minute), None, None, None, None],
            Stage::Locked(locked) => {
                let rules = rules(locked.power.phase);
                let pause = match locked.password {
                    PasswordFlow::Paused(PauseEnd::At(until)) => Some(until),
                    _ => None,
                };
                // The countdown shows whole minutes, so it changes on its own minute, not the wall clock's.
                let countdown = pause
                    .filter(|_| rules.display == Display::On)
                    .and_then(|until| countdown_change(until, self.clock.mono));
                [
                    locked.power.deadline(),
                    pause,
                    countdown,
                    rules.minute_tick.then_some(minute),
                    locked.farewell.as_ref().map(|f| f.since + FAREWELL),
                ]
            }
            Stage::Unlocking(unlocking) => [
                Some(match unlocking.phase {
                    UnlockPhase::Greeting { since, .. } => {
                        since + greeting(unlocking.shell_opens)
                    }
                    UnlockPhase::Opening { since } => since + OPENING,
                    UnlockPhase::Release { since } => since + RELEASE,
                }),
                None,
                None,
                None,
                None,
            ],
            Stage::Exiting => return None,
        };
        stage.into_iter().chain([motion]).flatten().min()
    }

    /// The size the model lays `output` out for.
    pub fn output_size(&self, output: OutputId) -> Option<Size<Pixels>> {
        self.outputs.get(&output).copied()
    }

    /// The outputs that should have a lock window.
    pub fn outputs(&self) -> impl Iterator<Item = OutputId> + '_ {
        self.outputs.keys().copied()
    }

    /// The output that takes keyboard input: the last one with input, else the first.
    pub(super) fn active_output(&self) -> Option<OutputId> {
        self.active
            .filter(|id| self.outputs.contains_key(id))
            .or_else(|| self.outputs.keys().next().copied())
    }

    fn start(&mut self, kind: MotionKind, now: Instant) {
        self.motion = Some(Motion { kind, since: now });
    }

    fn locked(&mut self, now: Now, fx: &mut Vec<Effect>) {
        let Stage::Requested { typed } = mem::replace(&mut self.stage, Stage::Exiting) else {
            return;
        };
        let pw = issue(&mut self.next_generation);
        let fp = issue(&mut self.next_generation);
        fx.extend([Effect::Spawn(password(pw)), Effect::Spawn(fingerprint(fp))]);
        let reveals = self.reveals();
        let reveal_from = reveals.then_some(now.mono);
        let farewell_from = if reveals { now.mono + REVEAL } else { now.mono };
        self.stage = Stage::Locked(Locked {
            since_unix: self.settings.lock_age_origin.unwrap_or(now.unix),
            password: PasswordFlow::Starting { worker: pw, typed },
            fingerprint: Fingerprint::Starting { worker: fp },
            power: Power {
                phase: PowerPhase::Active,
                since: now.mono,
            },
            rejected: false,
            farewell: self.settings.farewell.take().map(|text| Farewell {
                text,
                since: farewell_from,
            }),
            reveal_from,
        });
    }

    /// Whether the dial's parts fade in. A restarted locker has no closing to
    /// follow and reduced motion has no fades: both show them at once.
    pub(super) fn reveals(&self) -> bool {
        !self.settings.reduced_motion && self.settings.lock_age_origin.is_none()
    }

    fn finished(&mut self, fx: &mut Vec<Effect>) {
        let reason = match mem::replace(&mut self.stage, Stage::Exiting) {
            Stage::Requested { .. } => ExitReason::Refused,
            Stage::Locked(locked) => {
                fx.extend(locked.password.worker().map(|g| Effect::Kill(password(g))));
                fx.extend(
                    locked
                        .fingerprint
                        .worker()
                        .map(|g| Effect::Kill(fingerprint(g))),
                );
                ExitReason::Ended
            }
            Stage::Unlocking(_) => ExitReason::Ended,
            Stage::Exiting => return,
        };
        fx.push(Effect::Exit(reason));
    }

    fn sample_minute(&mut self, now: Now, fx: &mut Vec<Effect>) {
        let ticking = match &self.stage {
            Stage::Requested { .. } => true,
            Stage::Locked(locked) => rules(locked.power.phase).minute_tick,
            Stage::Unlocking(_) | Stage::Exiting => false,
        };
        let minute = now.unix.div_euclid(60);
        if ticking && minute != self.sampled_minute {
            self.sampled_minute = minute;
            fx.push(Effect::SampleChips);
        }
    }

    /// Everything due by `now`. Phases end at their deadline, not at `now`, so a late timer catches up exactly.
    fn advance(&mut self, now: Instant, fx: &mut Vec<Effect>) {
        if self.motion.is_some_and(|motion| motion.end() <= now) {
            self.motion = None;
        }
        match &mut self.stage {
            Stage::Locked(locked) => {
                if locked
                    .farewell
                    .as_ref()
                    .is_some_and(|f| f.since + FAREWELL <= now)
                {
                    locked.farewell = None;
                }
                while let Some((after, next)) = rules(locked.power.phase).next {
                    let due = locked.power.since + after;
                    if due > now {
                        break;
                    }
                    locked.power = Power {
                        phase: next,
                        since: due,
                    };
                    if rules(next).display == Display::Off {
                        fx.push(Effect::DisplayOff);
                        self.motion = None;
                    }
                    if rules(next).sensor == Sensor::Disarmed
                        && let Some(g) = locked.fingerprint.worker()
                    {
                        fx.push(Effect::Kill(fingerprint(g)));
                        locked.fingerprint = Fingerprint::Disarmed;
                    }
                }
                if let PasswordFlow::Paused(PauseEnd::At(until)) = locked.password
                    && until <= now
                {
                    let g = issue(&mut self.next_generation);
                    fx.push(Effect::Spawn(password(g)));
                    locked.password = PasswordFlow::Starting {
                        worker: g,
                        typed: Typed::Editing(Password::new()),
                    };
                }
            }
            Stage::Unlocking(unlocking) => {
                let greeting_end = match unlocking.phase {
                    UnlockPhase::Greeting { since, .. } => Some(since + greeting(unlocking.shell_opens)),
                    _ => None,
                };
                // The shell's opening starts as the locker lets go, so the greeting ends in the release.
                if let Some(end) = greeting_end.filter(|end| *end <= now) {
                    if unlocking.shell_opens {
                        fx.extend([Effect::Unlock, Effect::Exit(ExitReason::Unlocked)]);
                        self.stage = Stage::Exiting;
                        return;
                    }
                    unlocking.phase = UnlockPhase::Opening { since: end };
                }
                let done = match unlocking.phase {
                    UnlockPhase::Opening { since } => since + OPENING <= now,
                    UnlockPhase::Release { since } => since + RELEASE <= now,
                    UnlockPhase::Greeting { .. } => false,
                };
                if done {
                    fx.extend([Effect::Unlock, Effect::Exit(ExitReason::Unlocked)]);
                    self.stage = Stage::Exiting;
                }
            }
            Stage::Requested { .. } | Stage::Exiting => {}
        }
    }

    /// Any key skips straight to the resting hint, also one pressed before the lock.
    fn skip_farewell(&mut self) {
        self.settings.farewell = None;
        if let Stage::Locked(locked) = &mut self.stage {
            locked.farewell = None;
        }
    }

    /// A key, a pointer move or a resume, after `update` reset the power timeline.
    fn activity(&mut self, now: Instant, fx: &mut Vec<Effect>) {
        match &mut self.stage {
            Stage::Locked(locked) => {
                if matches!(
                    locked.fingerprint,
                    Fingerprint::Disarmed | Fingerprint::Exhausted
                ) {
                    let g = issue(&mut self.next_generation);
                    fx.push(Effect::Spawn(fingerprint(g)));
                    locked.fingerprint = Fingerprint::Starting { worker: g };
                }
            }
            Stage::Unlocking(unlocking) => {
                if let UnlockPhase::Greeting { .. } = unlocking.phase {
                    unlocking.phase = if unlocking.shell_opens {
                        UnlockPhase::Release { since: now }
                    } else {
                        UnlockPhase::Opening { since: now }
                    };
                }
            }
            Stage::Requested { .. } | Stage::Exiting => {}
        }
    }

    fn key(&mut self, key: Key, now: Instant, fx: &mut Vec<Effect>) {
        let shown_before = self.typed_dots();
        if let Stage::Locked(locked) = &mut self.stage {
            locked.rejected = false;
        }
        match &mut self.stage {
            Stage::Requested { typed } => edit_typed(typed, key),
            Stage::Locked(locked) => match &mut locked.password {
                PasswordFlow::Starting { typed, .. } => edit_typed(typed, key),
                PasswordFlow::Prompting { worker, typed } => {
                    if edit(typed, key) {
                        let (worker, secret) = (*worker, mem::take(typed));
                        let dots = secret.char_count();
                        fx.push(Effect::Answer { to: worker, secret });
                        locked.password = PasswordFlow::Checking { worker, dots };
                    }
                }
                PasswordFlow::Broken { typed } => {
                    if edit(typed, key) {
                        let typed = Typed::Submitted(mem::take(typed));
                        let g = issue(&mut self.next_generation);
                        fx.push(Effect::Spawn(password(g)));
                        locked.password = PasswordFlow::Starting { worker: g, typed };
                    }
                }
                PasswordFlow::Checking { .. } | PasswordFlow::Paused(_) => {}
            },
            Stage::Unlocking(_) | Stage::Exiting => {}
        }
        if shown_before == 0 && self.typed_dots() > 0 && !self.settings.reduced_motion {
            self.start(MotionKind::DialShrink, now);
        }
    }

    /// Characters in the shared buffer, whatever state holds it.
    pub(super) fn typed_dots(&self) -> usize {
        match &self.stage {
            Stage::Requested { typed }
            | Stage::Locked(Locked {
                password: PasswordFlow::Starting { typed, .. },
                ..
            }) => typed.password().char_count(),
            Stage::Locked(Locked {
                password: PasswordFlow::Prompting { typed, .. } | PasswordFlow::Broken { typed },
                ..
            }) => typed.char_count(),
            Stage::Locked(_) | Stage::Unlocking(_) | Stage::Exiting => 0,
        }
    }

    fn report(&mut self, id: WorkerId, report: Report, now: Now, fx: &mut Vec<Effect>) {
        let Stage::Locked(locked) = &self.stage else {
            return;
        };
        let current = match id.method {
            Method::Password => locked.password.worker(),
            Method::Fingerprint => locked.fingerprint.worker(),
        };
        if current != Some(id.generation) {
            return;
        }
        if report == Report::Finished(PamOutcome::Success) {
            return self.unlock(id.method, now, fx);
        }
        match id.method {
            Method::Password => self.password_report(id.generation, report, now.mono, fx),
            Method::Fingerprint => self.fingerprint_report(id.generation, report, now.mono, fx),
        }
    }

    fn password_report(&mut self, worker: u32, report: Report, now: Instant, fx: &mut Vec<Effect>) {
        let Stage::Locked(locked) = &mut self.stage else {
            return;
        };
        let flow = &mut locked.password;
        match report {
            Report::Prompt { .. } => match flow {
                PasswordFlow::Starting { typed, .. } => {
                    *flow = match mem::replace(typed, Typed::Editing(Password::new())) {
                        Typed::Submitted(secret) => {
                            let dots = secret.char_count();
                            fx.push(Effect::Answer { to: worker, secret });
                            PasswordFlow::Checking { worker, dots }
                        }
                        Typed::Editing(typed) => PasswordFlow::Prompting { worker, typed },
                    };
                }
                // The stack asked again after an answer.
                PasswordFlow::Checking { .. } => {
                    *flow = PasswordFlow::Prompting {
                        worker,
                        typed: Password::new(),
                    };
                }
                PasswordFlow::Prompting { .. }
                | PasswordFlow::Paused(_)
                | PasswordFlow::Broken { .. } => {}
            },
            Report::Notice(Notice::Hint { .. }) => {}
            Report::Notice(Notice::PasswordLocked) => {
                *flow = PasswordFlow::Paused(PauseEnd::Unknown { worker });
            }
            Report::Notice(Notice::PasswordPaused { remaining }) => {
                fx.push(Effect::Kill(password(worker)));
                *flow = PasswordFlow::Paused(PauseEnd::At(now + remaining));
            }
            Report::Finished(PamOutcome::Rejected | PamOutcome::MaxTries)
                if matches!(flow, PasswordFlow::Checking { .. }) =>
            {
                let g = issue(&mut self.next_generation);
                fx.push(Effect::Spawn(password(g)));
                *flow = PasswordFlow::Starting {
                    worker: g,
                    typed: Typed::Editing(Password::new()),
                };
                locked.rejected = true;
                self.motion = Some(Motion {
                    kind: MotionKind::Shake,
                    since: now,
                });
            }
            Report::Finished(_) | Report::Crashed => {
                let typed = match mem::replace(flow, PasswordFlow::Paused(PauseEnd::At(now))) {
                    PasswordFlow::Starting { typed, .. } => typed.into_password(),
                    PasswordFlow::Prompting { typed, .. } | PasswordFlow::Broken { typed } => typed,
                    PasswordFlow::Checking { .. } | PasswordFlow::Paused(_) => Password::new(),
                };
                *flow = PasswordFlow::Broken { typed };
            }
        }
    }

    fn fingerprint_report(
        &mut self,
        worker: u32,
        report: Report,
        now: Instant,
        fx: &mut Vec<Effect>,
    ) {
        let Stage::Locked(locked) = &mut self.stage else {
            return;
        };
        let sensor = &mut locked.fingerprint;
        match report {
            // A fingerprint stack that prompts is misconfigured. Nothing is ever sent to it.
            Report::Prompt { .. } => {
                fx.push(Effect::Kill(fingerprint(worker)));
                *sensor = Fingerprint::Unavailable;
            }
            Report::Notice(Notice::Hint { text, error }) => {
                let hint = match mem::replace(sensor, Fingerprint::Unavailable) {
                    Fingerprint::Listening { hint, .. } => hint,
                    _ => None,
                };
                let hint = if error { Some(text) } else { hint };
                *sensor = Fingerprint::Listening { worker, hint };
                if error {
                    locked.rejected = false;
                    self.motion = Some(Motion {
                        kind: MotionKind::RingFlash,
                        since: now,
                    });
                }
            }
            Report::Notice(Notice::PasswordLocked | Notice::PasswordPaused { .. }) => {}
            Report::Finished(PamOutcome::Unavailable) => {
                *sensor = match sensor {
                    Fingerprint::Listening { .. }
                        if rules(locked.power.phase).sensor == Sensor::Armed =>
                    {
                        let g = issue(&mut self.next_generation);
                        fx.push(Effect::Spawn(fingerprint(g)));
                        Fingerprint::Starting { worker: g }
                    }
                    Fingerprint::Listening { .. } => Fingerprint::Disarmed,
                    _ => Fingerprint::Unavailable,
                };
            }
            Report::Finished(_) | Report::Crashed => *sensor = Fingerprint::Exhausted,
        }
    }

    /// The host's answer about the shell's parked overlay. Only an unlock in progress
    /// cares: with no greeting the sequence is already in its opening, which the shell takes over.
    fn shell_overlay(&mut self, parked: bool) {
        let Stage::Unlocking(unlocking) = &mut self.stage else {
            return;
        };
        unlocking.shell_opens = parked;
        if parked && let UnlockPhase::Opening { since } = unlocking.phase {
            // The ring is still filling when there was no greeting to wait out.
            unlocking.phase = UnlockPhase::Release {
                since: since + RING_FILL,
            };
        }
    }

    /// Authenticated by `by`: kill the other worker and start the unlock sequence.
    fn unlock(&mut self, by: Method, now: Now, fx: &mut Vec<Effect>) {
        let Stage::Locked(locked) = mem::replace(&mut self.stage, Stage::Exiting) else {
            return;
        };
        match by {
            Method::Password => fx.extend(
                locked
                    .fingerprint
                    .worker()
                    .map(|g| Effect::Kill(fingerprint(g))),
            ),
            Method::Fingerprint => {
                fx.extend(locked.password.worker().map(|g| Effect::Kill(password(g))))
            }
        }
        self.start(MotionKind::RingFill, now.mono);
        let locked_for =
            Duration::from_secs(now.unix.saturating_sub(locked.since_unix).max(0) as u64);
        let phase = match &self.settings.greeting {
            Some(set) => UnlockPhase::Greeting {
                since: now.mono,
                text: set.pick(locked_for).to_owned(),
            },
            None => UnlockPhase::Opening { since: now.mono },
        };
        self.stage = Stage::Unlocking(Unlocking {
            since_unix: locked.since_unix,
            phase,
            shell_opens: false,
        });
        fx.push(Effect::ProbeShellOverlay);
    }
}
