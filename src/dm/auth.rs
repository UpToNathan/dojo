//! One login attempt as a state machine. The display manager keeps one `Attempt` per
//! PAM helper (a password and a fingerprint attempt race, each with its own generation)
//! and feeds it greeter requests and helper events. `reduce` is pure: it returns the
//! effects the caller must carry out, or rejects the input and leaves the state alone.

use super::launch::{HelperEvent, SessionEvent};
use super::protocol::{Event, Login, Request, Secret, SessionFailure, SessionId};
use crate::lock::pam::{Method, PamOutcome, Report};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    /// Nothing begun yet.
    Idle,
    /// The helper is running the PAM conversation.
    Authenticating,
    /// PAM accepted the user and the account; the helper waits for `StartSession`.
    Authenticated,
    /// `StartSession` is accepted and the helper is opening the session.
    Starting,
    SessionRunning,
    /// Terminal. The helper is gone or is being killed.
    Closed,
}

/// What a `Begin` created. Every phase between `Idle` and `Closed` holds one.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Live {
    generation: u32,
    method: Method,
    session: SessionId,
}

#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub enum Attempt {
    #[default]
    Idle,
    Authenticating(Live),
    Authenticated(Live),
    Starting(Live),
    SessionRunning(Live),
    Closed,
}

#[derive(PartialEq, Debug)]
pub enum Input {
    Greeter(Request),
    Helper(HelperEvent),
}

/// What the caller must do, in order.
#[derive(PartialEq, Debug)]
pub enum Effect {
    /// Start `dojo-dm-pam` for this method and send it the login as its first datagram.
    Spawn { method: Method, login: Login },
    /// Hand the password to the helper's pending PAM prompt.
    Answer(Secret),
    /// Send this to the greeter.
    Relay(Event),
    /// Resolve the id against the offered sessions and send the helper its `Start`.
    /// An id that resolves to nothing is the caller's to fail: relay
    /// `SessionFailed { UnknownSession }` and kill the helper.
    StartSession(SessionId),
    /// Kill the helper and its tree. Idempotent.
    Close,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Rejected {
    /// The request names another generation.
    Stale,
    /// The right attempt at the wrong moment: `StartSession` before success, a second `Begin`.
    OutOfOrder,
    /// Not about a login attempt at all: `Hello`, `SelectUser`, `Power`.
    Unrelated,
    /// The attempt is over.
    Closed,
}

impl Attempt {
    pub fn phase(&self) -> Phase {
        match self {
            Attempt::Idle => Phase::Idle,
            Attempt::Authenticating(_) => Phase::Authenticating,
            Attempt::Authenticated(_) => Phase::Authenticated,
            Attempt::Starting(_) => Phase::Starting,
            Attempt::SessionRunning(_) => Phase::SessionRunning,
            Attempt::Closed => Phase::Closed,
        }
    }

    pub fn generation(&self) -> Option<u32> {
        self.live().map(|live| live.generation)
    }

    fn live(&self) -> Option<&Live> {
        match self {
            Attempt::Authenticating(live)
            | Attempt::Authenticated(live)
            | Attempt::Starting(live)
            | Attempt::SessionRunning(live) => Some(live),
            Attempt::Idle | Attempt::Closed => None,
        }
    }

    /// Applies one input. On `Err` the attempt is exactly as it was.
    pub fn reduce(&mut self, input: Input) -> Result<Vec<Effect>, Rejected> {
        if let Attempt::Closed = self {
            return Err(Rejected::Closed);
        }
        match input {
            Input::Greeter(request) => self.greeter(request),
            Input::Helper(event) => self.helper(event),
        }
    }

