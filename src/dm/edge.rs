//! What `dojo-greeter` does with the greeter socket, with no sockets and no windows in it:
//! the lock model's effects become protocol requests, the display manager's events become
//! the model's events, and a user's snapshot becomes the model's `UserLook`.
//!
//! The edge owns the I/O. [`Bridge`] only decides, so every path (a login the display manager
//! cannot hold, a reply for a user who has since changed, a session that fails to start) is
//! a plain test.

use std::collections::BTreeSet;

use super::account::{MAX_UID, MIN_UID};
use super::protocol::{
    Event, Login, PowerOccasion, Request, Secret, SessionEntry, SessionFailure,
};
use crate::lock::pam::{Method, PamOutcome, Report, WorkerId};
use crate::lock::{
    Choice, Effect, EventKind, FarewellSet, GreetingSet, Leave, UserLook, Username,
};
use crate::session::message;
use crate::session::snapshot;
use crate::session::Occasion;

/// What the display manager said about the user the screen asked for.
#[derive(Clone, PartialEq, Debug)]
pub struct Answer {
    /// The name as typed. It is not necessarily a name the display manager can hold.
    pub login: String,
    pub display_name: Option<String>,
    pub fingerprint: bool,
}

/// One thing the edge must do for an effect or an event.
#[derive(PartialEq, Debug)]
pub enum Step {
    Send(Request),
    /// An event for the model, to run before anything is painted.
    Model(EventKind),
    /// Load this user's snapshot and answer with `user_look`.
    Look(Answer),
}

/// The display manager spoke out of turn. The greeter cannot trust the rest of the stream.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Violation {
    SecondInit,
}

#[derive(Debug)]
pub struct Bridge {
    sessions: Vec<SessionEntry>,
    /// Generations begun for a name the display manager cannot hold. They never leave the greeter:
    /// they prompt, and whatever is typed is refused, as the display manager would for any name
    /// it has no account for.
    local: BTreeSet<u32>,
    /// The user whose `User` reply the greeter is waiting for.
    asked: Option<String>,
}

impl Bridge {
    pub fn new(sessions: Vec<SessionEntry>) -> Bridge {
        Bridge {
            sessions,
            local: BTreeSet::new(),
            asked: None,
        }
    }

    /// The request that asks about the screen's first user. `SwapLook` asks about every later one.
    pub fn ask(&mut self, login: &Login) -> Request {
        self.asked = Some(login.as_str().to_owned());
        Request::SelectUser {
            login: login.clone(),
        }
    }

    /// The model's effect as requests and events. Effects that are the edge's own (the window,
    /// the chips, the monitors, leaving) produce nothing.
    pub fn effect(&mut self, effect: Effect, choice: Option<Choice<'_>>) -> Vec<Step> {
        match effect {
            Effect::Spawn(id) => self.begin(id, choice),
            Effect::Kill(id) => {
                if self.local.remove(&id.generation) {
                    vec![]
                } else {
                    vec![Step::Send(Request::Cancel {
                        generation: id.generation,
                    })]
                }
            }
            Effect::Answer { to, secret } => {
                if self.local.remove(&to) {
                    let id = WorkerId {
                        method: Method::Password,
                        generation: to,
                    };
                    return vec![Step::Model(EventKind::Worker(
                        id,
                        Report::Finished(PamOutcome::Rejected),
                    ))];
                }
                vec![Step::Send(Request::Answer {
                    generation: to,
                    secret: Secret::new(secret),
                })]
            }
            Effect::SwapLook(user) => self.swap(&user),
            Effect::Unlock { by } => vec![Step::Send(Request::StartSession {
                generation: by.generation,
            })],
            Effect::Power(leave) => vec![Step::Send(Request::Power {
                occasion: power_occasion(leave),
            })],
            Effect::DisplayOff
            | Effect::SampleChips
            | Effect::ProbeShellOverlay
            | Effect::CloseShellSurfaces
            | Effect::Exit(_) => vec![],
        }
    }

    fn begin(&mut self, id: WorkerId, choice: Option<Choice<'_>>) -> Vec<Step> {
        let held = choice.and_then(|choice| {
            Some((
                Login::parse(choice.login).ok()?,
                self.sessions.get(choice.index)?.id.clone(),
            ))
        });
        let Some((login, session)) = held else {
            self.local.insert(id.generation);
            let report = match id.method {
                Method::Password => Report::Prompt { echo: false },
                Method::Fingerprint => Report::Finished(PamOutcome::Unavailable),
            };
            return vec![Step::Model(EventKind::Worker(id, report))];
        };
        vec![Step::Send(Request::Begin {
            generation: id.generation,
            method: id.method,
            login,
            session,
        })]
    }

    fn swap(&mut self, user: &Username) -> Vec<Step> {
        match Login::parse(user.as_str()) {
            Ok(login) => vec![Step::Send(self.ask(&login))],
            Err(_) => {
                self.asked = None;
                vec![Step::Look(Answer {
                    login: user.as_str().to_owned(),
                    display_name: None,
                    fingerprint: false,
                })]
            }
        }
    }

