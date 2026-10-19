//! The greeter's process: the Niri fork running `dojo-greeter`, as the `dojo-greeter`
//! user, inside a logind session of class greeter on the display manager's terminal.
//!
//! The service sees it through two small traits, so its loop runs against a fake. The real
//! one asks `dojo-dm-pam` for the `dojo-greeter` service: the helper authenticates nothing
//! (`pam_permit`), opens the session, and execs the release's Niri with the greeter's
//! configuration as the user. The handshake with the helper is [`advance`], a pure step.

use std::io;
use std::os::fd::{AsRawFd, BorrowedFd};
use std::time::{Duration, Instant};

use super::account::{self, Entry, GREETER_LOGIN};
use super::config::Release;
use super::host::Seat;
use super::launch::{HelperEvent, SessionEvent, Start};
use super::process::{HelperProcess, HelperSpec, Received};
use super::protocol::{Login, SessionId};
use super::session::Service;
use super::vt::Vt;
use crate::lock::pam::{PamOutcome, Report};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
const STOP_TIMEOUT: Duration = Duration::from_secs(3);
/// How long the greeter's logind session may outlive its processes.
pub const SESSION_GONE_TIMEOUT: Duration = Duration::from_secs(5);
pub const DESKTOP: &str = "dojo-greeter";

/// A running greeter.
pub trait GreeterProcess {
    /// Readable when the greeter's session has probably ended; [`ended`](Self::ended) decides.
    fn exit_fd(&self) -> BorrowedFd<'_>;
    fn ended(&mut self) -> bool;
    /// Stops the session and everything in it. `true` when nothing of it is left.
    fn stop(&mut self) -> bool;
}

pub trait Greeters {
    /// The uid the greeter socket's peer must have.
    fn peer_uid(&self) -> u32;
    /// Starts the greeter on `vt`. Returns once it is running, not once it is ready.
    fn launch(&mut self, vt: Vt) -> io::Result<Box<dyn GreeterProcess>>;
}

