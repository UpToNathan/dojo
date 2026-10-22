//! The login attempts in flight, keyed by generation. A password attempt and a fingerprint
//! attempt run side by side, each its own [`Attempt`] and its own helper process, and the
//! first to succeed wins: every other attempt is killed. The registry is pure. It turns a
//! greeter request or a helper event into [`Action`]s, in order, and the service carries
//! them out.

use std::collections::BTreeMap;

use super::auth::{Attempt, Effect, Input, Phase, Rejected};
use super::launch::HelperEvent;
use super::protocol::{Event, Login, Request, Secret, SessionFailure, SessionId};
use crate::lock::pam::Method;

#[derive(PartialEq, Debug)]
pub enum Action {
    /// Start `dojo-dm-pam` for this method, send it the login, and watch its socket.
    Spawn {
        generation: u32,
        method: Method,
        login: Login,
    },
    Answer {
        generation: u32,
        secret: Secret,
    },
    Relay(Event),
    /// Resolve the id against the offered sessions. An id that resolves to nothing is
    /// the service's to refuse with [`Registry::refuse_start`].
    StartSession {
        generation: u32,
        session: SessionId,
    },
    /// Kill the helper and its tree, and stop watching its socket. Idempotent.
    Kill {
        generation: u32,
    },
}

struct Entry {
    attempt: Attempt,
    method: Method,
    login: Login,
}

/// The attempt that passed PAM and will be, or is, the session.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Winner<'a> {
    pub generation: u32,
    pub login: &'a Login,
}

#[derive(Default)]
pub struct Registry {
    entries: BTreeMap<u32, Entry>,
    winner: Option<u32>,
}

impl Registry {
    pub fn new() -> Registry {
        Registry::default()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn phase(&self, generation: u32) -> Option<Phase> {
        self.entries
            .get(&generation)
            .map(|entry| entry.attempt.phase())
    }

    pub fn winner(&self) -> Option<Winner<'_>> {
        let generation = self.winner?;
        Some(Winner {
            generation,
            login: &self.entries.get(&generation)?.login,
        })
    }

    /// A request the greeter sent. `Err` leaves everything as it was; the service
    /// ignores it, except that `Unrelated` requests are its own to answer.
    pub fn request(&mut self, request: Request) -> Result<Vec<Action>, Rejected> {
        let generation = match &request {
            Request::Begin { generation, .. }
            | Request::Answer { generation, .. }
            | Request::Cancel { generation }
            | Request::StartSession { generation } => *generation,
            Request::Hello { .. } | Request::SelectUser { .. } | Request::Power { .. } => {
                return Err(Rejected::Unrelated);
            }
        };
        let Request::Begin {
            method, ref login, ..
        } = request
        else {
            let entry = self.entries.get_mut(&generation).ok_or(Rejected::Stale)?;
            let effects = entry.attempt.reduce(Input::Greeter(request))?;
            return Ok(self.translate(generation, effects));
        };
        if self.entries.contains_key(&generation) || self.winner.is_some() {
            return Err(Rejected::OutOfOrder);
        }
        let login = login.clone();
        let mut attempt = Attempt::default();
        let effects = attempt.reduce(Input::Greeter(request))?;
        // A method has one attempt at a time: asking again replaces the old one.
        let replaced: Vec<u32> = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.method == method)
            .map(|(generation, _)| *generation)
            .collect();
        let mut actions = Vec::new();
        for old in replaced {
            self.entries.remove(&old);
            actions.push(Action::Kill { generation: old });
        }
        self.entries.insert(
            generation,
            Entry {
                attempt,
                method,
                login,
            },
        );
        actions.extend(self.translate(generation, effects));
        Ok(actions)
    }

    /// Something a helper said. Events from a helper that was already killed or finished
    /// are dropped; one the attempt cannot be in that state for kills the helper.
    pub fn helper(&mut self, generation: u32, event: HelperEvent) -> Vec<Action> {
        let Some(entry) = self.entries.get_mut(&generation) else {
            return Vec::new();
        };
        match entry.attempt.reduce(Input::Helper(event)) {
            Ok(effects) => self.translate(generation, effects),
            Err(_) => self.remove(generation).into_iter().collect(),
        }
    }

    /// The service could not resolve the session the greeter asked for.
    pub fn refuse_start(&mut self, generation: u32, reason: SessionFailure) -> Vec<Action> {
        if self.phase(generation) != Some(Phase::Starting) {
            return Vec::new();
        }
        let mut actions = vec![Action::Relay(Event::SessionFailed { reason })];
        actions.extend(self.remove(generation));
        actions
    }

    /// The greeter went away. Attempts still proving identity die with it; one that has
    /// been told to start, or is running, does not need it any more.
    pub fn greeter_gone(&mut self) -> Vec<Action> {
        let leaving: Vec<u32> = self
            .entries
            .iter()
            .filter(|(_, entry)| {
                !matches!(
                    entry.attempt.phase(),
                    Phase::Starting | Phase::SessionRunning
                )
            })
            .map(|(generation, _)| *generation)
            .collect();
        leaving
            .into_iter()
            .filter_map(|generation| self.remove(generation))
            .collect()
    }

    /// Everything, for a service that is stopping.
    pub fn shutdown(&mut self) -> Vec<Action> {
        let all: Vec<u32> = self.entries.keys().copied().collect();
        all.into_iter()
            .filter_map(|generation| self.remove(generation))
            .collect()
    }

    fn remove(&mut self, generation: u32) -> Option<Action> {
        if self.winner == Some(generation) {
            self.winner = None;
        }
        self.entries
            .remove(&generation)
            .map(|_| Action::Kill { generation })
    }

    fn translate(&mut self, generation: u32, effects: Vec<Effect>) -> Vec<Action> {
        let mut actions = Vec::new();
        for effect in effects {
            match effect {
                Effect::Spawn { method, login } => actions.push(Action::Spawn {
                    generation,
                    method,
                    login,
                }),
                Effect::Answer(secret) => actions.push(Action::Answer { generation, secret }),
                Effect::Relay(event) => actions.push(Action::Relay(event)),
                Effect::StartSession(session) => {
                    actions.push(Action::StartSession {
                        generation,
                        session,
                    });
                }
                Effect::Close => actions.extend(self.remove(generation)),
            }
        }
        let won = self.winner.is_none() && self.phase(generation) == Some(Phase::Authenticated);
        if won {
            self.winner = Some(generation);
            let losers: Vec<u32> = self
                .entries
                .keys()
                .copied()
                .filter(|other| *other != generation)
                .collect();
            actions.extend(losers.into_iter().filter_map(|other| self.remove(other)));
        }
        actions
    }
}

#[cfg(test)]
mod tests;
