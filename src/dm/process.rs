//! `dojo-dm-pam` as a child process of the display manager: started under the host's
//! loader with a clean environment, spoken to over fd 3, and killed with everything it
//! started. The same mechanism as `lock::worker::Helper`, for the helper's other datagrams.

use std::ffi::OsString;
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use super::launch::{self, HelperEvent, Start};
use super::protocol::Login;
use super::session::Service;
use crate::lock::secret::Password;
use crate::lock::worker::reaper::{self, Process};
use crate::lock::worker::{HOST_LIBRARIES, HOST_LOADER, WORKER_FD, WORKER_PATH, socketpair, wire};

/// How to start the helper. Test builds can point it at a private PAM directory and shell.
#[derive(Clone, Debug)]
pub struct HelperSpec {
    program: PathBuf,
    #[cfg(feature = "fake-pam-stack")]
    pub confdir: Option<PathBuf>,
    #[cfg(feature = "fake-pam-stack")]
    pub session_shell: Option<PathBuf>,
}

impl HelperSpec {
    pub fn new(program: PathBuf) -> HelperSpec {
        HelperSpec {
            program,
            #[cfg(feature = "fake-pam-stack")]
            confdir: None,
            #[cfg(feature = "fake-pam-stack")]
            session_shell: None,
        }
    }

    /// Arguments to the host loader. The user is not among them: it travels as the
    /// first datagram, so no argv in the process table names who is logging in.
    pub fn arguments(&self, service: Service) -> Vec<OsString> {
        let mut arguments = vec![
            "--library-path".into(),
            HOST_LIBRARIES.into(),
            self.program.clone().into(),
        ];
        #[cfg(feature = "fake-pam-stack")]
        {
            if let Some(confdir) = &self.confdir {
                arguments.extend(["--pam-confdir".into(), confdir.clone().into()]);
            }
            if let Some(shell) = &self.session_shell {
                arguments.extend(["--session-shell".into(), shell.clone().into()]);
            }
        }
        arguments.push(service.name().into());
        arguments
    }
}

/// What came out of a helper's socket.
#[derive(Debug, PartialEq)]
pub enum Received {
    Event(HelperEvent),
    /// End of stream: the helper is gone.
    Closed,
    /// Something the helper must never send.
    Garbled,
}

/// A running helper. Dropping it kills it.
pub struct HelperProcess {
    child: Child,
    socket: OwnedFd,
    victims: Option<Vec<Process>>,
}

