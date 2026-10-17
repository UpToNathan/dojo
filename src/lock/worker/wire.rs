//! The datagram format on the worker's `SOCK_SEQPACKET` socket, both directions.
//!
//! Worker to parent: `P` + echo byte (0 or 1), `I` + text, `E` + text, `D` + PAM code (i32, native endian).
//! Parent to worker: `A` + password bytes. Nothing else crosses the socket.

use std::fmt;
use std::io;
use std::os::fd::{AsRawFd, BorrowedFd};

use crate::lock::pam::{MessageStyle, Notice, PamOutcome, Report};
use crate::lock::secret::Password;

const PROMPT: u8 = b'P';
const INFO: u8 = b'I';
const ERROR: u8 = b'E';
const DONE: u8 = b'D';
const ANSWER: u8 = b'A';

/// PAM_MAX_MSG_SIZE. Longer text from a module is cut.
const MAX_TEXT: usize = 512;
/// The largest datagram in either direction: a tag plus a full password or text.
pub(super) const MAX_DATAGRAM: usize = 1 + if Password::CAPACITY > MAX_TEXT {
    Password::CAPACITY
} else {
    MAX_TEXT
};

pub(super) fn prompt(echo: bool) -> [u8; 2] {
    [PROMPT, echo as u8]
}

pub(super) fn text(style: MessageStyle, text: &[u8]) -> Vec<u8> {
    let tag = match style {
        MessageStyle::Info => INFO,
        MessageStyle::Error => ERROR,
    };
    let text = &text[..text.len().min(MAX_TEXT)];
    let mut datagram = Vec::with_capacity(1 + text.len());
    datagram.push(tag);
    datagram.extend_from_slice(text);
    datagram
}

pub(super) fn done(code: i32) -> [u8; 5] {
    let [a, b, c, d] = code.to_ne_bytes();
    [DONE, a, b, c, d]
}

/// Parent side. `None` for anything the worker must never send.
pub(super) fn decode_report(datagram: &[u8]) -> Option<Report> {
    let (&tag, body) = datagram.split_first()?;
    let style = match (tag, body) {
        (PROMPT, [0]) => return Some(Report::Prompt { echo: false }),
        (PROMPT, [1]) => return Some(Report::Prompt { echo: true }),
        (DONE, body) => {
            let code = i32::from_ne_bytes(body.try_into().ok()?);
            return Some(Report::Finished(PamOutcome::from_code(code)));
        }
        (INFO, _) => MessageStyle::Info,
        (ERROR, _) => MessageStyle::Error,
        _ => return None,
    };
    Some(Report::Notice(Notice::parse(
        style,
        &String::from_utf8_lossy(body),
    )))
}

pub(super) fn send(socket: BorrowedFd<'_>, datagram: &[u8]) -> io::Result<()> {
    let sent = retry(|| unsafe {
        libc::send(
            socket.as_raw_fd(),
            datagram.as_ptr().cast(),
            datagram.len(),
            libc::MSG_NOSIGNAL,
        )
    })?;
    if sent == datagram.len() {
        Ok(())
    } else {
        Err(io::ErrorKind::WriteZero.into())
    }
}

/// One datagram, gathered straight from the password's buffer so no copy is made.
pub(super) fn send_answer(socket: BorrowedFd<'_>, password: &Password) -> io::Result<()> {
    let secret = password.expose();
    let mut parts = [
        libc::iovec {
            iov_base: (&ANSWER as *const u8).cast_mut().cast(),
            iov_len: 1,
        },
        libc::iovec {
            iov_base: secret.as_ptr().cast_mut().cast(),
            iov_len: secret.len(),
        },
    ];
    let mut header: libc::msghdr = unsafe { std::mem::zeroed() };
    header.msg_iov = parts.as_mut_ptr();
    header.msg_iovlen = parts.len();
    let sent = retry(|| unsafe { libc::sendmsg(socket.as_raw_fd(), &header, libc::MSG_NOSIGNAL) })?;
    if sent == 1 + secret.len() {
        Ok(())
    } else {
        Err(io::ErrorKind::WriteZero.into())
    }
}

/// Reads one datagram into `buffer`. Ok(0) is end of stream; an oversized datagram is an error.
pub(super) fn receive(socket: BorrowedFd<'_>, buffer: &mut [u8]) -> io::Result<usize> {
    let length = retry(|| unsafe {
        libc::recv(
            socket.as_raw_fd(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            libc::MSG_TRUNC,
        )
    })?;
    if length > buffer.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "oversized datagram",
        ));
    }
    Ok(length)
}

/// Worker side: the password as received, in a heap buffer that never moves and is zeroed on drop.
pub(super) struct Answer {
    buffer: Box<[u8; MAX_DATAGRAM]>,
    length: usize,
}

