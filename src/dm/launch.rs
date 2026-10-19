//! The datagrams between `dojo-dm` and its root helper `dojo-dm-pam`, beyond the ones
//! `lock::worker::wire` already defines. This set is internal: the greeter never sees it.
//!
//! Display manager to helper: `L` login (always the first datagram), `A` answer
//! (`lock::worker::wire`), `G` go: start the session.
//! Helper to display manager: the lock worker's `P` `I` `E` `D`, then `S` session
//! started, `F` session failed, `X` session ended.

use super::codec::{DecodeError, Reader, Writer};
use super::protocol::{Login, MAX_LOGIN, MAX_SESSION_ID, SessionFailure, SessionId};
use crate::lock::pam::Report;
use crate::lock::worker::wire;

/// The largest datagram the helper reads once the conversation is over.
pub const MAX_DATAGRAM: usize = 8 * 1024;
pub const MAX_EXEC: usize = 4096;
pub const MAX_DESKTOP_NAMES: usize = 8;
pub const MAX_DESKTOP_NAME: usize = 64;
const MAX_TTY: usize = 16;

const LOGIN: u8 = b'L';
const GO: u8 = b'G';
const STARTED: u8 = b'S';
const FAILED: u8 = b'F';
const ENDED: u8 = b'X';

/// Everything the helper needs to start a session, from the display manager's own
/// root-owned view of the chosen desktop entry. The greeter contributes none of it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Start {
    /// The entry's `Exec`, one shell word list. The helper runs it as `exec <exec>`.
    pub exec: String,
    /// The entry's id, which becomes `XDG_SESSION_DESKTOP`.
    pub desktop: SessionId,
    /// The entry's `DesktopNames`, which become `XDG_CURRENT_DESKTOP`.
    pub desktop_names: Vec<String>,
    pub vtnr: u8,
    /// `PAM_TTY`: `ttyN` or `/dev/ttyN`.
    pub tty: String,
}

impl Start {
    pub fn encode(&self) -> Vec<u8> {
        let mut writer = Writer::new(GO, 256 + self.exec.len());
        writer
            .u8(self.vtnr)
            .text(&self.tty, MAX_TTY)
            .text(self.desktop.as_str(), MAX_SESSION_ID)
            .u8(self.desktop_names.len().min(MAX_DESKTOP_NAMES) as u8);
        for name in self.desktop_names.iter().take(MAX_DESKTOP_NAMES) {
            writer.text(name, MAX_DESKTOP_NAME);
        }
        writer.text(&self.exec, MAX_EXEC);
        writer.finish()
    }

    pub fn decode(datagram: &[u8]) -> Result<Start, DecodeError> {
        let mut reader = Reader::new(datagram);
        match reader.u8()? {
            GO => {}
            tag => return Err(DecodeError::UnknownTag(tag)),
        }
        let vtnr = reader.u8()?;
        if !(1..=63).contains(&vtnr) {
            return Err(DecodeError::Invalid("vtnr"));
        }
        let tty = reader.text("tty", MAX_TTY)?;
        if !is_tty(tty) {
            return Err(DecodeError::Invalid("tty"));
        }
        let desktop = SessionId::parse(reader.text("desktop", MAX_SESSION_ID)?)
            .ok_or(DecodeError::Invalid("desktop"))?;
        let count = usize::from(reader.u8()?);
        if count > MAX_DESKTOP_NAMES {
            return Err(DecodeError::Invalid("desktop names"));
        }
        let mut desktop_names = Vec::with_capacity(count);
        for _ in 0..count {
            let name = reader.text("desktop name", MAX_DESKTOP_NAME)?;
            if !is_desktop_name(name) {
                return Err(DecodeError::Invalid("desktop name"));
            }
            desktop_names.push(name.to_owned());
        }
        let exec = reader.text("exec", MAX_EXEC)?;
        if !is_exec(exec) {
            return Err(DecodeError::Invalid("exec"));
        }
        reader.finish()?;
        Ok(Start {
            exec: exec.to_owned(),
            desktop,
            desktop_names,
            vtnr,
            tty: tty.to_owned(),
        })
    }
}

