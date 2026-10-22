//! The greeter socket: what `dojo-greeter` and `dojo-dm` say to each other.
//!
//! Each message is one `SOCK_SEQPACKET` datagram: a tag byte, then fixed-width
//! little-endian fields and u16-length-prefixed UTF-8 text, in the style of
//! `lock::worker::wire`. Nothing that can hold a password is serialized with serde.
//! The password travels as the unframed tail of an `Answer`, lives in a [`Password`]
//! (fixed buffer, zeroed on drop) and every buffer that held it is zeroed.
//!
//! Greeter to display manager: `H` hello, `U` select user, `B` begin, `A` answer,
//! `C` cancel, `G` start session, `W` power.
//! Display manager to greeter: `i` init, `u` user, `r` report, `s` session started,
//! `f` session failed.

use std::io;
use std::os::fd::{AsRawFd, BorrowedFd};
use std::time::Duration;

use zeroize::Zeroizing;

pub use super::codec::DecodeError;
use super::codec::{Reader, Writer};
use crate::lock::pam::{Method, Notice, PamOutcome, Report};
use crate::lock::secret::Password;
use crate::lock::worker::wire;
use crate::session::Occasion;

pub const VERSION: u16 = 1;
/// Every datagram in either direction fits here; the largest is `Init`.
pub const MAX_DATAGRAM: usize = 16 * 1024;
pub const MAX_LOGIN: usize = 32;
pub const MAX_SESSION_ID: usize = 64;
pub const MAX_SESSIONS: usize = 32;
/// Display names and session names, in bytes.
pub const MAX_NAME: usize = 128;
/// PAM_MAX_MSG_SIZE, the same cut `lock::worker::wire` applies.
pub const MAX_TEXT: usize = 512;

pub use crate::session::snapshot::Login;

/// A desktop entry id such as `niri-uwsm`: a file stem the display manager resolves
/// against its own session directories. It never names a path.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct SessionId(String);

impl SessionId {
    pub fn parse(text: &str) -> Option<SessionId> {
        let bytes = text.as_bytes();
        let allowed = |c: &u8| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b'+');
        let valid = (1..=MAX_SESSION_ID).contains(&bytes.len())
            && !matches!(bytes[0], b'.' | b'-')
            && bytes.iter().all(allowed);
        valid.then(|| SessionId(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SessionEntry {
    pub id: SessionId,
    pub name: String,
}

/// The occasions a login screen can start: only the ones that end in a power action.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PowerOccasion {
    Suspend,
    Hibernate,
    Restart,
    Shutdown,
}

impl PowerOccasion {
    pub fn occasion(self) -> Occasion {
        match self {
            PowerOccasion::Suspend => Occasion::Suspend,
            PowerOccasion::Hibernate => Occasion::Hibernate,
            PowerOccasion::Restart => Occasion::Restart,
            PowerOccasion::Shutdown => Occasion::Shutdown,
        }
    }

    fn byte(self) -> u8 {
        match self {
            PowerOccasion::Suspend => 0,
            PowerOccasion::Hibernate => 1,
            PowerOccasion::Restart => 2,
            PowerOccasion::Shutdown => 3,
        }
    }

    fn from_byte(byte: u8) -> Option<PowerOccasion> {
        Some(match byte {
            0 => PowerOccasion::Suspend,
            1 => PowerOccasion::Hibernate,
            2 => PowerOccasion::Restart,
            3 => PowerOccasion::Shutdown,
            _ => return None,
        })
    }
}

/// Why a session did not start, or stopped at once.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SessionFailure {
    /// PAM refused the account, the credentials or the session.
    Refused,
    /// The session process could not be created.
    SpawnFailed,
    /// The chosen session is not one the display manager offers.
    UnknownSession,
    /// The root helper died.
    HelperCrashed,
    /// The session process ended at once.
    Exited,
}

impl SessionFailure {
    pub(super) fn byte(self) -> u8 {
        match self {
            SessionFailure::Refused => 0,
            SessionFailure::SpawnFailed => 1,
            SessionFailure::UnknownSession => 2,
            SessionFailure::HelperCrashed => 3,
            SessionFailure::Exited => 4,
        }
    }

    pub(super) fn from_byte(byte: u8) -> Option<SessionFailure> {
        Some(match byte {
            0 => SessionFailure::Refused,
            1 => SessionFailure::SpawnFailed,
            2 => SessionFailure::UnknownSession,
            3 => SessionFailure::HelperCrashed,
            4 => SessionFailure::Exited,
            _ => return None,
        })
    }
}

/// A password on its way to PAM. Debug is redacted and the bytes are zeroed on drop.
pub struct Secret(Password);

impl Secret {
    pub fn new(password: Password) -> Secret {
        Secret(password)
    }