/// The `dojo-greeter` account, if the machine has one.
pub fn greeter_account() -> Option<Entry> {
    account::lookup(&Login::parse(GREETER_LOGIN).ok()?).filter(|entry| entry.uid != 0)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Handshake {
    Authenticating,
    Starting,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Step {
    /// PAM said yes: send the helper its `Start`.
    SendStart,
    /// The session is running.
    Running,
    Failed,
}

/// The helper's side of starting the greeter: one success report, then `Started`.
/// Anything else, a prompt included, means the greeter's PAM stack is not what it should be.
pub fn advance(state: Handshake, event: &HelperEvent) -> Step {
    match (state, event) {
        (Handshake::Authenticating, HelperEvent::Report(Report::Finished(PamOutcome::Success))) => {
            Step::SendStart
        }
        (Handshake::Starting, HelperEvent::Session(SessionEvent::Started)) => Step::Running,
        _ => Step::Failed,
    }
}

pub struct PamGreeters {
    spec: HelperSpec,
    release: Release,
    uid: u32,
}

impl PamGreeters {
    pub fn new(spec: &HelperSpec, release: Release, uid: u32) -> PamGreeters {
        PamGreeters {
            spec: spec.clone(),
            release,
            uid,
        }
    }
}

impl Greeters for PamGreeters {
    fn peer_uid(&self) -> u32 {
        self.uid
    }

    fn launch(&mut self, vt: Vt) -> io::Result<Box<dyn GreeterProcess>> {
        let login = Login::parse(GREETER_LOGIN).map_err(io::Error::other)?;
        let start = Start {
            exec: self.release.greeter_command(),
            desktop: SessionId::parse(DESKTOP).ok_or_else(|| io::Error::other("desktop"))?,
            desktop_names: Vec::new(),
            vtnr: vt.number(),
            tty: vt.tty(),
        };
        let helper = HelperProcess::spawn(&self.spec, Service::Greeter, &login)?;
        let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
        let mut state = Handshake::Authenticating;
        loop {
            if !wait_readable(helper.socket(), deadline) {
                return Err(io::ErrorKind::TimedOut.into());
            }
            let event = match helper.receive() {
                Received::Event(event) => event,
                Received::Closed | Received::Garbled => {
                    return Err(io::Error::other("the greeter helper went away"));
                }
            };
            match advance(state, &event) {
                Step::SendStart => {
                    helper.start(&start)?;
                    state = Handshake::Starting;
                }
                Step::Running => {
                    return Ok(Box::new(PamGreeter {
                        helper,
                        over: false,
                    }));
                }
                Step::Failed => return Err(io::Error::other(format!("greeter helper: {event:?}"))),
            }
        }
    }
}

struct PamGreeter {
    helper: HelperProcess,
    over: bool,
}

impl PamGreeter {
    /// Reads whatever the helper sent. The session ending, the socket closing or garbage
    /// all mean the greeter is over.
    fn drain(&mut self) {
        match self.helper.receive() {
            Received::Event(HelperEvent::Session(SessionEvent::Ended))
            | Received::Closed
            | Received::Garbled => self.over = true,
            Received::Event(_) => {}
        }
    }
}

impl GreeterProcess for PamGreeter {
    fn exit_fd(&self) -> BorrowedFd<'_> {
        self.helper.socket()
    }

    fn ended(&mut self) -> bool {
        if !self.over {
            self.drain();
        }
        self.over
    }

    /// Asks the greeter's tree to stop and lets the helper close the PAM session, which is
    /// what ends the logind session. What is still alive after the timeout is killed.
    fn stop(&mut self) -> bool {
        if !self.over {
            self.helper.terminate_session();
            let deadline = Instant::now() + STOP_TIMEOUT;
            while !self.over && wait_readable(self.helper.socket(), deadline) {
                self.drain();
            }
        }
        // Ended is sent just before the helper's last PAM call; give it the moment to exit.
        self.helper.wait_for_exit(Duration::from_secs(1));
        self.helper.kill().is_empty()
    }
}

/// Waits for `fd` to become readable. `false` at the deadline.
pub fn wait_readable(fd: BorrowedFd<'_>, deadline: Instant) -> bool {
    loop {
        let now = Instant::now();
        if now >= deadline {
            return false;
        }
        let mut poll = libc::pollfd {
            fd: fd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let milliseconds = (deadline - now).as_millis().min(1000) as libc::c_int;
        match unsafe { libc::poll(&mut poll, 1, milliseconds) } {
            n if n > 0 => return true,
            n if n < 0 && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted => {
                return false;
            }
            _ => {}
        }
    }
}

/// Waits until logind has no session of `uid`, so the next session finds the seat free.
/// Returns `false` at the timeout. A logind that cannot be asked counts as gone: the
/// processes are dead, which is what frees the display.
pub fn wait_session_gone(seat: &dyn Seat, uid: u32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if seat.has_session_of(uid) != Some(true) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    fn finished(outcome: PamOutcome) -> HelperEvent {
        HelperEvent::Report(Report::Finished(outcome))
    }

    #[test]
    fn the_handshake_is_one_success_then_started() {
        assert_eq!(
            advance(Handshake::Authenticating, &finished(PamOutcome::Success)),
            Step::SendStart
        );
        assert_eq!(
            advance(
                Handshake::Starting,
                &HelperEvent::Session(SessionEvent::Started)
            ),
            Step::Running
        );
    }

    #[test]
    fn every_other_step_fails_the_start() {
        for outcome in [
            PamOutcome::Rejected,
            PamOutcome::Unavailable,
            PamOutcome::MaxTries,
        ] {
            assert_eq!(
                advance(Handshake::Authenticating, &finished(outcome)),
                Step::Failed
            );
        }
        for event in [
            HelperEvent::Report(Report::Prompt { echo: false }),
            HelperEvent::Report(Report::Crashed),
            HelperEvent::Session(SessionEvent::Started),
            HelperEvent::Session(SessionEvent::Ended),
        ] {
            assert_eq!(
                advance(Handshake::Authenticating, &event),
                Step::Failed,
                "{event:?}"
            );
        }
        for event in [
            finished(PamOutcome::Success),
            HelperEvent::Session(SessionEvent::Ended),
            HelperEvent::Session(SessionEvent::Failed(
                crate::dm::protocol::SessionFailure::SpawnFailed,
            )),
        ] {
            assert_eq!(
                advance(Handshake::Starting, &event),
                Step::Failed,
                "{event:?}"
            );
        }
    }

    struct Gone {
        calls: Cell<u32>,
        answers: Vec<Option<bool>>,
    }

    impl Seat for Gone {
        fn has_session_of(&self, _uid: u32) -> Option<bool> {
            let call = self.calls.get() as usize;
            self.calls.set(self.calls.get() + 1);
            self.answers.get(call).copied().unwrap_or(Some(true))
        }
    }

    #[test]
    fn the_seat_is_free_once_logind_forgets_the_greeter_or_cannot_say() {
        let seat = Gone {
            calls: Cell::new(0),
            answers: vec![Some(true), Some(true), Some(false)],
        };
        assert!(wait_session_gone(&seat, 975, Duration::from_secs(2)));
        assert_eq!(seat.calls.get(), 3);
        let unknown = Gone {
            calls: Cell::new(0),
            answers: vec![None],
        };
        assert!(wait_session_gone(&unknown, 975, Duration::from_secs(2)));
        let stuck = Gone {
            calls: Cell::new(0),
            answers: vec![Some(true); 1000],
        };
        assert!(!wait_session_gone(&stuck, 975, Duration::from_millis(120)));
    }
}
