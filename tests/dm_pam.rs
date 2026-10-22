//! `dojo-dm-pam` as a real process against private `pam_start_confdir` stacks, run as the
//! current user. Stand-in stacks only. The helper's identity change needs root and is not
//! exercised here: a test build run as the target user skips `initgroups`/`setgid`/`setuid`
//! and runs everything else, including `pam_setcred`, `pam_open_session`, `pam_getenvlist`,
//! the new session, its environment and `pam_close_session`.

use std::collections::HashMap;
use std::ffi::CStr;
use std::fs;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use dojo::dm::launch::{HelperEvent, SessionEvent, Start, decode_helper_event, encode_login};
use dojo::dm::protocol::{Login, SessionFailure, SessionId};
use dojo::lock::pam::{Method, Notice, PamOutcome, Report};

const HOST_LOADER: &str = "/usr/lib64/ld-linux-x86-64.so.2";
const FINGER_HINT: &str = "Place your finger on the reader (stand-in)";
const SECRET: &str = "fake-login-secret-0123456789";

const CHECK_PASSWORD: &str = r#"#!/bin/bash
IFS= read -r -d '' pw
[ "$pw" = "fake-login-secret-0123456789" ]
"#;

const HOOK: &str = r#"#!/bin/bash
# Records which PAM phase ran, and what the session phases could see.
case "$PAM_TYPE" in
  open_session) echo "open_session desktop=$XDG_SESSION_DESKTOP vtnr=$XDG_VTNR type=$XDG_SESSION_TYPE class=$XDG_SESSION_CLASS seat=$XDG_SEAT tty=$PAM_TTY" ;;
  *) echo "$PAM_TYPE" ;;
esac >> "$DIR/pam-phases.log"
"#;

const SESSION: &str = r#"#!/bin/bash
read -r _ _ _ _ _ sid _ < /proc/$$/stat
{
  echo "pid=$$"
  echo "sid=$sid"
  echo "cwd=$(pwd -P)"
  env
} > "$DIR/session.out"
"#;

const SESSION_SHELL: &str = r#"#!/bin/sh
# Stands in for the user's login shell, so no profile of this machine is read.
printf '%s\n' "$@" > "$DIR/shell.args"
exec /bin/sh -c "$3"
"#;

const ACCOUNT_AND_SESSION: &str = "account required pam_exec.so quiet $DIR/hook.sh\n\
     session required pam_exec.so quiet $DIR/hook.sh\n";

fn password_stack() -> String {
    format!(
        "auth [success=1 default=ignore] pam_exec.so quiet expose_authtok $DIR/check_password.sh\n\
         auth requisite pam_deny.so\n\
         auth required pam_permit.so\n{ACCOUNT_AND_SESSION}"
    )
}

fn finger_stack() -> String {
    format!(
        "auth optional pam_echo.so file=$DIR/finger.txt\nauth required pam_permit.so\n{ACCOUNT_AND_SESSION}"
    )
}

const DENY: &str = "auth required pam_deny.so\naccount required pam_permit.so\n";

/// The account the helper will accept: the user running this test, if they are a human
/// with an acceptable shell. Anything else makes these tests meaningless, so they skip.
fn test_login() -> Option<Login> {
    let passwd = unsafe { libc::getpwuid(libc::getuid()) };
    if passwd.is_null() {
        return None;
    }
    let name = unsafe { CStr::from_ptr((*passwd).pw_name) }.to_str().ok()?;
    let login = Login::parse(name).ok()?;
    let entry = dojo::dm::account::lookup(&login)?;
    dojo::dm::account::Account::validate(
        &login,
        Some(entry),
        &dojo::dm::account::read_shells(),
        dojo::dm::account::is_executable_file,
    )
    .ok()
    .map(|_| login)
}

fn login_or_skip() -> Option<Login> {
    let login = test_login();
    if login.is_none() {
        eprintln!("skipped: the current user is not an account dojo-dm-pam accepts");
    }
    login
}

struct Bench {
    dir: tempfile::TempDir,
}