    /// The display manager's event as the model's events. A `User` reply for anyone but the
    /// user last asked about is stale and produces nothing.
    pub fn event(&mut self, event: Event) -> Result<Vec<Step>, Violation> {
        Ok(match event {
            Event::Init { .. } => return Err(Violation::SecondInit),
            Event::User {
                login,
                display_name,
                fingerprint,
            } => {
                if self.asked.as_deref() != Some(login.as_str()) {
                    return Ok(vec![]);
                }
                self.asked = None;
                vec![Step::Look(Answer {
                    login: login.as_str().to_owned(),
                    display_name,
                    fingerprint,
                })]
            }
            Event::Report {
                generation,
                method,
                report,
            } => vec![Step::Model(EventKind::Worker(
                WorkerId { method, generation },
                report,
            ))],
            Event::SessionStarted => vec![Step::Model(EventKind::SessionStarted)],
            Event::SessionFailed { reason } => vec![Step::Model(EventKind::SessionFailed(
                failure_text(reason).to_owned(),
            ))],
        })
    }
}

fn power_occasion(leave: Leave) -> PowerOccasion {
    match leave {
        Leave::Suspend => PowerOccasion::Suspend,
        Leave::Hibernate => PowerOccasion::Hibernate,
        Leave::Restart => PowerOccasion::Restart,
        Leave::Shutdown => PowerOccasion::Shutdown,
    }
}

/// The hint the login screen shows when the session did not start.
pub fn failure_text(reason: SessionFailure) -> &'static str {
    match reason {
        SessionFailure::Refused => "That login was refused",
        SessionFailure::SpawnFailed => "The session could not be started",
        SessionFailure::UnknownSession => "That session is not available",
        SessionFailure::HelperCrashed => "The login stopped unexpectedly. Try again.",
        SessionFailure::Exited => "The session ended right away",
    }
}

/// The names for the session chip. Two sessions that share a name are told apart by their id,
/// so the chip never shows the same text for two choices.
pub fn session_names(sessions: &[SessionEntry]) -> Vec<String> {
    sessions
        .iter()
        .map(|entry| {
            let twins = sessions.iter().filter(|other| other.name == entry.name).count();
            if twins > 1 {
                format!("{} ({})", entry.name, entry.id.as_str())
            } else {
                entry.name.clone()
            }
        })
        .collect()
}

/// The first account a person could log in as, for a machine where nobody has logged in yet:
/// the first `passwd` line with a human uid and a shell that is not a refusal.
pub fn first_human(passwd: &str) -> Option<Login> {
    passwd.lines().find_map(|line| {
        let mut fields = line.split(':');
        let (name, _, uid, _, _, _, shell) = (
            fields.next()?,
            fields.next()?,
            fields.next()?.parse::<u32>().ok()?,
            fields.next()?,
            fields.next()?,
            fields.next()?,
            fields.next()?,
        );
        let refusal = shell.rsplit('/').next().is_some_and(|base| {
            matches!(base, "nologin" | "false" | "") || base.starts_with("nologin")
        });
        ((MIN_UID..MAX_UID).contains(&uid) && !refusal)
            .then(|| Login::parse(name).ok())
            .flatten()
    })
}

/// The theme to paint for a user: their snapshot's palette, else the default theme with the
/// default error colour. Never a home directory.
pub fn palette(snapshot: Option<&snapshot::Look>) -> (crate::config::ThemeColors, u32) {
    match snapshot.and_then(|look| look.palette.as_ref()) {
        Some(palette) => (palette.colors(), palette.error_color()),
        None => (
            crate::config::ThemeColors::default(),
            crate::session::host::FALLBACK_ERROR,
        ),
    }
}

/// What the model needs to know about `answer`'s user. `base` carries the clock, whether the
/// machine just booted and a seed; its `previous` is replaced by the user's own last greeting.
///
/// A user with no snapshot gets the neutral look: their typed name, no greeting and no farewell.
pub fn user_look(
    answer: &Answer,
    snapshot: Option<&snapshot::Look>,
    base: &message::Context,
) -> UserLook {
    let Some(snapshot) = snapshot else {
        return UserLook {
            login: answer.login.clone(),
            display: answer.display_name.clone(),
            greeting: None,
            farewells: None,
            fingerprint: answer.fingerprint,
        };
    };
    let settings = snapshot.settings_or_default();
    let name = snapshot.display_name();
    let context = message::Context {
        previous: snapshot
            .last_greeting
            .as_ref()
            .map(|line| line.as_str().to_owned()),
        ..base.clone()
    };
    UserLook {
        login: answer.login.clone(),
        display: answer
            .display_name
            .clone()
            .or_else(|| snapshot.full_name.as_ref().map(|name| name.as_str().to_owned())),
        greeting: settings
            .greeting
            .then(|| GreetingSet::choose_for(Occasion::Login, &context, &name)),
        farewells: settings.farewell.then(|| FarewellSet::choose(base, &name)),
        fingerprint: answer.fingerprint,
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod over_the_socket;
