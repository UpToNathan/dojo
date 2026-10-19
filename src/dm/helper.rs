//! The `dojo-dm-pam` process: one login attempt for the user the display manager names.
//!
//! argv is the service name (`dojo-login` or `dojo-login-fingerprint`) and nothing else;
//! the socket is fd 3; the user is the first datagram, checked against passwd here and
//! never on a command line. The helper authenticates, reports, and waits. On `Start` it
//! becomes the session leader: credentials, session, a child that drops to the user and
//! runs the session through the login shell, then it closes everything when that ends.
//!
//! [`serve`] holds the lifecycle and runs against the traits below, so its order is
//! tested without root. The `Handle`, `SocketPeer` and `ForkLauncher` implementations
//! are the only parts that need a real stack, a real peer and root.

use std::ffi::{CString, OsString, c_int};
use std::io;
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::Duration;

use zeroize::Zeroizing;

use super::account::{self, Account};
use super::launch::{self, SessionEvent, Start};
use super::protocol::SessionFailure;
use super::session::{self, Service, SessionPlan};
use super::stack::Pam;
use crate::lock::worker::helper::{
    Conversation, EXIT_SOFTWARE, EXIT_UNAVAILABLE, EXIT_USAGE, converse, prepare_process,
    run_check, take_socket,
};
use crate::lock::pam::Method;
use crate::lock::worker::libpam::{LibPam, PAM_AUTH_ERR, PAM_SUCCESS, PamConv};
use crate::lock::worker::wire;

/// What the helper says to the display manager.
pub trait Peer {
    /// The PAM result. Nothing from PAM crosses the socket after this.
    fn finished(&mut self, code: c_int) -> io::Result<()>;
    fn next_start(&mut self) -> Waited;
    fn session(&mut self, event: SessionEvent) -> io::Result<()>;
    /// Prompts for a password and drops the answer, zeroed, unread.
    fn ask_and_discard(&mut self) -> io::Result<()>;
}

#[derive(Debug, PartialEq)]
pub enum Waited {
    Start(Box<Start>),
    /// The display manager closed the socket: it picked another attempt or went away.
    Gone,
    Invalid,
}