/// `ttyN` or `/dev/ttyN`, one or two digits.
fn is_tty(text: &str) -> bool {
    let name = text.strip_prefix("/dev/").unwrap_or(text);
    name.strip_prefix("tty").is_some_and(|number| {
        (1..=2).contains(&number.len()) && number.bytes().all(|c| c.is_ascii_digit())
    })
}

/// A word of `XDG_CURRENT_DESKTOP`: no `:` or `;`, which would split it.
fn is_desktop_name(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.'))
}

/// One line without control characters, so the shell sees exactly one command line.
fn is_exec(text: &str) -> bool {
    !text.trim().is_empty() && !text.chars().any(char::is_control)
}

pub fn encode_login(login: &Login) -> Vec<u8> {
    let mut writer = Writer::new(LOGIN, MAX_LOGIN);
    writer.raw(login.as_str().as_bytes());
    writer.finish()
}

pub fn decode_login(datagram: &[u8]) -> Result<Login, DecodeError> {
    match datagram.split_first() {
        Some((&LOGIN, name)) => std::str::from_utf8(name)
            .ok()
            .and_then(Login::parse)
            .ok_or(DecodeError::Invalid("login")),
        Some((&tag, _)) => Err(DecodeError::UnknownTag(tag)),
        None => Err(DecodeError::Truncated),
    }
}

/// What happened to the session after `Start`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SessionEvent {
    Started,
    Failed(SessionFailure),
    Ended,
}

/// Anything the helper can say.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum HelperEvent {
    Report(Report),
    Session(SessionEvent),
}

pub fn encode_session_event(event: SessionEvent) -> Vec<u8> {
    match event {
        SessionEvent::Started => vec![STARTED],
        SessionEvent::Failed(reason) => vec![FAILED, reason.byte()],
        SessionEvent::Ended => vec![ENDED],
    }
}

