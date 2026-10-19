//! Exactly what one output paints, derived from the model at an instant.
//! No timestamps, so equal scenes mean no frame.

use gpui::{Pixels, Size, px};

use super::login::SwapPhase;
use super::model::{
    FAREWELL, FAREWELL_HOLD, FAREWELL_IN, FAREWELL_OUT, Fingerprint, GREETING_IN, GREETING_OUT,
    LockModel, Locked, MotionKind, Now, OPENING, OutputId, PasswordFlow, PauseEnd, RELEASE, REVEAL,
    Stage, Typed, UnlockPhase, greeting, minutes_left,
};
use crate::session::opening::greeting_hold;
use super::power::{Display, PowerPhase, rules};
use crate::session::transition::{self, Pose, Style};
use crate::wall_clock;

#[derive(Clone, PartialEq, Debug)]
pub struct Scene {
    /// The session transition pose: the dial at rest, the shell frame when the opening ends.
    pub pose: Pose,
    /// The dial's time, at minute resolution.
    pub clock: String,
    /// The shell's top-bar label, while the pose shows any of the shell frame.
    pub shell_clock: Option<String>,
    /// Brightness; 1 is full and 0 is the display-off black.
    pub dim: f32,
    /// How loud the dial's time is: full on the active output, and on the others
    /// only once the dial's parts have faded in, so the closed state is the same everywhere.
    pub quiet: f32,
    pub role: Role,
    /// True only while a bounded motion or the unlock sequence runs.
    pub animating: bool,
    /// Which user's look to paint, on the login screen. `None` on the lock screen, which has one look.
    pub look: Option<LookMix>,
}