/// Starts the session process and waits for it.
pub trait Launcher {
    fn spawn(&mut self, plan: &SessionPlan) -> io::Result<()>;
    fn wait(&mut self);
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Exit {
    Clean,
    Software,
}

impl Exit {
    pub fn code(self) -> i32 {
        match self {
            Exit::Clean => 0,
            Exit::Software => EXIT_SOFTWARE,
        }
    }
}

/// The whole life of one attempt, in PAM's required order. `revalidate` looks the account
/// up again when the start arrives: the passwd entry may have changed since the prompt.
pub fn serve(
    pam: &mut impl Pam,
    peer: &mut impl Peer,
    launcher: &mut impl Launcher,
    account: &Account,
    shell: Option<PathBuf>,
    revalidate: impl FnOnce() -> Option<Account>,
) -> Exit {
    let mut code = pam.authenticate();
    if code == PAM_SUCCESS {
        code = pam.account();
    }
    let sent = peer.finished(code);
    if code != PAM_SUCCESS || sent.is_err() {
        pam.end(code);
        return if sent.is_ok() {
            Exit::Clean
        } else {
            Exit::Software
        };
    }
    let start = match peer.next_start() {
        Waited::Start(start) => start,
        Waited::Gone => {
            pam.end(PAM_SUCCESS);
            return Exit::Clean;
        }
        Waited::Invalid => {
            pam.end(PAM_SUCCESS);
            return Exit::Software;
        }
    };

    if revalidate().as_ref() != Some(account) {
        let _ = peer.session(SessionEvent::Failed(SessionFailure::Refused));
        pam.end(PAM_SUCCESS);
        return Exit::Clean;
    }
    let Some(environment) = open_session(pam, account, &start) else {
        let _ = peer.session(SessionEvent::Failed(SessionFailure::Refused));
        pam.end(PAM_SUCCESS);
        return Exit::Clean;
    };
    let plan = session::plan(account, &start, environment, shell);
    if launcher.spawn(&plan).is_err() {
        pam.close_session();
        pam.delete_credentials();
        let _ = peer.session(SessionEvent::Failed(SessionFailure::SpawnFailed));
        pam.end(PAM_SUCCESS);
        return Exit::Clean;
    }
    let _ = peer.session(SessionEvent::Started);
    launcher.wait();
    pam.close_session();
    pam.delete_credentials();
    let _ = peer.session(SessionEvent::Ended);
    pam.end(PAM_SUCCESS);
    Exit::Clean
}

/// Credentials, then the identity `pam_systemd` reads, then the session. Undoes the
/// credentials when the session does not open.
fn open_session(
    pam: &mut impl Pam,
    account: &Account,
    start: &Start,
) -> Option<Vec<(OsString, OsString)>> {
    if pam.establish_credentials() != PAM_SUCCESS {
        return None;
    }
    let mut code = pam.set_tty(&start.tty);
    for (name, value) in session::pam_variables(account, start) {
        if code != PAM_SUCCESS {
            break;
        }
        code = pam.put_environment(name, &value);
    }
    if code == PAM_SUCCESS {
        code = pam.open_session();
    }
    if code != PAM_SUCCESS {
        pam.delete_credentials();
        return None;
    }
    Some(session::parse_pam_environment(pam.environment()))
}

struct Invocation {
    service: Service,
    #[cfg(feature = "fake-pam-stack")]
    confdir: Option<CString>,
    #[cfg(feature = "fake-pam-stack")]
    shell: Option<PathBuf>,
}

enum Mode {
    Check,
    Serve(Invocation),
}

/// `--check` is accepted only alone; everything else is the service name:
/// `dojo-login`, `dojo-login-fingerprint` or the greeter's `dojo-greeter`.
/// Test builds also accept `--pam-confdir DIR` and `--session-shell PATH` before it.
fn parse_command(arguments: Vec<OsString>) -> Option<Mode> {
    if arguments.len() == 1 && arguments[0] == "--check" {
        return Some(Mode::Check);
    }
    parse_arguments(arguments).map(Mode::Serve)
}

fn parse_arguments(arguments: Vec<OsString>) -> Option<Invocation> {
    let arguments: Vec<&str> = arguments
        .iter()
        .map(|argument| argument.to_str())
        .collect::<Option<_>>()?;
    let (service, options) = arguments.split_last()?;
    #[allow(unused_mut)]
    let mut invocation = Invocation {
        service: Service::from_name(service)?,
        #[cfg(feature = "fake-pam-stack")]
        confdir: None,
        #[cfg(feature = "fake-pam-stack")]
        shell: None,
    };
    #[cfg(feature = "fake-pam-stack")]
    {
        let mut rest = options;
        while let [flag, value, tail @ ..] = rest {
            match *flag {
                "--pam-confdir" => invocation.confdir = Some(CString::new(*value).ok()?),
                "--session-shell" => invocation.shell = Some(PathBuf::from(value)),
                _ => return None,
            }
            rest = tail;
        }
        if !rest.is_empty() {
            return None;
        }
    }
    #[cfg(not(feature = "fake-pam-stack"))]
    if !options.is_empty() {
        return None;
    }
    Some(invocation)
}

/// The body of `src/bin/dojo-dm-pam.rs`.
pub fn run_helper() -> ! {
    let command = parse_command(std::env::args_os().skip(1).collect());
    if let Some(Mode::Check) = command {
        run_check();
    }
    prepare_process();
    let Some(Mode::Serve(invocation)) = command else {
        std::process::exit(EXIT_USAGE);
    };
    let Some(socket) = take_socket() else {
        std::process::exit(EXIT_USAGE);
    };
    let Some(login) = read_login(socket.as_fd()) else {
        std::process::exit(EXIT_USAGE);
    };
    let Ok(libpam) = LibPam::open() else {
        std::process::exit(EXIT_UNAVAILABLE);
    };

    let conversation = Conversation::new(socket.as_fd());
    let mut peer = SocketPeer {
        conversation: &conversation,
    };
    let service = invocation.service;
    let validate = || match service {
        Service::Login(_) => account::Account::validate(
            &login,
            account::lookup(&login),
            &account::read_shells(),
            account::is_executable_file,
        ),
        Service::Greeter => account::Account::validate_greeter(&login, account::lookup(&login)),
    };
    let Ok(account) = validate() else {
        std::process::exit(refuse_unknown(service, &mut peer, std::thread::sleep).code());
    };

    let service = CString::new(invocation.service.name()).expect("service names have no NUL");
    let conv = PamConv {
        conv: converse,
        appdata_ptr: (&conversation as *const Conversation).cast_mut().cast(),
    };
    let started = libpam.start(
        &service,
        &account.name,
        &conv,
        #[cfg(feature = "fake-pam-stack")]
        invocation.confdir.as_deref(),
    );
    let mut handle = match started {
        Ok(handle) => handle,
        Err(code) => {
            let sent = peer.finished(code);
            std::process::exit(if sent.is_ok() { 0 } else { EXIT_SOFTWARE });
        }
    };
    #[cfg(feature = "fake-pam-stack")]
    let shell = invocation.shell;
    #[cfg(not(feature = "fake-pam-stack"))]
    let shell = None;
    let mut launcher = ForkLauncher { child: None };
    let exit = serve(
        &mut handle,
        &mut peer,
        &mut launcher,
        &account,
        shell,
        || validate().ok(),
    );
    drop(handle);
    std::process::exit(exit.code());
}

/// A real account's wrong password waits out PAM's fail delay (about two seconds) before it
/// is refused. An account that is unknown, a system account or without a shell is refused
/// after the same wait, so the time to a refusal does not say which names exist.
pub const UNKNOWN_ACCOUNT_DELAY: Duration = Duration::from_secs(2);

/// Unknown, system and shell-less accounts look like a failed login. For a password that
/// includes the prompt, which the screen waits for before it can show a refusal, and the delay.
fn refuse_unknown(
    service: Service,
    peer: &mut impl Peer,
    sleep: impl FnOnce(Duration),
) -> Exit {
    if service == Service::Login(Method::Password) {
        if peer.ask_and_discard().is_err() {
            return Exit::Software;
        }
        sleep(UNKNOWN_ACCOUNT_DELAY);
    }
    if peer.finished(PAM_AUTH_ERR).is_ok() {
        Exit::Clean
    } else {
        Exit::Software
    }
}

/// Asks the display manager for a password and drops it, zeroed, unread.
fn ask_and_discard(socket: BorrowedFd<'_>) -> io::Result<()> {
    wire::send(socket, &wire::prompt(false))?;
    wire::Answer::receive(socket).map(drop)
}

fn read_login(socket: BorrowedFd<'_>) -> Option<super::protocol::Login> {
    let mut buffer = [0u8; 2 + super::protocol::MAX_LOGIN];
    let length = wire::receive(socket, &mut buffer).ok()?;
    launch::decode_login(&buffer[..length]).ok()
}

struct SocketPeer<'a> {
    conversation: &'a Conversation<'a>,
}