    pub fn into_password(self) -> Password {
        self.0
    }

    /// `None` for bytes that are not UTF-8, hold a NUL, or exceed `Password::CAPACITY`.
    fn from_bytes(bytes: &[u8]) -> Option<Secret> {
        let text = std::str::from_utf8(bytes).ok()?;
        let mut password = Password::new();
        text.chars()
            .all(|c| password.push(c))
            .then_some(Secret(password))
    }
}

impl PartialEq for Secret {
    fn eq(&self, other: &Secret) -> bool {
        self.0.expose() == other.0.expose()
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

/// Greeter to display manager.
#[derive(PartialEq, Debug)]
pub enum Request {
    Hello {
        version: u16,
    },
    /// The user chip now names this user; the answer is an `Event::User`.
    SelectUser {
        login: Login,
    },
    /// Start one PAM conversation. Generations are unique across both methods.
    Begin {
        generation: u32,
        method: Method,
        login: Login,
        session: SessionId,
    },
    Answer {
        generation: u32,
        secret: Secret,
    },
    Cancel {
        generation: u32,
    },
    /// The conversation of this generation succeeded; log the user in.
    StartSession {
        generation: u32,
    },
    Power {
        occasion: PowerOccasion,
    },
}

/// Display manager to greeter.
#[derive(Clone, PartialEq, Debug)]
pub enum Event {
    Init {
        last_login: Option<Login>,
        last_session: Option<SessionId>,
        sessions: Vec<SessionEntry>,
    },
    User {
        login: Login,
        display_name: Option<String>,
        fingerprint: bool,
    },
    /// What a PAM helper said, relayed as the report stream the lock model already consumes.
    Report {
        generation: u32,
        method: Method,
        report: Report,
    },
    SessionStarted,
    SessionFailed {
        reason: SessionFailure,
    },
}

/// A protocol message that can cross the socket in one datagram.
pub trait Message: Sized {
    /// The bytes live in a buffer that is zeroed when dropped.
    fn encode(&self) -> Zeroizing<Vec<u8>>;
    /// Total: any bytes decode to a message or an error.
    fn decode(datagram: &[u8]) -> Result<Self, DecodeError>;
}

pub fn send<M: Message>(socket: BorrowedFd<'_>, message: &M) -> io::Result<()> {
    wire::send(socket, &message.encode())
}

/// One message. `Ok(None)` is end of stream; an oversized datagram is an I/O error;
/// a datagram that does not decode is the inner `Err`, and the socket stays usable.
/// The receive buffer is zeroed before this returns.
pub fn receive<M: Message>(socket: BorrowedFd<'_>) -> io::Result<Option<Result<M, DecodeError>>> {
    let mut buffer = Zeroizing::new(vec![0u8; MAX_DATAGRAM]);
    let length = wire::receive(socket, &mut buffer)?;
    if length == 0 {
        return Ok(None);
    }
    Ok(Some(M::decode(&buffer[..length])))
}

/// The uid of the process on the other end, from `SO_PEERCRED`.
pub fn peer_uid(socket: BorrowedFd<'_>) -> io::Result<u32> {
    let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
    let mut length = size_of::<libc::ucred>() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            socket.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut credentials as *mut libc::ucred).cast(),
            &mut length,
        )
    };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(credentials.uid)
}

const HELLO: u8 = b'H';
const SELECT_USER: u8 = b'U';
const BEGIN: u8 = b'B';
const ANSWER: u8 = b'A';
const CANCEL: u8 = b'C';
const START_SESSION: u8 = b'G';
const POWER: u8 = b'W';

const INIT: u8 = b'i';
const USER: u8 = b'u';
const REPORT: u8 = b'r';
const SESSION_STARTED: u8 = b's';
const SESSION_FAILED: u8 = b'f';

fn method_byte(method: Method) -> u8 {
    match method {
        Method::Fingerprint => 0,
        Method::Password => 1,
    }
}

fn method_from(byte: u8) -> Result<Method, DecodeError> {
    match byte {
        0 => Ok(Method::Fingerprint),
        1 => Ok(Method::Password),
        _ => Err(DecodeError::Invalid("method")),
    }
}

fn login(reader: &mut Reader<'_>) -> Result<Login, DecodeError> {
    Login::parse(reader.text("login", MAX_LOGIN)?).map_err(|_| DecodeError::Invalid("login"))
}

fn session_id(reader: &mut Reader<'_>) -> Result<SessionId, DecodeError> {
    SessionId::parse(reader.text("session id", MAX_SESSION_ID)?)
        .ok_or(DecodeError::Invalid("session id"))
}

