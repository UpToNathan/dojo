//! PAM workers as separate processes: spawn, answer, kill, and a report channel.
//! Mechanism only; deciding which worker should run is the caller's job.

pub(crate) mod helper;
pub(crate) mod libpam;
pub(crate) mod reaper;
pub(crate) mod wire;

use std::ffi::OsString;
use std::fmt;
use std::io;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use async_channel::Sender;

use super::pam::{Report, WorkerId};
use super::secret::Password;

pub use helper::run_helper;

/// Dojo binaries run on Nix's glibc, but the host's libpam and PAM modules need
/// the host's newer glibc. The helper therefore runs under the host's loader,
/// with the host's library directory ahead of the binary's Nix RUNPATH.
const HOST_LOADER: &str = "/usr/lib64/ld-linux-x86-64.so.2";
const HOST_LIBRARIES: &str = "/usr/lib";
const WORKER_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/bin";
pub(crate) const WORKER_FD: i32 = 3;

/// How to start the `dojo-lock-pam` helper.
#[derive(Clone, Debug)]
pub struct Helper {
    program: PathBuf,
    #[cfg(feature = "fake-pam-stack")]
    confdir: Option<PathBuf>,
}

impl Helper {
    pub fn new(program: PathBuf) -> Helper {
        Helper {
            program,
            #[cfg(feature = "fake-pam-stack")]
            confdir: None,
        }
    }

    /// Run the service files from `confdir` instead of /etc/pam.d. Test builds only.
    #[cfg(feature = "fake-pam-stack")]
    pub fn with_confdir(mut self, confdir: PathBuf) -> Helper {
        self.confdir = Some(confdir);
        self
    }

    fn arguments(&self, id: WorkerId) -> Vec<OsString> {
        let mut arguments = vec![
            "--library-path".into(),
            HOST_LIBRARIES.into(),
            self.program.clone().into(),
        ];
        #[cfg(feature = "fake-pam-stack")]
        if let Some(confdir) = &self.confdir {
            arguments.push("--pam-confdir".into());
            arguments.push(confdir.clone().into());
        }
        arguments.push(id.method.service().into());
        arguments
    }

    /// Starts one PAM conversation. Its reports arrive on `reports` tagged with `id`.
    /// PDEATHSIG fires when the spawning thread exits, so spawn from a long-lived thread.
    pub fn spawn(&self, id: WorkerId, reports: &Sender<(WorkerId, Report)>) -> io::Result<Worker> {
        let (parent_end, child_end) = socketpair()?;
        let child_fd = child_end.as_raw_fd();
        let locker = unsafe { libc::getpid() };
        let mut command = Command::new(HOST_LOADER);
        command
            .args(self.arguments(id))
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
                if moved < 0
                    || libc::setsid() < 0
                    || libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0) < 0
                    || libc::setrlimit(libc::RLIMIT_CORE, &no_core) < 0
                {
                    return Err(io::Error::last_os_error());
                }
                if libc::getppid() != locker {
                    libc::_exit(1);
                }
                Ok(())
            });
        }
        let child = command.spawn()?;
        drop(child_end);

        let socket = Arc::new(parent_end);
        let silenced = Arc::new(Mutex::new(false));
        let reader = std::thread::Builder::new()
            .name(format!("pam-{:?}-{}", id.method, id.generation))
            .spawn({
                let socket = socket.clone();
                let silenced = silenced.clone();
                let reports = reports.clone();
                move || read_reports(id, &socket, &silenced, &reports)
            })?;
        Ok(Worker {
            id,
            child,
            socket,
            silenced,
            reader: Some(reader),
            victims: None,
        })
    }
}

/// A running PAM conversation. Dropping it kills it.
pub struct Worker {
    id: WorkerId,
    child: Child,
    socket: Arc<OwnedFd>,
    silenced: Arc<Mutex<bool>>,
    reader: Option<JoinHandle<()>>,
    victims: Option<Vec<reaper::Process>>,
}

impl Worker {
    pub fn id(&self) -> WorkerId {
        self.id
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Sends the password as one datagram; the password is zeroed when this returns.
    /// An answer sent before the prompt waits in the socket until PAM asks.
    pub fn answer(&self, password: Password) -> io::Result<()> {
        wire::send_answer(self.socket.as_fd(), &password)
    }

    /// Kills the worker and every descendant, snapshotted before any signal.
    /// No report for this worker is delivered once this starts. Returns the pids
    /// still alive after SIGKILL, so an empty list means the tree is gone.
    /// Idempotent: a later call retries only the survivors.
    pub fn kill(&mut self) -> Vec<u32> {
        *self
            .silenced
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
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
            if let Some(reader) = self.reader.take() {
                let _ = reader.join();
            }
        }
        survivors.iter().map(|process| process.pid as u32).collect()
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.kill();
    }
}

impl fmt::Debug for Worker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Worker")
            .field("id", &self.id)
            .field("pid", &self.pid())
            .finish_non_exhaustive()
    }
}

fn read_reports(
    id: WorkerId,
    socket: &OwnedFd,
    silenced: &Mutex<bool>,
    reports: &Sender<(WorkerId, Report)>,
) {
    let mut buffer = [0u8; wire::MAX_DATAGRAM];
    loop {
        let report = match wire::receive(socket.as_fd(), &mut buffer) {
            // A worker that exits with an unread answer resets the socket. The error is
            // reported once, ahead of datagrams still queued, so read on for its result.
            Err(error) if error.raw_os_error() == Some(libc::ECONNRESET) => continue,
            Ok(0) | Err(_) => Report::Crashed,
            Ok(length) => wire::decode_report(&buffer[..length]).unwrap_or(Report::Crashed),
        };
        let last = matches!(report, Report::Finished(_) | Report::Crashed);
        let silenced = silenced
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if *silenced {
            return;
        }
        let _ = reports.try_send((id, report));
        if last {
            return;
        }
    }
}

fn socketpair() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0; 2];
    let rc = unsafe {
        libc::socketpair(
            libc::AF_UNIX,
            libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC,
            0,
            fds.as_mut_ptr(),
        )
    };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}