impl Peer for SocketPeer<'_> {
    fn finished(&mut self, code: c_int) -> io::Result<()> {
        let sent = wire::send(self.conversation.socket, &wire::done(code));
        self.conversation.silence();
        sent
    }

    fn next_start(&mut self) -> Waited {
        let mut buffer = Zeroizing::new(vec![0u8; launch::MAX_DATAGRAM]);
        loop {
            let received = wire::receive(self.conversation.socket, &mut buffer);
            let length = match received {
                Ok(0) => return Waited::Gone,
                Ok(length) => length,
                Err(error) if error.raw_os_error() == Some(libc::ECONNRESET) => {
                    return Waited::Gone;
                }
                Err(_) => return Waited::Invalid,
            };
            // An answer sent just as PAM finished is still queued. It is not for us.
            if buffer[0] == b'A' {
                zeroize::Zeroize::zeroize(&mut buffer[..length]);
                continue;
            }
            return match Start::decode(&buffer[..length]) {
                Ok(start) => Waited::Start(Box::new(start)),
                Err(_) => Waited::Invalid,
            };
        }
    }

    fn session(&mut self, event: SessionEvent) -> io::Result<()> {
        wire::send(
            self.conversation.socket,
            &launch::encode_session_event(event),
        )
    }

    fn ask_and_discard(&mut self) -> io::Result<()> {
        ask_and_discard(self.conversation.socket)
    }
}