impl Answer {
    pub fn receive(socket: BorrowedFd<'_>) -> io::Result<Answer> {
        let mut answer = Answer {
            buffer: Box::new([0; MAX_DATAGRAM]),
            length: 0,
        };
        answer.length = receive(socket, &mut answer.buffer[..])?;
        match answer.buffer[..answer.length].split_first() {
            None => Err(io::ErrorKind::UnexpectedEof.into()),
            Some((&ANSWER, secret)) if !secret.contains(&0) => Ok(answer),
            Some(_) => Err(io::Error::new(io::ErrorKind::InvalidData, "not an answer")),
        }
    }

    pub fn secret(&self) -> &[u8] {
        &self.buffer[1..self.length]
    }
}

impl Drop for Answer {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut *self.buffer);
    }
}

impl fmt::Debug for Answer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Answer(<redacted>)")
    }
}

fn retry(mut call: impl FnMut() -> isize) -> io::Result<usize> {
    loop {
        let result = call();
        if result >= 0 {
            return Ok(result as usize);
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::{AsFd, FromRawFd, OwnedFd};
    use std::time::Duration;

    fn pair() -> (OwnedFd, OwnedFd) {
        let mut fds = [0; 2];
        let rc = unsafe {
            libc::socketpair(
                libc::AF_UNIX,
                libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC,
                0,
                fds.as_mut_ptr(),
            )
        };
        assert_eq!(rc, 0);
        unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) }
    }

    #[test]
    fn worker_datagrams_have_fixed_bytes() {
        assert_eq!(prompt(false), *b"P\x00");
        assert_eq!(prompt(true), *b"P\x01");
        assert_eq!(text(MessageStyle::Info, b"hi"), b"Ihi");
        assert_eq!(text(MessageStyle::Error, b"no"), b"Eno");
        assert_eq!(done(7), [b'D', 7, 0, 0, 0]);
        assert_eq!(text(MessageStyle::Info, &[b'x'; 600]).len(), 513);
    }

    #[test]
    fn worker_datagrams_decode_to_reports() {
        assert_eq!(
            decode_report(b"P\x00"),
            Some(Report::Prompt { echo: false })
        );
        assert_eq!(decode_report(b"P\x01"), Some(Report::Prompt { echo: true }));
        assert_eq!(
            decode_report(&done(0)),
            Some(Report::Finished(PamOutcome::Success))
        );
        assert_eq!(
            decode_report(&done(11)),
            Some(Report::Finished(PamOutcome::MaxTries))
        );
        assert_eq!(
            decode_report(b"Iplace finger"),
            Some(Report::Notice(Notice::Hint {
                text: "place finger".into(),
                error: false
            }))
        );
        assert_eq!(
            decode_report(b"E(3 minutes left to unlock)"),
            Some(Report::Notice(Notice::PasswordPaused {
                remaining: Duration::from_secs(180)
            }))
        );
    }

    #[test]
    fn malformed_worker_datagrams_are_refused() {
        for datagram in [
            &b""[..],
            b"P",
            b"P\x02",
            b"P\x00\x00",
            b"D\x00",
            b"D\x00\x00\x00\x00\x00",
            b"Ahunter2",
            b"X",
        ] {
            assert_eq!(decode_report(datagram), None, "{datagram:?}");
        }
    }

    #[test]
    fn answer_crosses_the_socket_intact_and_stays_redacted() {
        let (parent, worker) = pair();
        let mut password = Password::new();
        for c in "pä55".chars() {
            password.push(c);
        }
        send_answer(parent.as_fd(), &password).unwrap();
        let answer = Answer::receive(worker.as_fd()).unwrap();
        assert_eq!(answer.secret(), "pä55".as_bytes());
        assert_eq!(format!("{answer:?}"), "Answer(<redacted>)");
    }

    #[test]
    fn a_full_password_fits_one_datagram() {
        let (parent, worker) = pair();
        let mut password = Password::new();
        while password.push('k') {}
        send_answer(parent.as_fd(), &password).unwrap();
        assert_eq!(
            Answer::receive(worker.as_fd()).unwrap().secret(),
            &[b'k'; Password::CAPACITY][..]
        );
    }

    #[test]
    fn worker_refuses_non_answers_and_eof() {
        let (parent, worker) = pair();
        send(parent.as_fd(), b"Pnope").unwrap();
        send(parent.as_fd(), b"Aem\0bedded").unwrap();
        send(parent.as_fd(), &[b'A'; MAX_DATAGRAM + 1]).unwrap();
        assert_eq!(
            Answer::receive(worker.as_fd()).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            Answer::receive(worker.as_fd()).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            Answer::receive(worker.as_fd()).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        drop(parent);
        assert_eq!(
            Answer::receive(worker.as_fd()).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
    }
}
