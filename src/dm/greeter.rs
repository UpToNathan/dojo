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
                        uid: self.uid,
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
    uid: u32,
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
    /// what ends the logind session. What is still alive after the timeout is killed, and
    /// then so is anything else that runs as the greeter's user: a daemon that forked twice
    /// and called `setsid` is in no tree the helper can snapshot.
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
        let helper_gone = self.helper.kill().is_empty();
        evict(self.uid).is_empty() && helper_gone
    }
}

const EVICT_ROUNDS: u32 = 40;
const EVICT_PAUSE: Duration = Duration::from_millis(50);

/// The real uid in a `/proc/<pid>/status`, or `None` for a zombie or a dead process, which
/// hold nothing and cannot be killed any deader.
fn live_owner(status: &str) -> Option<u32> {
    let mut owner = None;
    for line in status.lines() {
        if let Some(state) = line.strip_prefix("State:") {
            if state.trim_start().starts_with(['Z', 'X', 'x']) {
                return None;
            }
        } else if let Some(ids) = line.strip_prefix("Uid:") {
            owner = ids.split_whitespace().next()?.parse().ok();
        }
    }
    owner
}

fn processes_of(uid: u32) -> Vec<i32> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| entry.file_name().to_str()?.parse::<i32>().ok())
        .filter(|pid| {
            std::fs::read_to_string(format!("/proc/{pid}/status"))
                .ok()
                .and_then(|status| live_owner(&status))
                == Some(uid)
        })
        .collect()
}

/// SIGKILLs every process of `uid` until none is left or the rounds run out, and returns
/// the pids still alive. Never touches root's processes.
pub fn evict(uid: u32) -> Vec<i32> {
    if uid == 0 {
        return Vec::new();
    }
    evict_with(
        || processes_of(uid),
        |pid| unsafe {
            libc::kill(pid, libc::SIGKILL);
        },
        std::thread::sleep,
    )
}

fn evict_with(
    mut living: impl FnMut() -> Vec<i32>,
    mut kill: impl FnMut(i32),
    mut pause: impl FnMut(Duration),
) -> Vec<i32> {
    for _ in 0..EVICT_ROUNDS {
        let found = living();
        if found.is_empty() {
            return found;
        }
        found.into_iter().for_each(&mut kill);
        pause(EVICT_PAUSE);
    }
    living()
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
    use std::sync::atomic::{AtomicUsize, Ordering};

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

    #[test]
    fn a_status_file_names_its_owner_unless_the_process_is_a_zombie() {
        let status = |state: &str| {
            format!("Name:\tsleep\nState:\t{state}\nPid:\t7\nUid:\t975\t975\t975\t975\nGid:\t975\t975\t975\t975\n")
        };
        assert_eq!(live_owner(&status("S (sleeping)")), Some(975));
        assert_eq!(live_owner(&status("R (running)")), Some(975));
        assert_eq!(live_owner(&status("Z (zombie)")), None);
        assert_eq!(live_owner(&status("X (dead)")), None);
        assert_eq!(live_owner("Name:\tx\nState:\tS (sleeping)\n"), None);
        assert_eq!(live_owner("Uid:\tnope\n"), None);
    }

    #[test]
    fn this_process_is_found_among_its_own_users_processes() {
        let me = std::process::id() as i32;
        let uid = unsafe { libc::getuid() };
        assert!(processes_of(uid).contains(&me));
        assert!(!processes_of(uid.wrapping_add(1_000_000)).contains(&me));
    }

    #[test]
    fn leftovers_are_killed_round_after_round_until_none_is_left() {
        let mut alive = vec![40, 41, 42];
        let mut killed = Vec::new();
        let mut pauses = 0;
        let table = std::cell::RefCell::new(&mut alive);
        let left = evict_with(
            || table.borrow().clone(),
            |pid| {
                killed.push(pid);
                // 41 forks a replacement the first time it is killed; 42 takes two rounds.
                let mut alive = table.borrow_mut();
                match pid {
                    41 if !killed[..killed.len() - 1].contains(&41) => {
                        alive.retain(|&p| p != 41);
                        alive.push(43);
                    }
                    42 if killed.iter().filter(|&&p| p == 42).count() < 2 => {}
                    _ => alive.retain(|&p| p != pid),
                }
            },
            |_| pauses += 1,
        );
        assert!(left.is_empty());
        assert_eq!(killed, [40, 41, 42, 42, 43]);
        assert_eq!(pauses, 2);
    }

    #[test]
    fn a_process_that_will_not_die_is_reported_after_the_rounds() {
        let mut pauses = 0;
        let left = evict_with(|| vec![9], |_| {}, |_| pauses += 1);
        assert_eq!(left, [9]);
        assert_eq!(pauses, EVICT_ROUNDS);
    }

    struct Gone {
        calls: AtomicUsize,
        answers: Vec<Option<bool>>,
    }

    impl Seat for Gone {
        fn has_session_of(&self, _uid: u32) -> Option<bool> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            self.answers.get(call).copied().unwrap_or(Some(true))
        }

        fn has_session_besides(&self, _uid: u32) -> Option<bool> {
            None
        }
    }

    #[test]
    fn the_seat_is_free_once_logind_forgets_the_greeter_or_cannot_say() {
        let seat = Gone {
            calls: AtomicUsize::new(0),
            answers: vec![Some(true), Some(true), Some(false)],
        };
        assert!(wait_session_gone(&seat, 975, Duration::from_secs(2)));
        assert_eq!(seat.calls.load(Ordering::SeqCst), 3);
        let unknown = Gone {
            calls: AtomicUsize::new(0),
            answers: vec![None],
        };
        assert!(wait_session_gone(&unknown, 975, Duration::from_secs(2)));
        let stuck = Gone {
            calls: AtomicUsize::new(0),
            answers: vec![Some(true); 1000],
        };
        assert!(!wait_session_gone(&stuck, 975, Duration::from_millis(120)));
    }
}