impl HelperProcess {
    /// Starts the helper and sends it `login`, which it checks against passwd itself.
    pub fn spawn(spec: &HelperSpec, service: Service, login: &Login) -> io::Result<HelperProcess> {
        let (parent_end, child_end) = socketpair()?;
        let child_fd = child_end.as_raw_fd();
        let manager = unsafe { libc::getpid() };
        let mut command = Command::new(HOST_LOADER);
        command
            .args(spec.arguments(service))
            .env_clear()
            .env("LC_MESSAGES", "C.UTF-8")
            .env("PATH", WORKER_PATH)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        unsafe {
            command.pre_exec(move || {
                let moved = if child_fd == WORKER_FD {
                    libc::fcntl(WORKER_FD, libc::F_SETFD, 0)
                } else {
                    libc::dup2(child_fd, WORKER_FD)
                };
                let no_core = libc::rlimit {
                    rlim_cur: 0,
                    rlim_max: 0,
                };
                let mut unblocked: libc::sigset_t = std::mem::zeroed();
                libc::sigemptyset(&mut unblocked);
                if moved < 0
                    || libc::sigprocmask(libc::SIG_SETMASK, &unblocked, std::ptr::null_mut()) < 0
                    || libc::setsid() < 0
                    || libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0) < 0
                    || libc::setrlimit(libc::RLIMIT_CORE, &no_core) < 0
                {
                    return Err(io::Error::last_os_error());
                }
                if libc::getppid() != manager {
                    libc::_exit(1);
                }
                Ok(())
            });
        }
        let child = command.spawn()?;
        drop(child_end);
        let process = HelperProcess {
            child,
            socket: parent_end,
            victims: None,
        };
        wire::send(process.socket.as_fd(), &launch::encode_login(login))?;
        Ok(process)
    }

    pub fn socket(&self) -> BorrowedFd<'_> {
        self.socket.as_fd()
    }

    /// One datagram. Call it when the socket is readable.
    pub fn receive(&self) -> Received {
        let mut buffer = [0u8; wire::MAX_DATAGRAM];
        loop {
            return match wire::receive(self.socket.as_fd(), &mut buffer) {
                // A helper that exits with an unread answer resets the socket. That is
                // reported once, ahead of datagrams still queued, so read on for them.
                Err(error) if error.raw_os_error() == Some(libc::ECONNRESET) => continue,
                Ok(0) | Err(_) => Received::Closed,
                Ok(length) => match launch::decode_helper_event(&buffer[..length]) {
                    Some(event) => Received::Event(event),
                    None => Received::Garbled,
                },
            };
        }
    }

    /// The password as one datagram; it is zeroed when this returns.
    pub fn answer(&self, password: Password) -> io::Result<()> {
        wire::send_answer(self.socket.as_fd(), &password)
    }

    pub fn start(&self, start: &Start) -> io::Result<()> {
        wire::send(self.socket.as_fd(), &start.encode())
    }

    pub fn has_exited(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }

    /// Waits for the process itself, which is not the same as its socket closing.
    pub fn wait_for_exit(&mut self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while !self.has_exited() {
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        true
    }

    /// Asks everything the helper started to stop, and leaves the helper to close the PAM
    /// session once its child is gone.
    pub fn terminate_session(&mut self) {
        let root = self.child.id() as i32;
        let tree = reaper::tree(&reaper::read_table(), root);
        for process in &tree {
            if process.pid != root {
                unsafe { libc::kill(process.pid, libc::SIGTERM) };
            }
        }
        // Remembered for a later `kill`, which must sweep what this tree leaves behind.
        self.victims.get_or_insert(tree);
    }

    /// Kills the helper and every descendant, snapshotted before any signal, and returns
    /// the pids still alive afterwards. A helper that already exited and was collected
    /// has nothing left to kill. Idempotent: a later call retries only the survivors.
    pub fn kill(&mut self) -> Vec<u32> {
        if self.victims.is_none() && self.has_exited() {
            return Vec::new();
        }
        let root = self.child.id() as i32;
        let victims = self
            .victims
            .get_or_insert_with(|| reaper::tree(&reaper::read_table(), root));
        let child = &mut self.child;
        let survivors = reaper::kill(victims, || {
            let _ = child.try_wait();
        });
        if survivors.iter().all(|process| process.pid != root) {
            let _ = self.child.wait();
            unsafe { libc::shutdown(self.socket.as_raw_fd(), libc::SHUT_RDWR) };
        }
        survivors.iter().map(|process| process.pid as u32).collect()
    }
}

impl Drop for HelperProcess {
    fn drop(&mut self) {
        self.kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lock::pam::Method;

    #[test]
    fn the_helper_runs_under_the_host_loader_with_only_the_service_on_argv() {
        let spec = HelperSpec::new("/usr/local/lib/dojo-dm/r/dojo-dm-pam".into());
        for (service, name) in [
            (Service::Login(Method::Password), "dojo-login"),
            (Service::Login(Method::Fingerprint), "dojo-login-fingerprint"),
            (Service::Greeter, "dojo-greeter"),
        ] {
            assert_eq!(
                spec.arguments(service),
                [
                    "--library-path",
                    "/usr/lib",
                    "/usr/local/lib/dojo-dm/r/dojo-dm-pam",
                    name
                ]
                .map(OsString::from)
            );
        }
    }

    #[cfg(feature = "fake-pam-stack")]
    #[test]
    fn test_builds_pass_the_private_stack_before_the_service() {
        let mut spec = HelperSpec::new("/x/dojo-dm-pam".into());
        spec.confdir = Some("/c".into());
        spec.session_shell = Some("/s".into());
        assert_eq!(
            spec.arguments(Service::Greeter)[3..],
            ["--pam-confdir", "/c", "--session-shell", "/s", "dojo-greeter"].map(OsString::from)
        );
    }
}