impl Bench {
    fn new(fingerprint: &str, password: &str) -> Bench {
        let dir = tempfile::Builder::new()
            .prefix("dm-pam-")
            .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
            .unwrap();
        let root = dir.path().to_str().unwrap().to_owned();
        let confdir = dir.path().join("pam.d");
        fs::create_dir(&confdir).unwrap();
        fs::create_dir(dir.path().join("tally")).unwrap();
        for (method, stack) in [
            (Method::Fingerprint, fingerprint),
            (Method::Password, password),
        ] {
            fs::write(
                confdir.join(method.login_service()),
                stack.replace("$DIR", &root),
            )
            .unwrap();
        }
        fs::write(dir.path().join("finger.txt"), FINGER_HINT).unwrap();
        for (name, body) in [
            ("check_password.sh", CHECK_PASSWORD),
            ("hook.sh", HOOK),
            ("session.sh", SESSION),
            ("shell.sh", SESSION_SHELL),
        ] {
            let path = dir.path().join(name);
            fs::write(&path, body.replace("$DIR", &root)).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        Bench { dir }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn spawn(&self, method: Method) -> Run {
        self.spawn_with(method, &[])
    }

    fn spawn_with(&self, method: Method, extra: &[&str]) -> Run {
        self.spawn_service(method.login_service(), extra)
    }

    fn spawn_service(&self, service: &str, extra: &[&str]) -> Run {
        let (parent, child_end) = socketpair();
        let child_fd = child_end.as_raw_fd();
        let timeout = libc::timeval {
            tv_sec: 15,
            tv_usec: 0,
        };
        unsafe {
            libc::setsockopt(
                parent.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_RCVTIMEO,
                (&timeout as *const libc::timeval).cast(),
                size_of::<libc::timeval>() as libc::socklen_t,
            )
        };
        let mut command = Command::new(HOST_LOADER);
        command
            .args(["--library-path", "/usr/lib"])
            .arg(env!("CARGO_BIN_EXE_dojo-dm-pam"))
            .arg("--pam-confdir")
            .arg(self.path("pam.d"))
            .arg("--session-shell")
            .arg(self.path("shell.sh"))
            .args(extra)
            .arg(service)
            .env_clear()
            .env("LC_MESSAGES", "C.UTF-8")
            .env("PATH", "/usr/local/sbin:/usr/local/bin:/usr/bin")
            .env("DOJO_LEAK", "from-the-helper-environment")
            .stdin(Stdio::null())
            .stdout(Stdio::null());
        unsafe {
            command.pre_exec(move || {
                let moved = if child_fd == 3 {
                    libc::fcntl(3, libc::F_SETFD, 0)
                } else {
                    libc::dup2(child_fd, 3)
                };
                if moved < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command.spawn().unwrap();
        drop(child_end);
        Run {
            socket: parent,
            child,
        }
    }

    fn start(&self) -> Start {
        Start {
            exec: self.path("session.sh").to_str().unwrap().to_owned(),
            desktop: SessionId::parse("test-desktop").unwrap(),
            desktop_names: vec!["niri".into(), "GNOME".into()],
            vtnr: 1,
            tty: "tty1".into(),
        }
    }

    fn phases(&self) -> Vec<String> {
        fs::read_to_string(self.path("pam-phases.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn session_env(&self) -> HashMap<String, String> {
        fs::read_to_string(self.path("session.out"))
            .unwrap()
            .lines()
            .filter_map(|line| line.split_once('='))
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect()
    }
}

fn socketpair() -> (OwnedFd, OwnedFd) {
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

struct Run {
    socket: OwnedFd,
    child: Child,
}

impl Run {
    fn send(&self, datagram: &[u8]) {
        let sent = unsafe {
            libc::send(
                self.socket.as_raw_fd(),
                datagram.as_ptr().cast(),
                datagram.len(),
                libc::MSG_NOSIGNAL,
            )
        };
        assert_eq!(sent, datagram.len() as isize, "send failed");
    }

    fn login(&self, login: &Login) {
        self.send(&encode_login(login));
    }

    fn answer(&self, secret: &str) {
        let mut datagram = vec![b'A'];
        datagram.extend_from_slice(secret.as_bytes());
        self.send(&datagram);
    }

    fn start(&self, start: &Start) {
        self.send(&start.encode());
    }

    /// The next thing the helper said; `None` at end of stream.
    fn next(&self) -> Option<HelperEvent> {
        let mut buffer = [0u8; 1024];
        let length = unsafe {
            libc::recv(
                self.socket.as_raw_fd(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                0,
            )
        };
        assert!(length >= 0, "no datagram within 15 s");
        if length == 0 {
            return None;
        }
        Some(
            decode_helper_event(&buffer[..length as usize])
                .unwrap_or_else(|| panic!("undecodable datagram {:?}", &buffer[..length as usize])),
        )
    }

    fn expect(&self, expected: HelperEvent) {
        assert_eq!(self.next(), Some(expected));
    }

    fn expect_report(&self, report: Report) {
        self.expect(HelperEvent::Report(report));
    }

    /// Closes the socket as the display manager does for a loser, and returns the exit code.
    fn finish(mut self) -> Option<i32> {
        drop(self.socket);
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status.code();
            }
            if std::time::Instant::now() > deadline {
                let _ = self.child.kill();
                panic!("helper did not exit");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

fn hint(text: &str) -> Report {
    Report::Notice(Notice::Hint {
        text: text.into(),
        error: false,
    })
}

fn success() -> Report {
    Report::Finished(PamOutcome::Success)
}

#[test]
fn a_password_login_runs_the_whole_lifecycle_and_starts_the_session() {
    let Some(login) = login_or_skip() else { return };
    let bench = Bench::new(&finger_stack(), &password_stack());
    let run = bench.spawn(Method::Password);
    run.login(&login);
    run.expect_report(Report::Prompt { echo: false });
    run.answer(SECRET);
    run.expect_report(success());
    assert_eq!(
        bench.phases(),
        ["account"],
        "acct_mgmt ran before the result"
    );

    run.start(&bench.start());
    run.expect(HelperEvent::Session(SessionEvent::Started));
    run.expect(HelperEvent::Session(SessionEvent::Ended));
    assert_eq!(run.finish(), Some(0));

    assert_eq!(
        bench.phases(),
        [
            "account",
            "open_session desktop=test-desktop vtnr=1 type=wayland class=user seat=seat0 tty=tty1",
            "close_session"
        ]
    );
    let env = bench.session_env();
    let passwd = dojo::dm::account::lookup(&login).unwrap();
    assert_eq!(env["HOME"], passwd.home.to_str().unwrap());
    assert_eq!(env["USER"], login.as_str());
    assert_eq!(env["LOGNAME"], login.as_str());
    assert_eq!(env["SHELL"], passwd.shell.to_str().unwrap());
    assert_eq!(env["PATH"], "/usr/local/sbin:/usr/local/bin:/usr/bin");
    assert_eq!(env["XDG_SESSION_TYPE"], "wayland");
    assert_eq!(env["XDG_SESSION_CLASS"], "user");
    assert_eq!(env["XDG_SESSION_DESKTOP"], "test-desktop");
    assert_eq!(env["XDG_CURRENT_DESKTOP"], "niri:GNOME");
    assert_eq!(env["XDG_SEAT"], "seat0");
    assert_eq!(env["XDG_VTNR"], "1");
    assert!(
        !env.contains_key("DOJO_LEAK"),
        "the helper's environment leaked"
    );
    assert!(!env.contains_key("LC_MESSAGES"));
    assert_eq!(
        env["cwd"],
        fs::canonicalize(&passwd.home).unwrap().to_str().unwrap()
    );
    assert_eq!(
        env["sid"], env["pid"],
        "the session is its own session leader"
    );

    let args = fs::read_to_string(bench.path("shell.args")).unwrap();
    let args: Vec<&str> = args.lines().collect();
    assert_eq!(args[..2], ["-l", "-c"]);
    assert_eq!(
        args[2],
        format!("exec {}", bench.path("session.sh").display())
    );
    assert_eq!(args.len(), 3);
}

#[test]
fn a_wrong_password_is_rejected_and_the_helper_exits() {
    let Some(login) = login_or_skip() else { return };
    let bench = Bench::new(&finger_stack(), &password_stack());
    let run = bench.spawn(Method::Password);
    run.login(&login);
    run.expect_report(Report::Prompt { echo: false });
    run.answer("wrong");
    run.expect_report(Report::Finished(PamOutcome::Rejected));
    assert_eq!(run.next(), None);
    assert_eq!(run.finish(), Some(0));
    assert_eq!(
        bench.phases(),
        Vec::<String>::new(),
        "no account or session phase"
    );
}

#[test]
fn a_stack_that_denies_outright_fails_before_any_prompt() {
    let Some(login) = login_or_skip() else { return };
    let bench = Bench::new(DENY, DENY);
    let run = bench.spawn(Method::Password);
    run.login(&login);
    run.expect_report(Report::Finished(PamOutcome::Rejected));
    assert_eq!(run.finish(), Some(0));
}

#[test]
fn the_fingerprint_service_reports_its_hint_and_a_loser_leaves_without_a_session() {
    let Some(login) = login_or_skip() else { return };
    let bench = Bench::new(&finger_stack(), &password_stack());
    let run = bench.spawn(Method::Fingerprint);
    run.login(&login);
    run.expect_report(hint(FINGER_HINT));
    run.expect_report(success());
    assert_eq!(
        run.finish(),
        Some(0),
        "closing the socket releases the helper"
    );
    assert_eq!(bench.phases(), ["account"]);
}

#[test]
fn faillock_text_arrives_as_the_same_notices_the_lock_screen_reads() {
    let Some(login) = login_or_skip() else { return };
    let tally = "dir=$DIR/tally deny=3 unlock_time=600";
    let faillock = format!(
        "auth required pam_faillock.so preauth {tally}\n\
         auth [success=1 default=bad] pam_deny.so\n\
         auth [default=die] pam_faillock.so authfail {tally}\n\
         auth required pam_faillock.so authsucc {tally}\n\
         account required pam_permit.so\n"
    );
    let bench = Bench::new(DENY, &faillock);
    for _ in 0..3 {
        let run = bench.spawn(Method::Password);
        run.login(&login);
        run.expect_report(Report::Finished(PamOutcome::Rejected));
        assert_eq!(run.finish(), Some(0));
    }
    let locked = bench.spawn(Method::Password);
    locked.login(&login);
    locked.expect_report(Report::Notice(Notice::PasswordLocked));
    locked.expect_report(Report::Notice(Notice::PasswordPaused {
        remaining: Duration::from_secs(600),
    }));
    locked.expect_report(Report::Finished(PamOutcome::Rejected));
    assert_eq!(locked.finish(), Some(0));
}

#[test]
fn accounts_the_display_manager_would_not_start_are_rejected_like_a_failed_login() {
    if login_or_skip().is_none() {
        return;
    }
    let bench = Bench::new(&finger_stack(), &finger_stack());
    for name in ["root", "daemon", "no-such-user-dojo"] {
        // A password login asks first, so the screen sees what a wrong password looks like.
        let run = bench.spawn(Method::Password);
        run.login(&Login::parse(name).unwrap());
        run.expect_report(Report::Prompt { echo: false });
        run.answer(SECRET);
        run.expect_report(Report::Finished(PamOutcome::Rejected));
        assert_eq!(run.next(), None);
        assert_eq!(run.finish(), Some(0), "{name}");
        let run = bench.spawn(Method::Fingerprint);
        run.login(&Login::parse(name).unwrap());
        run.expect_report(Report::Finished(PamOutcome::Rejected));
        assert_eq!(run.finish(), Some(0), "{name}");
    }
    assert_eq!(
        bench.phases(),
        Vec::<String>::new(),
        "PAM never ran for them"
    );
}

#[test]
fn a_bad_first_datagram_is_a_usage_error() {
    let bench = Bench::new(&finger_stack(), &password_stack());
    // An empty datagram reads as end of stream.
    for first in [&b"Lroot\0"[..], b"LRoot", b"Anathan", b"", b"L../etc"] {
        let run = bench.spawn(Method::Password);
        run.send(first);
        assert_eq!(run.next(), None, "{first:?}");
        assert_eq!(run.finish(), Some(64), "{first:?}");
    }
}

#[test]
fn a_start_that_fails_validation_never_opens_a_session() {
    let Some(login) = login_or_skip() else { return };
    let bench = Bench::new(&finger_stack(), &password_stack());
    let run = bench.spawn(Method::Fingerprint);
    run.login(&login);
    run.expect_report(hint(FINGER_HINT));
    run.expect_report(success());
    run.send(b"Gnot a start");
    assert_eq!(run.next(), None);
    assert_eq!(run.finish(), Some(70));
    assert_eq!(bench.phases(), ["account"]);
}

#[test]
fn a_session_that_cannot_be_executed_is_reported_and_the_pam_session_is_closed() {
    let Some(login) = login_or_skip() else { return };
    let bench = Bench::new(&finger_stack(), &password_stack());
    let run = bench.spawn_with(
        Method::Fingerprint,
        &["--session-shell", "/nonexistent/shell"],
    );
    run.login(&login);
    run.expect_report(hint(FINGER_HINT));
    run.expect_report(success());
    run.start(&bench.start());
    run.expect(HelperEvent::Session(SessionEvent::Failed(
        SessionFailure::SpawnFailed,
    )));
    assert_eq!(run.finish(), Some(0));
    let phases = bench.phases();
    assert_eq!(phases.len(), 3, "{phases:?}");
    assert!(phases[1].starts_with("open_session"));
    assert_eq!(phases[2], "close_session");
}

#[test]
fn the_greeter_service_refuses_every_user_but_the_greeter() {
    let Some(login) = login_or_skip() else { return };
    let bench = Bench::new(&finger_stack(), &password_stack());
    for name in [login.as_str(), "root"] {
        let run = bench.spawn_service("dojo-greeter", &[]);
        run.login(&Login::parse(name).unwrap());
        run.expect_report(Report::Finished(PamOutcome::Rejected));
        assert_eq!(run.finish(), Some(0), "{name}");
    }
    assert_eq!(
        bench.phases(),
        Vec::<String>::new(),
        "PAM never ran for them"
    );
}

#[test]
fn the_login_services_refuse_the_greeter_user() {
    if login_or_skip().is_none() {
        return;
    }
    let bench = Bench::new(&finger_stack(), &finger_stack());
    for method in [Method::Password, Method::Fingerprint] {
        let run = bench.spawn(method);
        run.login(&Login::parse("dojo-greeter").unwrap());
        if method == Method::Password {
            run.expect_report(Report::Prompt { echo: false });
            run.answer(SECRET);
        }
        run.expect_report(Report::Finished(PamOutcome::Rejected));
        assert_eq!(run.finish(), Some(0));
    }
    assert_eq!(bench.phases(), Vec::<String>::new());
}

#[test]
fn argv_other_than_a_login_service_is_refused() {
    for arguments in [
        vec![],
        vec!["dojo-lock-password"],
        vec!["dojo-login", "nathan"],
        vec!["--pam-confdir", "/tmp"],
    ] {
        let status = Command::new(HOST_LOADER)
            .args(["--library-path", "/usr/lib"])
            .arg(env!("CARGO_BIN_EXE_dojo-dm-pam"))
            .args(&arguments)
            .env_clear()
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(64), "{arguments:?}");
    }
}

#[test]
fn check_loads_libpam_and_every_symbol_the_helper_uses() {
    let output = Command::new(HOST_LOADER)
        .args(["--library-path", "/usr/lib"])
        .arg(env!("CARGO_BIN_EXE_dojo-dm-pam"))
        .arg("--check")
        .env_clear()
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, b"ok\n");
}
