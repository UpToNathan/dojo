//! Login mode's vocabulary: the account name the user chip holds, the session
//! choice and the single-line name editor. Pure data and rules; `LockModel`
//! turns the outcomes into effects.

use std::fmt;
use std::time::Instant;

use super::model::{GreetingSet, Key};

/// The longest account name accepted, in bytes (the Linux `useradd` limit).
pub const MAX_LOGIN_BYTES: usize = 32;

/// An account name that passed the boundary: non-empty, at most
/// `MAX_LOGIN_BYTES` bytes, no whitespace and no control characters. Whether the
/// account exists is PAM's to say, and a name with no look still gets the default one.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Username(String);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NameError {
    Empty,
    TooLong,
    /// Whitespace or a control character.
    Forbidden,
}

impl Username {
    pub fn parse(text: &str) -> Result<Username, NameError> {
        if text.is_empty() {
            Err(NameError::Empty)
        } else if text.len() > MAX_LOGIN_BYTES {
            Err(NameError::TooLong)
        } else if text.chars().any(|c| c.is_whitespace() || c.is_control()) {
            Err(NameError::Forbidden)
        } else {
            Ok(Username(text.to_owned()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Username {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum LoginError {
    User(NameError),
    NoSessions,
}

/// Login mode's settings, built by the edge. `Settings::user` stays the display
/// name shown in the user chip.
#[derive(Clone, PartialEq, Debug)]
pub struct Login {
    user: Username,
    sessions: Vec<String>,
    selected: usize,
    /// Whether the fingerprint method exists on this screen: a reader, and an enrolled print
    /// for the typed user. Without it the scene has no sensor state and no fingerprint worker runs.
    pub fingerprint: bool,
}

impl Login {
    /// `user` is the prefilled account, `sessions` the offered session names and `selected` the
    /// last choice. A `selected` that is not offered falls back to the first session.
    pub fn new(user: &str, sessions: Vec<String>, selected: &str) -> Result<Login, LoginError> {
        let user = Username::parse(user).map_err(LoginError::User)?;
        if sessions.is_empty() {
            return Err(LoginError::NoSessions);
        }
        let selected = sessions.iter().position(|s| s == selected).unwrap_or(0);
        Ok(Login {
            user,
            sessions,
            selected,
            fingerprint: false,
        })
    }

    pub fn with_fingerprint(self, fingerprint: bool) -> Login {
        Login {
            fingerprint,
            ..self
        }
    }
}

/// What a successful login starts: read it when `Effect::Unlock` arrives.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Choice<'a> {
    pub login: &'a str,
    pub session: &'a str,
}

/// The edge's answer to `Effect::SwapLook`: what it found for `login`.
#[derive(Clone, PartialEq, Debug)]
pub struct UserLook {
    pub login: String,
    /// The name for the user chip. `None`: no look snapshot, so the chip keeps the login.
    pub display: Option<String>,
    /// The greetings for this user, chosen for `Occasion::Login`. `None` skips the greeting.
    pub greeting: Option<GreetingSet>,
}

/// The username being typed. Refusals are recorded for the scene and never change the text.
pub(super) struct NameBuffer {
    text: String,
    refused: bool,
}

impl NameBuffer {
    fn new() -> NameBuffer {
        NameBuffer {
            text: String::new(),
            refused: false,
        }
    }

    pub(super) fn text(&self) -> &str {
        &self.text
    }

    pub(super) fn refused(&self) -> bool {
        self.refused
    }

    fn push(&mut self, c: char) {
        let mut next = self.text.clone();
        next.push(c);
        self.refused = matches!(
            Username::parse(&next),
            Err(NameError::TooLong | NameError::Forbidden)
        );
        if !self.refused {
            self.text = next;
        }
    }
}

pub(super) struct Swap {
    /// The look on screen before the swap.
    pub(super) from: Username,
    pub(super) phase: SwapPhase,
}

pub(super) enum SwapPhase {
    /// `SwapLook` is out and the edge has not answered: the old look stays.
    Loading,
    /// The new look is fading in over `REVEAL`.
    Fading { since: Instant },
}

pub(super) struct LoginState {
    /// The committed account: the one the password worker authenticates.
    pub(super) user: Username,
    sessions: Vec<String>,
    selected: usize,
    pub(super) fingerprint: bool,
    pub(super) editing: Option<NameBuffer>,
    pub(super) swap: Option<Swap>,
}

impl LoginState {
    pub(super) fn new(login: Login) -> LoginState {
        LoginState {
            user: login.user,
            sessions: login.sessions,
            selected: login.selected,
            fingerprint: login.fingerprint,
            editing: None,
            swap: None,
        }
    }

    pub(super) fn session(&self) -> &str {
        &self.sessions[self.selected]
    }

    pub(super) fn next_session(&mut self) {
        self.selected = (self.selected + 1) % self.sessions.len();
    }

    pub(super) fn begin_edit(&mut self) {
        self.editing.get_or_insert_with(NameBuffer::new);
    }

    /// Applies a key to the name being typed. `Some` is a committed name that differs
    /// from the current one; the editor is closed by then.
    pub(super) fn edit(&mut self, key: Key) -> Option<Username> {
        let buffer = self.editing.as_mut()?;
        match key {
            Key::Char(c) => buffer.push(c),
            Key::Backspace => {
                buffer.text.pop();
                buffer.refused = false;
            }
            Key::ClearLine => *buffer = NameBuffer::new(),
            Key::Escape => self.editing = None,
            Key::Enter => match Username::parse(&buffer.text) {
                Ok(name) => {
                    self.editing = None;
                    return (name != self.user).then_some(name);
                }
                Err(_) => buffer.refused = true,
            },
            Key::Tab | Key::NextSession => {}
        }
        None
    }

    /// Makes `user` the committed account and starts a look swap from what is on screen.
    pub(super) fn commit(&mut self, user: Username) {
        let on_screen = match self.swap.take() {
            Some(Swap {
                from,
                phase: SwapPhase::Loading,
            }) => from,
            _ => self.user.clone(),
        };
        self.swap = Some(Swap {
            from: on_screen,
            phase: SwapPhase::Loading,
        });
        self.user = user;
    }

    /// The edge has the look for the committed user: fade to it, or cut under reduced motion.
    pub(super) fn look_ready(&mut self, now: Instant, reduced_motion: bool) {
        if let Some(Swap {
            phase: phase @ SwapPhase::Loading,
            ..
        }) = &mut self.swap
        {
            if reduced_motion {
                self.swap = None;
            } else {
                *phase = SwapPhase::Fading { since: now };
            }
        }
    }
}