/// Forks the session as the user. Needs root to change identity; a test build run
/// as the target user itself skips the identity change, which would fail without root.
struct ForkLauncher {
    child: Option<Child>,
}

/// The core limit the session gets. The helper itself holds 0/0 so it never dumps a password,
/// and a session must not inherit that: its hard limit would be zero and could never be
/// raised. Unless `pam_limits` set something while the session opened, the session gets what
/// a systemd service gets by default, a soft limit of zero under an unlimited hard one.
fn session_core_limit(held: libc::rlimit) -> libc::rlimit {
    if held.rlim_cur == 0 && held.rlim_max == 0 {
        libc::rlimit {
            rlim_cur: 0,
            rlim_max: libc::RLIM_INFINITY,
        }
    } else {
        held
    }
}

/// Makes every inherited descriptor above stderr close on exec. `std` keeps its own pipe for
/// reporting a failed exec open until then, so closing them outright would hide that failure.
fn close_inherited_descriptors() -> io::Result<()> {
    const CLOSE_RANGE_CLOEXEC: libc::c_uint = 4;
    let rc = unsafe {
        libc::syscall(
            libc::SYS_close_range,
            3 as libc::c_uint,
            libc::c_uint::MAX,
            CLOSE_RANGE_CLOEXEC,
        )
    };
    if rc < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

impl ForkLauncher {
    fn drops_identity(plan: &SessionPlan) -> io::Result<bool> {
        let euid = unsafe { libc::geteuid() };
        if euid == 0 {
            return Ok(true);
        }
        if cfg!(feature = "fake-pam-stack") && euid == plan.uid {
            return Ok(false);
        }
        Err(io::Error::from(io::ErrorKind::PermissionDenied))
    }
}

impl Launcher for ForkLauncher {
    fn spawn(&mut self, plan: &SessionPlan) -> io::Result<()> {
        let drop_identity = Self::drops_identity(plan)?;
        let home = CString::new(plan.home.as_os_str().as_encoded_bytes())
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        let account = plan.account.clone();
        let (uid, gid) = (plan.uid, plan.gid);
        let mut held = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        unsafe { libc::getrlimit(libc::RLIMIT_CORE, &mut held) };
        let core = session_core_limit(held);
        let mut command = Command::new(&plan.shell);
        command
            .args(&plan.arguments)
            .env_clear()
            .envs(plan.environment.iter().map(|(k, v)| (k, v)));
        // The session outlives the display manager's interest in the helper. A user's
        // orphans belong to init, not to this process; the greeter's stay here, where the
        // display manager's tree snapshot finds them when it stops the greeter.
        unsafe {
            libc::prctl(libc::PR_SET_PDEATHSIG, 0, 0, 0, 0);
            if !plan.own_orphans {
                libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 0, 0, 0, 0);
            }
            command.pre_exec(move || {
                if libc::setsid() < 0 {
                    return Err(io::Error::last_os_error());
                }
                close_inherited_descriptors()?;
                libc::setrlimit(libc::RLIMIT_CORE, &core);
                if drop_identity
                    && (libc::initgroups(account.as_ptr(), gid) < 0
                        || libc::setgid(gid) < 0
                        || libc::setuid(uid) < 0)
                {
                    return Err(io::Error::last_os_error());
                }
                if drop_identity && uid != 0 && libc::setuid(0) == 0 {
                    return Err(io::Error::from(io::ErrorKind::PermissionDenied));
                }
                if libc::chdir(home.as_ptr()) < 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        self.child = Some(command.spawn()?);
        Ok(())
    }

    fn wait(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.wait();
        }
    }
}

#[cfg(test)]
mod tests;
