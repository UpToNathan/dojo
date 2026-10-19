//! The fakes a `fake-pam-stack` build of `dojo-dm` can be started with: private
//! directories, a plain command in place of the greeter's Niri, a console that switches
//! nothing, a power log in place of logind, and a fixed list of enrolled fingerprints.
//! Never part of a release build.

use std::ffi::OsString;
use std::io::{self, Write as _};
use std::os::fd::{AsFd, BorrowedFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt as _;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

use super::{Parts, Settings, Unusable};
use crate::dm::greeter::{GreeterProcess, Greeters};
use crate::dm::host::{Fingerprints, Power, Seat, logind_method};
use crate::dm::process::HelperSpec;
use crate::dm::protocol::{Login, PowerOccasion};
use crate::dm::vt::{Console, Vt};
use crate::lock::worker::reaper;

#[derive(Debug, PartialEq, Eq)]
pub struct Options {
    pub socket: PathBuf,
    pub state: PathBuf,
    pub sessions: PathBuf,
    pub sessions_owner: u32,
    pub helper: PathBuf,
    pub confdir: Option<PathBuf>,
    pub session_shell: Option<PathBuf>,
    pub vt: u8,
    pub power_log: Option<PathBuf>,
    /// While this file exists the fake greeter reports that its processes survived being stopped.
    pub greeter_survivor: Option<PathBuf>,
    pub fingerprints: Vec<String>,
    pub greeter: Vec<OsString>,
}

pub fn parse(arguments: Vec<OsString>) -> Result<Options, Unusable> {
    let mut arguments = arguments.into_iter();
    let mut options = Options {
        socket: PathBuf::new(),
        state: PathBuf::new(),
        sessions: PathBuf::new(),
        sessions_owner: unsafe { libc::geteuid() },
        helper: PathBuf::new(),
        confdir: None,
        session_shell: None,
        vt: 1,
        power_log: None,
        greeter_survivor: None,
        fingerprints: Vec::new(),
        greeter: Vec::new(),
    };
    let bad = |what: &str| Unusable(format!("test option: {what}"));
    while let Some(flag) = arguments.next() {
        let flag = flag.into_string().map_err(|_| bad("not UTF-8"))?;
        if flag == "--greeter-command" {
            options.greeter = arguments.by_ref().collect();
            break;
        }
        let value = arguments
            .next()
            .ok_or_else(|| bad(&format!("{flag} needs a value")))?;
        let text = || {
            value
                .to_str()
                .map(str::to_owned)
                .ok_or_else(|| bad("not UTF-8"))
        };
        match flag.as_str() {
            "--socket" => options.socket = value.into(),
            "--state" => options.state = value.into(),
            "--sessions" => options.sessions = value.into(),
            "--sessions-owner" => {
                options.sessions_owner = text()?.parse().map_err(|_| bad("--sessions-owner"))?;
            }
            "--helper" => options.helper = value.into(),
            "--pam-confdir" => options.confdir = Some(value.into()),
            "--session-shell" => options.session_shell = Some(value.into()),
            "--vt" => options.vt = text()?.parse().map_err(|_| bad("--vt"))?,
            "--power-log" => options.power_log = Some(value.into()),
            "--greeter-survivor" => options.greeter_survivor = Some(value.into()),
            "--fingerprint" => options.fingerprints.push(text()?),
            other => return Err(bad(&format!("unknown {other}"))),
        }
    }
    let present = [
        &options.socket,
        &options.state,
        &options.sessions,
        &options.helper,
    ]
    .iter()
    .all(|path| !path.as_os_str().is_empty());
    if !present || options.greeter.is_empty() || Vt::new(options.vt).is_none() {
        return Err(bad(
            "--socket, --state, --sessions, --helper and --greeter-command are required",
        ));
    }
    Ok(options)
}

pub fn build(arguments: Vec<OsString>) -> Result<(Settings, Parts), Unusable> {
    let options = parse(arguments)?;
    let mut helper = HelperSpec::new(options.helper);
    helper.confdir = options.confdir;
    helper.session_shell = options.session_shell;
    let settings = Settings {
        socket: options.socket,
        state: options.state,
        sessions: options.sessions,
        sessions_owner: options.sessions_owner,
        helper,
    };
    let parts = Parts {
        console: Box::new(FakeConsole(Vt::new(options.vt).expect("checked"))),
        greeters: Box::new(CommandGreeters {
            command: options.greeter,
            survivor: options.greeter_survivor,
        }),
        power: Box::new(PowerLog(options.power_log)),
        seat: Box::new(NoSessions),
        fingerprints: Box::new(Enrolled(options.fingerprints)),
        greeter_group: None,
    };
    Ok((settings, parts))
}

struct FakeConsole(Vt);

impl Console for FakeConsole {
    fn vt(&self) -> Vt {
        self.0
    }

    fn acquire(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct PowerLog(Option<PathBuf>);

impl Power for PowerLog {
    fn perform(&self, occasion: PowerOccasion) -> Result<(), String> {
        let Some(path) = &self.0 else { return Ok(()) };
        let mut log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| e.to_string())?;
        writeln!(log, "{}", logind_method(occasion)).map_err(|e| e.to_string())
    }
}

struct NoSessions;

impl Seat for NoSessions {
    fn has_session_of(&self, _uid: u32) -> Option<bool> {
        Some(false)
    }
}

struct Enrolled(Vec<String>);

impl Fingerprints for Enrolled {
    fn enrolled(&mut self, login: &Login) -> bool {
        self.0.iter().any(|name| name == login.as_str())
    }
}

/// A command that stands in for the greeter's session. Whoever connects to the socket as
/// this user is the greeter; the command only has to stay alive until it is stopped.
struct CommandGreeters {
    command: Vec<OsString>,
    survivor: Option<PathBuf>,
}

impl Greeters for CommandGreeters {
    fn peer_uid(&self) -> u32 {
        unsafe { libc::geteuid() }
    }

    fn launch(&mut self, _vt: Vt) -> io::Result<Box<dyn GreeterProcess>> {
        let mut command = Command::new(&self.command[0]);
        command
            .args(&self.command[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        unsafe {
            command.pre_exec(|| {
                let mut unblocked: libc::sigset_t = std::mem::zeroed();
                libc::sigemptyset(&mut unblocked);
                if libc::sigprocmask(libc::SIG_SETMASK, &unblocked, std::ptr::null_mut()) < 0
                    || libc::setsid() < 0
                    || libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0) < 0
                {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command.spawn()?;
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, child.id() as libc::pid_t, 0) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let pidfd = unsafe { OwnedFd::from_raw_fd(fd as i32) };
        Ok(Box::new(CommandGreeter {
            child,
            pidfd,
            survivor: self.survivor.clone(),
        }))
    }
}

struct CommandGreeter {
    child: Child,
    pidfd: OwnedFd,
    survivor: Option<PathBuf>,
}

impl GreeterProcess for CommandGreeter {
    fn exit_fd(&self) -> BorrowedFd<'_> {
        self.pidfd.as_fd()
    }

    fn ended(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }

    fn stop(&mut self) -> bool {
        let victims = reaper::tree(&reaper::read_table(), self.child.id() as i32);
        let child = &mut self.child;
        let survivors = reaper::kill(&victims, || {
            let _ = child.try_wait();
        });
        let _ = self.child.wait();
        let sticky = self.survivor.as_ref().is_some_and(|marker| marker.exists());
        survivors.is_empty() && !sticky
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(Into::into).collect()
    }

    #[test]
    fn the_required_options_and_the_greeter_command_are_parsed() {
        let options = parse(args(&[
            "--socket",
            "/s",
            "--state",
            "/st",
            "--sessions",
            "/se",
            "--helper",
            "/h",
            "--vt",
            "3",
            "--fingerprint",
            "alyssa",
            "--fingerprint",
            "bob",
            "--pam-confdir",
            "/c",
            "--greeter-command",
            "sleep",
            "--vt",
            "600",
        ]))
        .unwrap();
        assert_eq!(options.vt, 3);
        assert_eq!(options.fingerprints, ["alyssa", "bob"]);
        assert_eq!(options.confdir, Some("/c".into()));
        assert_eq!(options.greeter, args(&["sleep", "--vt", "600"]));
    }

    #[test]
    fn missing_or_unknown_options_are_refused() {
        assert!(parse(args(&[])).is_err());
        assert!(parse(args(&["--socket", "/s"])).is_err());
        assert!(parse(args(&["--bogus", "x"])).is_err());
        let complete = [
            "--socket",
            "/s",
            "--state",
            "/t",
            "--sessions",
            "/e",
            "--helper",
            "/h",
        ];
        let mut without_command = args(&complete);
        assert!(parse(without_command.clone()).is_err());
        without_command.extend(args(&["--vt", "0", "--greeter-command", "x"]));
        assert!(parse(without_command).is_err(), "vt 0");
    }
}