    fn greeter(&mut self, request: Request) -> Result<Vec<Effect>, Rejected> {
        let generation = match &request {
            Request::Begin { generation, .. }
            | Request::Answer { generation, .. }
            | Request::Cancel { generation }
            | Request::StartSession { generation } => *generation,
            Request::Hello { .. } | Request::SelectUser { .. } | Request::Power { .. } => {
                return Err(Rejected::Unrelated);
            }
        };
        if let Request::Begin {
            method,
            login,
            session,
            ..
        } = request
        {
            if !matches!(self, Attempt::Idle) {
                return Err(Rejected::OutOfOrder);
            }
            *self = Attempt::Authenticating(Live {
                generation,
                method,
                session,
            });
            return Ok(vec![Effect::Spawn { method, login }]);
        }
        let live = self.live().ok_or(Rejected::OutOfOrder)?;
        if live.generation != generation {
            return Err(Rejected::Stale);
        }
        match (&*self, request) {
            (Attempt::Authenticating(_), Request::Answer { secret, .. }) => {
                Ok(vec![Effect::Answer(secret)])
            }
            (Attempt::Authenticating(_) | Attempt::Authenticated(_), Request::Cancel { .. }) => {
                *self = Attempt::Closed;
                Ok(vec![Effect::Close])
            }
            (Attempt::Authenticated(live), Request::StartSession { .. }) => {
                let live = live.clone();
                let effect = Effect::StartSession(live.session.clone());
                *self = Attempt::Starting(live);
                Ok(vec![effect])
            }
            _ => Err(Rejected::OutOfOrder),
        }
    }

    fn helper(&mut self, event: HelperEvent) -> Result<Vec<Effect>, Rejected> {
        let live = self.live().ok_or(Rejected::OutOfOrder)?.clone();
        let relay = |report: Report| {
            Effect::Relay(Event::Report {
                generation: live.generation,
                method: live.method,
                report,
            })
        };
        let failed = |reason: SessionFailure| Effect::Relay(Event::SessionFailed { reason });
        let (next, effects) = match (&*self, event) {
            (
                Attempt::Authenticating(_),
                HelperEvent::Report(report @ (Report::Prompt { .. } | Report::Notice(_))),
            ) => (None, vec![relay(report)]),
            (
                Attempt::Authenticating(_),
                HelperEvent::Report(report @ Report::Finished(PamOutcome::Success)),
            ) => (
                Some(Attempt::Authenticated(live.clone())),
                vec![relay(report)],
            ),
            (
                Attempt::Authenticating(_),
                HelperEvent::Report(report @ (Report::Finished(_) | Report::Crashed)),
            ) => (Some(Attempt::Closed), vec![relay(report), Effect::Close]),
            (
                Attempt::Authenticated(_) | Attempt::Starting(_),
                HelperEvent::Report(Report::Crashed),
            ) => (
                Some(Attempt::Closed),
                vec![failed(SessionFailure::HelperCrashed), Effect::Close],
            ),
            (Attempt::SessionRunning(_), HelperEvent::Report(Report::Crashed)) => {
                (Some(Attempt::Closed), vec![Effect::Close])
            }
            (Attempt::Starting(_), HelperEvent::Session(SessionEvent::Started)) => (
                Some(Attempt::SessionRunning(live.clone())),
                vec![Effect::Relay(Event::SessionStarted)],
            ),
            (Attempt::Starting(_), HelperEvent::Session(SessionEvent::Failed(reason))) => {
                (Some(Attempt::Closed), vec![failed(reason), Effect::Close])
            }
            (Attempt::Starting(_), HelperEvent::Session(SessionEvent::Ended)) => (
                Some(Attempt::Closed),
                vec![failed(SessionFailure::Exited), Effect::Close],
            ),
            (Attempt::SessionRunning(_), HelperEvent::Session(SessionEvent::Ended)) => {
                (Some(Attempt::Closed), vec![Effect::Close])
            }
            _ => return Err(Rejected::OutOfOrder),
        };
        if let Some(next) = next {
            *self = next;
        }
        Ok(effects)
    }
}

#[cfg(test)]
mod tests;