impl Message for Request {
    fn encode(&self) -> Zeroizing<Vec<u8>> {
        let bytes = match self {
            Request::Hello { version } => Writer::new(HELLO, 2).u16(*version).finish(),
            Request::SelectUser { login } => Writer::new(SELECT_USER, 2 + MAX_LOGIN)
                .text(login.as_str(), MAX_LOGIN)
                .finish(),
            Request::Begin {
                generation,
                method,
                login,
                session,
            } => Writer::new(BEGIN, 5 + 4 + MAX_LOGIN + MAX_SESSION_ID)
                .u32(*generation)
                .u8(method_byte(*method))
                .text(login.as_str(), MAX_LOGIN)
                .text(session.as_str(), MAX_SESSION_ID)
                .finish(),
            Request::Answer { generation, secret } => {
                let secret = secret.0.expose();
                let mut writer = Writer::new(ANSWER, 4 + secret.len());
                writer.u32(*generation).raw(secret);
                return Zeroizing::new(writer.finish());
            }
            Request::Cancel { generation } => Writer::new(CANCEL, 4).u32(*generation).finish(),
            Request::StartSession { generation } => {
                Writer::new(START_SESSION, 4).u32(*generation).finish()
            }
            Request::Power { occasion } => Writer::new(POWER, 1).u8(occasion.byte()).finish(),
        };
        Zeroizing::new(bytes)
    }

    fn decode(datagram: &[u8]) -> Result<Request, DecodeError> {
        let mut reader = Reader::new(datagram);
        let request = match reader.u8()? {
            HELLO => Request::Hello {
                version: reader.u16()?,
            },
            SELECT_USER => Request::SelectUser {
                login: login(&mut reader)?,
            },
            BEGIN => Request::Begin {
                generation: reader.u32()?,
                method: method_from(reader.u8()?)?,
                login: login(&mut reader)?,
                session: session_id(&mut reader)?,
            },
            ANSWER => {
                let generation = reader.u32()?;
                let secret =
                    Secret::from_bytes(reader.rest()).ok_or(DecodeError::Invalid("secret"))?;
                Request::Answer { generation, secret }
            }
            CANCEL => Request::Cancel {
                generation: reader.u32()?,
            },
            START_SESSION => Request::StartSession {
                generation: reader.u32()?,
            },
            POWER => Request::Power {
                occasion: PowerOccasion::from_byte(reader.u8()?)
                    .ok_or(DecodeError::Invalid("occasion"))?,
            },
            tag => return Err(DecodeError::UnknownTag(tag)),
        };
        reader.finish()?;
        Ok(request)
    }
}

const PROMPT: u8 = 0;
const HINT: u8 = 1;
const LOCKED: u8 = 2;
const PAUSED: u8 = 3;
const FINISHED: u8 = 4;
const CRASHED: u8 = 5;

fn outcome_parts(outcome: PamOutcome) -> (u8, i32) {
    match outcome {
        PamOutcome::Success => (0, 0),
        PamOutcome::Rejected => (1, 0),
        PamOutcome::Unavailable => (2, 0),
        PamOutcome::MaxTries => (3, 0),
        PamOutcome::Other(code) => (4, code),
    }
}

fn outcome_from(reader: &mut Reader<'_>) -> Result<PamOutcome, DecodeError> {
    let kind = reader.u8()?;
    let code = reader.i32()?;
    match (kind, code) {
        (0, 0) => Ok(PamOutcome::Success),
        (1, 0) => Ok(PamOutcome::Rejected),
        (2, 0) => Ok(PamOutcome::Unavailable),
        (3, 0) => Ok(PamOutcome::MaxTries),
        (4, code) => Ok(PamOutcome::Other(code)),
        _ => Err(DecodeError::Invalid("outcome")),
    }
}

fn write_report(writer: &mut Writer, report: &Report) {
    match report {
        Report::Prompt { echo } => {
            writer.u8(PROMPT).flag(*echo);
        }
        Report::Notice(Notice::Hint { text, error }) => {
            writer.u8(HINT).flag(*error).text(text, MAX_TEXT);
        }
        Report::Notice(Notice::PasswordLocked) => {
            writer.u8(LOCKED);
        }
        Report::Notice(Notice::PasswordPaused { remaining }) => {
            let seconds = u32::try_from(remaining.as_secs()).unwrap_or(u32::MAX);
            writer.u8(PAUSED).u32(seconds);
        }
        Report::Finished(outcome) => {
            let (kind, code) = outcome_parts(*outcome);
            writer.u8(FINISHED).u8(kind).i32(code);
        }
        Report::Crashed => {
            writer.u8(CRASHED);
        }
    }
}