/// `None` for anything the helper must never send.
pub fn decode_helper_event(datagram: &[u8]) -> Option<HelperEvent> {
    match datagram {
        [STARTED] => Some(HelperEvent::Session(SessionEvent::Started)),
        [ENDED] => Some(HelperEvent::Session(SessionEvent::Ended)),
        [FAILED, reason] => Some(HelperEvent::Session(SessionEvent::Failed(
            SessionFailure::from_byte(*reason)?,
        ))),
        _ => wire::decode_report(datagram).map(HelperEvent::Report),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lock::pam::PamOutcome;

    fn start() -> Start {
        Start {
            exec: "uwsm start -F -- niri".into(),
            desktop: SessionId::parse("niri-uwsm").unwrap(),
            desktop_names: vec!["niri".into(), "GNOME".into()],
            vtnr: 1,
            tty: "tty1".into(),
        }
    }

    #[test]
    fn start_round_trips() {
        assert_eq!(Start::decode(&start().encode()), Ok(start()));
        let mut bare = start();
        bare.desktop_names.clear();
        bare.tty = "/dev/tty12".into();
        bare.vtnr = 12;
        assert_eq!(Start::decode(&bare.encode()), Ok(bare));
    }

    #[test]
    fn start_has_fixed_bytes() {
        let start = Start {
            exec: "niri".into(),
            desktop: SessionId::parse("n").unwrap(),
            desktop_names: vec!["N".into()],
            vtnr: 2,
            tty: "tty2".into(),
        };
        assert_eq!(
            start.encode(),
            [
                b'G', 2, 4, 0, b't', b't', b'y', b'2', 1, 0, b'n', 1, 1, 0, b'N', 4, 0, b'n', b'i',
                b'r', b'i'
            ]
        );
    }

    #[test]
    fn start_fields_are_validated() {
        let with = |change: &dyn Fn(&mut Start)| {
            let mut changed = start();
            change(&mut changed);
            Start::decode(&changed.encode())
        };
        assert_eq!(with(&|s| s.vtnr = 0), Err(DecodeError::Invalid("vtnr")));
        assert_eq!(with(&|s| s.vtnr = 64), Err(DecodeError::Invalid("vtnr")));
        for tty in [
            "",
            "tty",
            "ttyX",
            "tty123",
            "/dev/null",
            "../tty1",
            "/dev/tty1/x",
        ] {
            assert_eq!(
                with(&|s| s.tty = tty.into()),
                Err(DecodeError::Invalid("tty")),
                "{tty}"
            );
        }
        for exec in ["", "  ", "niri\nrm -rf ~", "niri\0", "a\tb\u{7}"] {
            assert_eq!(
                with(&|s| s.exec = exec.into()),
                Err(DecodeError::Invalid("exec")),
                "{exec:?}"
            );
        }
        for name in ["", "a:b", "a;b", "a b"] {
            assert_eq!(
                with(&|s| s.desktop_names = vec![name.into()]),
                Err(DecodeError::Invalid("desktop name")),
                "{name:?}"
            );
        }
        assert!(Start::decode(&[]).is_err());
        assert!(Start::decode(b"Lniri").is_err());
        let mut trailing = start().encode();
        trailing.push(0);
        assert_eq!(Start::decode(&trailing), Err(DecodeError::Trailing));
        assert!(start().encode().len() <= MAX_DATAGRAM);
        let mut huge = start();
        huge.exec = "x".repeat(MAX_EXEC);
        huge.desktop_names = vec!["n".repeat(MAX_DESKTOP_NAME); MAX_DESKTOP_NAMES];
        assert_eq!(Start::decode(&huge.encode()), Ok(huge.clone()));
        assert!(huge.encode().len() <= MAX_DATAGRAM);
    }

    #[test]
    fn start_decoding_is_total() {
        let valid = start().encode();
        for end in 0..valid.len() {
            assert!(Start::decode(&valid[..end]).is_err());
        }
        let mut seed = 0x2545_f491_4f6c_dd1d_u64;
        for _ in 0..5_000 {
            let mut mutated = valid.clone();
            for _ in 0..3 {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                let index = (seed % mutated.len() as u64) as usize;
                mutated[index] = (seed >> 32) as u8;
            }
            let _ = Start::decode(&mutated);
        }
    }

    #[test]
    fn login_is_the_first_datagram_and_must_be_valid() {
        let login = Login::parse("nathan").unwrap();
        assert_eq!(encode_login(&login), b"Lnathan");
        assert_eq!(decode_login(b"Lnathan"), Ok(login));
        for bad in [
            &b""[..],
            b"L",
            b"Lroot\0",
            b"LRoot",
            b"L../x",
            b"Anathan",
            b"L\xff",
        ] {
            assert!(decode_login(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn helper_events_decode_the_lock_vocabulary_and_the_session_tags() {
        assert_eq!(
            decode_helper_event(&wire::done(0)),
            Some(HelperEvent::Report(Report::Finished(PamOutcome::Success)))
        );
        assert_eq!(
            decode_helper_event(&wire::prompt(false)),
            Some(HelperEvent::Report(Report::Prompt { echo: false }))
        );
        for event in [
            SessionEvent::Started,
            SessionEvent::Ended,
            SessionEvent::Failed(SessionFailure::Refused),
            SessionEvent::Failed(SessionFailure::SpawnFailed),
        ] {
            assert_eq!(
                decode_helper_event(&encode_session_event(event)),
                Some(HelperEvent::Session(event))
            );
        }
        for bad in [&b"S\0"[..], b"X\0", b"F", b"F\x09", b"F\0\0", b""] {
            assert_eq!(decode_helper_event(bad), None, "{bad:?}");
        }
    }
}