/// The login screen's look: `login`'s, or a cross-fade into it from `from`.
#[derive(Clone, PartialEq, Debug)]
pub struct LookMix {
    /// The account whose snapshot is the target of the paint.
    pub login: String,
    /// The account fading out, while `mix` is below 1.
    pub from: Option<String>,
    /// 0 paints `from` (or `login` alone when there is no `from`), 1 paints `login`.
    pub mix: f32,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Role {
    /// Not the active output: the dim clock only.
    Passive,
    Active(Panel),
}

#[derive(Clone, PartialEq, Debug)]
pub struct Panel {
    pub date: String,
    pub dial: Dial,
    pub pill: Option<Pill>,
    pub line: Option<Line>,
    pub chips: Chips,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Dial {
    /// 1 at rest; smaller while the pill is shown.
    pub scale: f32,
    pub ring: Ring,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Ring {
    pub tone: Tone,
    /// How much of the ring has filled clockwise, on success.
    pub fill: f32,
    /// Strength of a flash, fading from 1.
    pub flash: f32,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Pill {
    pub dots: usize,
    pub tone: Tone,
    /// Horizontal shake offset.
    pub offset_x: Pixels,
    /// Dashed outline: password pause.
    pub dashed: bool,
    /// Whole minutes left in a password pause, rounded up.
    pub countdown_minutes: Option<u32>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tone {
    Normal,
    Wrong,
    Muted,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Line {
    Hint(String),
    Greeting { text: String, alpha: f32 },
    /// The shell's farewell, over the resting dial where the hint goes.
    Farewell { text: String, alpha: f32 },
}

#[derive(Clone, PartialEq, Debug)]
pub struct Chips {
    pub user: String,
    pub battery: Option<String>,
    pub network: Option<String>,
    pub lock_age: Option<String>,
    /// The session chip and the username editor. `None` on the lock screen.
    pub login: Option<LoginChips>,
}

#[derive(Clone, PartialEq, Debug)]
pub struct LoginChips {
    /// The selected session's name.
    pub session: String,
    /// The user chip is being edited: it shows the text typed so far.
    pub editing: Option<NameEdit>,
}

#[derive(Clone, PartialEq, Debug)]
pub struct NameEdit {
    pub text: String,
    /// The last key or Enter was refused by the name rules.
    pub refused: bool,
}

/// The inactive outputs' clock: present but quiet.
const PASSIVE_CLOCK: f32 = 0.55;
const TYPING_SCALE: f32 = 0.72;
const SHAKE_PX: f32 = 12.;
const SHAKE_SWINGS: f32 = 3.;

const HINT_RESTING: &str = "Touch the sensor, or just start typing";
const HINT_TYPING: &str = "The sensor still works";
const HINT_WRONG: &str = "That wasn't it. Try again.";
const HINT_PASSWORD_ONLY: &str = "Start typing to unlock";
const HINT_LOGIN_PASSWORD: &str = "Type your password";
const HINT_PAUSED_SENSOR: &str = "Password paused. The sensor still works";
const HINT_PAUSED: &str = "Password paused";
const HINT_BROKEN: &str = "Press Enter to try again";

fn smoothstep(x: f32) -> f32 {
    let x = x.clamp(0., 1.);
    x * x * (3. - 2. * x)
}

/// Whole minutes elapsed since the lock, rounded down. It is shown on the next
/// frame, so a minute can pass before the minute tick redraws it.
fn lock_age(since: i64, now: i64) -> String {
    let minutes = (now - since).max(0) / 60;
    match minutes {
        0 => "Locked just now".into(),
        1..60 => format!("Locked {minutes} min"),
        _ => format!("Locked {} h {} min", minutes / 60, minutes % 60),
    }
}

fn greeting_alpha(elapsed: std::time::Duration, shell_opens: bool) -> f32 {
    let hold = greeting_hold(shell_opens);
    if elapsed < GREETING_IN {
        smoothstep(elapsed.as_secs_f32() / GREETING_IN.as_secs_f32())
    } else if elapsed < GREETING_IN + hold {
        1.
    } else {
        let out = elapsed - GREETING_IN - hold;
        1. - smoothstep(out.as_secs_f32() / GREETING_OUT.as_secs_f32())
    }
}

/// The farewell's alpha `elapsed` after `Locked`, or `None` once it is over.
/// Reduced motion shows it at full strength for the same span, with no fades.
fn farewell_alpha(elapsed: std::time::Duration, reduced_motion: bool) -> Option<f32> {
    if elapsed >= FAREWELL {
        return None;
    }
    Some(if reduced_motion {
        1.
    } else if elapsed < FAREWELL_IN {
        smoothstep(elapsed.as_secs_f32() / FAREWELL_IN.as_secs_f32())
    } else if elapsed < FAREWELL_IN + FAREWELL_HOLD {
        1.
    } else {
        1. - smoothstep((elapsed - FAREWELL_IN - FAREWELL_HOLD).as_secs_f32() / FAREWELL_OUT.as_secs_f32())
    })
}

impl LockModel {
    /// How much of the dial's ring, date, hint and chips shows: nothing on the
    /// closed state the shell's closing ends on, everything once they have faded in.
    fn reveal(&self, now: Now) -> f32 {
        match &self.stage {
            Stage::Requested { .. } if self.reveals() => 0.,
            Stage::Locked(Locked {
                reveal_from: Some(from),
                ..
            }) => smoothstep(
                now.mono.saturating_duration_since(*from).as_secs_f32() / REVEAL.as_secs_f32(),
            ),
            _ => 1.,
        }
    }

    /// How much of the dial's ring, date, hint and chips is left while the locker lets
    /// go for the shell's opening: 1 until the last `RELEASE` of the unlock sequence.
    /// Reduced motion has no fades, so the parts stay until the shell's cross-fade.
    fn letting_go(&self, now: Now) -> f32 {
        let Stage::Unlocking(unlocking) = &self.stage else {
            return 1.;
        };
        if !unlocking.shell_opens || self.settings.reduced_motion {
            return 1.;
        }
        let since = match &unlocking.phase {
            UnlockPhase::Greeting { since, .. } => *since + greeting(true) - RELEASE,
            UnlockPhase::Release { since } => *since,
            UnlockPhase::Opening { .. } => return 1.,
        };
        1. - smoothstep(now.mono.saturating_duration_since(since).as_secs_f32() / RELEASE.as_secs_f32())
    }

    /// The farewell showing at `now` as (text, alpha).
    fn farewell(&self, now: Now) -> Option<(&str, f32)> {
        let Stage::Locked(locked) = &self.stage else {
            return None;
        };
        let farewell = locked.farewell.as_ref()?;
        let elapsed = now.mono.saturating_duration_since(farewell.since);
        let alpha = farewell_alpha(elapsed, self.settings.reduced_motion)?;
        Some((&farewell.text, alpha))
    }

    /// What `output` shows at `now`. Pure. `None` for an unknown output.
    pub fn scene(&self, output: OutputId, now: Now) -> Option<Scene> {
        let viewport = *self.outputs.get(&output)?;
        let phase = match &self.stage {
            Stage::Locked(locked) => locked.power.phase,
            _ => PowerPhase::Active,
        };
        let power = rules(phase);
        let style = if self.settings.reduced_motion {
            Style::Crossfade
        } else {
            Style::Morph
        };
        if power.display == Display::Off {
            return Some(dark(viewport, style));
        }
        let progress = match &self.stage {
            Stage::Requested { .. } | Stage::Locked(_) => 1.,
            Stage::Unlocking(unlocking) => match unlocking.phase {
                UnlockPhase::Greeting { .. } | UnlockPhase::Release { .. } => 1.,
                UnlockPhase::Opening { since } => {
                    1. - transition::progress(now.mono.saturating_duration_since(since), OPENING)
                }
            },
            Stage::Exiting => 0.,
        };
        let motion = self
            .motion
            .and_then(|m| Some((m.kind, m.progress(now.mono)?)))
            .map(|(kind, p)| (kind, if self.settings.reduced_motion { 0. } else { p }));
        let unlocking = matches!(self.stage, Stage::Unlocking(_));
        let opening = progress > 0. && progress < 1.;
        let role = if self.active_output() == Some(output) {
            self.panel(now, motion).map_or(Role::Passive, Role::Active)
        } else {
            Role::Passive
        };
        // Passive outputs show only the clock, so only the opening moves them.
        let fading = self.farewell(now).is_some() && !self.settings.reduced_motion;
        let reveal = self.reveal(now);
        let revealing = reveal < 1. && matches!(self.stage, Stage::Locked(_));
        let look = self.look(now);
        let animating = opening
            || look.as_ref().is_some_and(|look| look.from.is_some())
            || matches!(role, Role::Active(_))
                && (unlocking
                    || fading
                    || revealing
                    || (motion.is_some() && !self.settings.reduced_motion));
        let mut pose = transition::layout(viewport, self.settings.controls_width, progress, style);
        // The ring, date, hint and chips go before the shell takes the closed disc over.
        let letting_go = self.letting_go(now);
        let parts = reveal.min(letting_go);
        if parts < 1.
            && let Pose::Moving(layout) = &mut pose
        {
            layout.ring = parts;
            layout.text = parts;
        }
        Some(Scene {
            pose,
            clock: wall_clock::dial_time(now.unix),
            shell_clock: (progress < 1.).then(|| wall_clock::top_bar_label(now.unix)),
            dim: power.dim,
            quiet: match role {
                Role::Active(_) => 1.,
                Role::Passive => 1. + (PASSIVE_CLOCK - 1.) * reveal,
            },
            role,
            animating,
            look,
        })
    }

    /// The look to paint on the login screen. The old one stays until the edge has the new one.
    fn look(&self, now: Now) -> Option<LookMix> {
        let login = self.login.as_ref()?;
        let to = |user: &str| LookMix {
            login: user.to_owned(),
            from: None,
            mix: 1.,
        };
        Some(match &login.swap {
            None => to(login.user.as_str()),
            Some(swap) => match swap.phase {
                SwapPhase::Loading => to(swap.from.as_str()),
                SwapPhase::Fading { since } => {
                    let elapsed = now.mono.saturating_duration_since(since);
                    if elapsed >= REVEAL {
                        to(login.user.as_str())
                    } else {
                        LookMix {
                            from: Some(swap.from.as_str().to_owned()),
                            mix: smoothstep(elapsed.as_secs_f32() / REVEAL.as_secs_f32()),
                            ..to(login.user.as_str())
                        }
                    }
                }
            },
        })
    }

    fn panel(&self, now: Now, motion: Option<(MotionKind, f32)>) -> Option<Panel> {
        let since_unix = match &self.stage {
            Stage::Requested { .. } => None,
            Stage::Locked(locked) => Some(locked.since_unix),
            Stage::Unlocking(unlocking) => Some(unlocking.since_unix),
            Stage::Exiting => return None,
        };
        let pill = self.pill(now, motion);
        let flash = |kind| match motion {
            Some((k, p)) if k == kind => Some(p),
            _ => None,
        };
        let armed = match &self.stage {
            Stage::Requested { .. } => self.fingerprint_wanted(),
            Stage::Locked(locked) => locked.fingerprint.armed(),
            Stage::Unlocking(_) | Stage::Exiting => false,
        };
        let ring = if unlocking(&self.stage) {
            Ring {
                tone: Tone::Normal,
                fill: flash(MotionKind::RingFill).map_or(1., |p| {
                    if self.settings.reduced_motion {
                        1.
                    } else {
                        smoothstep(p)
                    }
                }),
                flash: 0.,
            }
        } else if let Some(p) = flash(MotionKind::RingFlash).or(flash(MotionKind::Shake)) {
            Ring {
                tone: Tone::Wrong,
                fill: 0.,
                flash: 1. - p,
            }
        } else {
            Ring {
                // Without the fingerprint method the ring says nothing about a sensor.
                tone: if armed || !self.fingerprint_wanted() {
                    Tone::Normal
                } else {
                    Tone::Muted
                },
                fill: 0.,
                flash: 0.,
            }
        };
        let scale = match (pill.is_some(), flash(MotionKind::DialShrink)) {
            (false, _) => 1.,
            (true, Some(p)) => 1. + (TYPING_SCALE - 1.) * smoothstep(p),
            (true, None) => TYPING_SCALE,
        };
        Some(Panel {
            date: wall_clock::date_line(now.unix),
            dial: Dial { scale, ring },
            line: self.line(now, armed, pill.is_some()),
            pill,
            chips: Chips {
                user: self.settings.user.clone(),
                battery: self.chips.battery.clone(),
                network: self.chips.network.clone(),
                lock_age: since_unix
                    .filter(|_| self.login.is_none())
                    .map(|since| lock_age(since, now.unix)),
                login: self.login.as_ref().map(|login| LoginChips {
                    session: login.session().to_owned(),
                    editing: login.editing.as_ref().map(|buffer| NameEdit {
                        text: buffer.text().to_owned(),
                        refused: buffer.refused(),
                    }),
                }),
            },
        })
    }

    fn pill(&self, now: Now, motion: Option<(MotionKind, f32)>) -> Option<Pill> {
        let shake = match motion {
            Some((MotionKind::Shake, p)) => Some(p),
            _ => None,
        };
        let (tone, dashed, countdown_minutes) = match &self.stage {
            Stage::Requested {
                typed: Typed::Submitted(_),
            }
            | Stage::Locked(Locked {
                password:
                    PasswordFlow::Starting {
                        typed: Typed::Submitted(_),
                        ..
                    }
                    | PasswordFlow::Checking { .. },
                ..
            }) => (Tone::Muted, false, None),
            Stage::Locked(Locked {
                password: PasswordFlow::Paused(end),
                ..
            }) => (
                Tone::Muted,
                true,
                match end {
                    PauseEnd::At(until) => Some(minutes_left(*until, now.mono)),
                    PauseEnd::Unknown { .. } => None,
                },
            ),
            Stage::Unlocking(_) | Stage::Exiting => return None,
            _ => (Tone::Normal, false, None),
        };
        let dots = match &self.stage {
            Stage::Locked(Locked {
                password: PasswordFlow::Checking { dots, .. },
                ..
            }) => *dots,
            _ => self.typed_dots(),
        };
        if dots == 0 && tone == Tone::Normal && shake.is_none() {
            return None;
        }
        Some(Pill {
            dots,
            tone: if shake.is_some() { Tone::Wrong } else { tone },
            offset_x: px(shake.map_or(0., |p| {
                SHAKE_PX * (1. - p) * (p * SHAKE_SWINGS * std::f32::consts::TAU).sin()
            })),
            dashed,
            countdown_minutes,
        })
    }

    fn line(&self, now: Now, armed: bool, typing: bool) -> Option<Line> {
        let hint = |text: &str| Some(Line::Hint(text.into()));
        let locked = match &self.stage {
            Stage::Requested { .. } => None,
            Stage::Locked(locked) => Some(locked),
            Stage::Unlocking(unlocking) => {
                return match &unlocking.phase {
                    UnlockPhase::Greeting { since, text } => Some(Line::Greeting {
                        text: text.clone(),
                        alpha: greeting_alpha(
                            now.mono.saturating_duration_since(*since),
                            unlocking.shell_opens,
                        ),
                    }),
                    UnlockPhase::Opening { .. } | UnlockPhase::Release { .. } => None,
                };
            }
            Stage::Exiting => return None,
        };
        if let Some((text, alpha)) = self.farewell(now) {
            return Some(Line::Farewell {
                text: text.into(),
                alpha,
            });
        }
        if let Some(locked) = locked {
            match (&locked.password, &locked.fingerprint) {
                (PasswordFlow::Broken { .. }, _) => return hint(HINT_BROKEN),
                (PasswordFlow::Paused(_), _) if armed => return hint(HINT_PAUSED_SENSOR),
                (PasswordFlow::Paused(_), _) => return hint(HINT_PAUSED),
                _ if locked.rejected => return hint(HINT_WRONG),
                (
                    _,
                    Fingerprint::Listening {
                        hint: Some(text), ..
                    },
                ) => return hint(text),
                _ => {}
            }
        }
        match (armed, typing) {
            (true, true) => hint(HINT_TYPING),
            (true, false) => hint(HINT_RESTING),
            (false, _) if self.login.is_some() => hint(HINT_LOGIN_PASSWORD),
            (false, _) => hint(HINT_PASSWORD_ONLY),
        }
    }
}

fn unlocking(stage: &Stage) -> bool {
    matches!(stage, Stage::Unlocking(_))
}

/// The display is off: one fixed scene, so nothing that happens meanwhile paints a frame.
fn dark(viewport: Size<Pixels>, style: Style) -> Scene {
    Scene {
        pose: transition::layout(viewport, 0., 1., style),
        clock: String::new(),
        shell_clock: None,
        dim: rules(PowerPhase::DisplayOff).dim,
        quiet: 1.,
        role: Role::Passive,
        animating: false,
        look: None,
    }
}