fn read_report(reader: &mut Reader<'_>) -> Result<Report, DecodeError> {
    Ok(match reader.u8()? {
        PROMPT => Report::Prompt {
            echo: reader.flag("echo")?,
        },
        HINT => {
            let error = reader.flag("error")?;
            Report::Notice(Notice::Hint {
                text: reader.text("hint", MAX_TEXT)?.to_owned(),
                error,
            })
        }
        LOCKED => Report::Notice(Notice::PasswordLocked),
        PAUSED => Report::Notice(Notice::PasswordPaused {
            remaining: Duration::from_secs(u64::from(reader.u32()?)),
        }),
        FINISHED => Report::Finished(outcome_from(reader)?),
        CRASHED => Report::Crashed,
        _ => return Err(DecodeError::Invalid("report")),
    })
}

impl Message for Event {
    fn encode(&self) -> Zeroizing<Vec<u8>> {
        let bytes = match self {
            Event::Init {
                last_login,
                last_session,
                sessions,
            } => {
                let mut writer = Writer::new(INIT, 1024);
                writer.flag(last_login.is_some());
                if let Some(login) = last_login {
                    writer.text(login.as_str(), MAX_LOGIN);
                }
                writer.flag(last_session.is_some());
                if let Some(session) = last_session {
                    writer.text(session.as_str(), MAX_SESSION_ID);
                }
                let sessions = &sessions[..sessions.len().min(MAX_SESSIONS)];
                writer.u8(sessions.len() as u8);
                for entry in sessions {
                    writer
                        .text(entry.id.as_str(), MAX_SESSION_ID)
                        .text(&entry.name, MAX_NAME);
                }
                writer.finish()
            }
            Event::User {
                login,
                display_name,
                fingerprint,
            } => {
                let mut writer = Writer::new(USER, 2 + MAX_LOGIN + 3 + MAX_NAME);
                writer.text(login.as_str(), MAX_LOGIN);
                writer.flag(display_name.is_some());
                if let Some(name) = display_name {
                    writer.text(name, MAX_NAME);
                }
                writer.flag(*fingerprint).finish()
            }
            Event::Report {
                generation,
                method,
                report,
            } => {
                let mut writer = Writer::new(REPORT, 8 + MAX_TEXT);
                writer.u32(*generation).u8(method_byte(*method));
                write_report(&mut writer, report);
                writer.finish()
            }
            Event::SessionStarted => Writer::new(SESSION_STARTED, 0).finish(),
            Event::SessionFailed { reason } => {
                Writer::new(SESSION_FAILED, 1).u8(reason.byte()).finish()
            }
        };
        Zeroizing::new(bytes)
    }

    fn decode(datagram: &[u8]) -> Result<Event, DecodeError> {
        let mut reader = Reader::new(datagram);
        let event = match reader.u8()? {
            INIT => {
                let last_login = reader
                    .flag("last login")?
                    .then(|| login(&mut reader))
                    .transpose()?;
                let last_session = reader
                    .flag("last session")?
                    .then(|| session_id(&mut reader))
                    .transpose()?;
                let count = usize::from(reader.u8()?);
                if count > MAX_SESSIONS {
                    return Err(DecodeError::Invalid("sessions"));
                }
                let mut sessions = Vec::with_capacity(count);
                for _ in 0..count {
                    sessions.push(SessionEntry {
                        id: session_id(&mut reader)?,
                        name: reader.text("session name", MAX_NAME)?.to_owned(),
                    });
                }
                Event::Init {
                    last_login,
                    last_session,
                    sessions,
                }
            }
            USER => {
                let login = login(&mut reader)?;
                let display_name = reader
                    .flag("display name")?
                    .then(|| reader.text("display name", MAX_NAME).map(str::to_owned))
                    .transpose()?;
                Event::User {
                    login,
                    display_name,
                    fingerprint: reader.flag("fingerprint")?,
                }
            }
            REPORT => Event::Report {
                generation: reader.u32()?,
                method: method_from(reader.u8()?)?,
                report: read_report(&mut reader)?,
            },
            SESSION_STARTED => Event::SessionStarted,
            SESSION_FAILED => Event::SessionFailed {
                reason: SessionFailure::from_byte(reader.u8()?)
                    .ok_or(DecodeError::Invalid("reason"))?,
            },
            tag => return Err(DecodeError::UnknownTag(tag)),
        };
        reader.finish()?;
        Ok(event)
    }
}

#[cfg(test)]
mod tests;
