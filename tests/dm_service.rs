//! `dojo-dm` as a real process, as the current user: a command stands in for the greeter's
//! Niri, this test is the greeter client, `dojo-dm-pam` runs against private PAM stacks, and
//! a script records the session. No console is switched, no logind is asked, and the
//! identity changes that need root are skipped, as `tests/dm_pam.rs` skips them. What a
//! real boot does differently is listed in `.scratch/login-screen/issues/04-dojo-dm-service.md`.

use std::collections::HashMap;
use std::ffi::CStr;
use std::fs;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use dojo::dm::protocol::{
    self, Event, Login, PowerOccasion, Request, Secret, SessionEntry, SessionFailure, SessionId,
};
use dojo::lock::pam::{Method, Notice, PamOutcome, Report};
use dojo::lock::secret::Password;

const FINGER_HINT: &str = "Place your finger on the reader (stand-in)";
const SECRET: &str = "fake-login-secret-0123456789";

const CHECK_PASSWORD: &str = r#"#!/bin/bash
IFS= read -r -d '' pw
[ "$pw" = "fake-login-secret-0123456789" ]
"#;

const HOOK: &str = r#"#!/bin/bash
case "$PAM_TYPE" in
  open_session) echo "open_session desktop=$XDG_SESSION_DESKTOP vtnr=$XDG_VTNR type=$XDG_SESSION_TYPE class=$XDG_SESSION_CLASS seat=$XDG_SEAT tty=$PAM_TTY" ;;
  *) echo "$PAM_TYPE" ;;
esac >> "$DIR/pam-phases.log"
"#;

const SESSION: &str = r#"#!/bin/bash
{
  echo "pid=$$"
  echo "cwd=$(pwd -P)"
  env
} > "$DIR/session.out.tmp"
mv "$DIR/session.out.tmp" "$DIR/session.out"
"#;

const SESSION_SHELL: &str = r#"#!/bin/sh
exec /bin/sh -c "$3"
"#;

const GREETER: &str = r#"#!/bin/sh
echo $$ > "$DIR/greeter.pid"
exec sleep 600
"#;

const DESKTOP: &str = "[Desktop Entry]\n\
    Name=Test Desktop\n\
    Name[de]=Testschreibtisch\n\
    Exec=$DIR/session.sh %U\n\
    DesktopNames=niri;GNOME;\n\
    Type=Application\n";

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

/// The user running the test, if the display manager would accept them.
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
        eprintln!("skipped: the current user is not an account dojo-dm accepts");
    }
    login
}

struct Bench {
    dir: tempfile::TempDir,
}

impl Bench {
    fn new() -> Bench {
        let dir = tempfile::Builder::new()
            .prefix("dm-svc-")
            .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
            .unwrap();
        let root = dir.path().to_str().unwrap().to_owned();
        for sub in ["pam.d", "tally", "sessions", "state"] {
            fs::create_dir(dir.path().join(sub)).unwrap();
        }
        for sub in ["sessions", "state"] {
            fs::set_permissions(dir.path().join(sub), fs::Permissions::from_mode(0o755)).unwrap();
        }
        for (method, stack) in [
            (Method::Fingerprint, finger_stack()),
            (Method::Password, password_stack()),
        ] {
            fs::write(
                dir.path().join("pam.d").join(method.login_service()),
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
            ("greeter.sh", GREETER),
        ] {
            let path = dir.path().join(name);
            fs::write(&path, body.replace("$DIR", &root)).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let entry = dir.path().join("sessions/test-desktop.desktop");
        fs::write(&entry, DESKTOP.replace("$DIR", &root)).unwrap();
        fs::set_permissions(&entry, fs::Permissions::from_mode(0o644)).unwrap();
        Bench { dir }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn dm(&self, extra: &[&str], greeter: &[&str]) -> Dm {
        let log = fs::File::create(self.path("dm.log")).unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_dojo-dm"))
            .arg("--socket")
            .arg(self.path("greeter.sock"))
            .arg("--state")
            .arg(self.path("state"))
            .arg("--sessions")
            .arg(self.path("sessions"))
            .arg("--helper")
            .arg(env!("CARGO_BIN_EXE_dojo-dm-pam"))
            .arg("--pam-confdir")
            .arg(self.path("pam.d"))
            .arg("--session-shell")
            .arg(self.path("shell.sh"))
            .args(extra)
            .arg("--greeter-command")
            .args(greeter)
            .env_clear()
            .env("PATH", "/usr/local/bin:/usr/bin")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()
            .unwrap();
        Dm {
            child: Some(child),
            log: self.path("dm.log"),
        }
    }

    fn standard_dm(&self) -> Dm {
        let greeter = self.path("greeter.sh");
        self.dm(&[], &[greeter.to_str().unwrap()])
    }

    fn phases(&self) -> Vec<String> {
        fs::read_to_string(self.path("pam-phases.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn wait_for(&self, what: &str, ready: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !ready() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn session_env(&self) -> HashMap<String, String> {
        self.wait_for("the session to run", || self.path("session.out").exists());
        fs::read_to_string(self.path("session.out"))
            .unwrap()
            .lines()
            .filter_map(|line| line.split_once('='))
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect()
    }
}

struct Dm {
    child: Option<Child>,
    log: PathBuf,
}

impl Dm {
    fn signal(&self, signal: i32) {
        let pid = self.child.as_ref().unwrap().id() as i32;
        assert_eq!(unsafe { libc::kill(pid, signal) }, 0);
    }

    fn exit_code(&mut self, within: Duration) -> Option<i32> {
        let deadline = Instant::now() + within;
        let child = self.child.as_mut().unwrap();
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                self.child = None;
                return status.code();
            }
            assert!(
                Instant::now() < deadline,
                "dojo-dm did not exit; its log:\n{}",
                fs::read_to_string(&self.log).unwrap_or_default()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn terminate(mut self) -> Option<i32> {
        self.signal(libc::SIGTERM);
        self.exit_code(Duration::from_secs(20))
    }
}

impl Drop for Dm {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
            if std::thread::panicking() {
                eprintln!(
                    "dojo-dm log:\n{}",
                    fs::read_to_string(&self.log).unwrap_or_default()
                );
            }
        }
    }
}

/// The test as the greeter: one connection, speaking the protocol.
struct Greeter {
    socket: OwnedFd,
}

impl Greeter {
    fn connect(path: &std::path::Path) -> Option<Greeter> {
        use std::os::unix::ffi::OsStrExt as _;
        let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
        address.sun_family = libc::AF_UNIX as _;
        for (slot, byte) in address.sun_path.iter_mut().zip(path.as_os_str().as_bytes()) {
            *slot = *byte as libc::c_char;
        }
        let fd =
            unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC, 0) };
        assert!(fd >= 0);
        let socket = unsafe { OwnedFd::from_raw_fd(fd) };
        let connected = unsafe {
            libc::connect(
                socket.as_raw_fd(),
                (&address as *const libc::sockaddr_un).cast(),
                size_of::<libc::sockaddr_un>() as libc::socklen_t,
            )
        };
        if connected != 0 {
            return None;
        }
        let timeout = libc::timeval {
            tv_sec: 15,
            tv_usec: 0,
        };
        unsafe {
            libc::setsockopt(
                socket.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_RCVTIMEO,
                (&timeout as *const libc::timeval).cast(),
                size_of::<libc::timeval>() as libc::socklen_t,
            )
        };
        Some(Greeter { socket })
    }

    /// Connects, says hello and returns the `Init`, retrying while the display manager is
    /// between greeters and drops the connection.
    fn hello(bench: &Bench) -> (Greeter, Event) {
        let deadline = Instant::now() + Duration::from_secs(25);
        while Instant::now() < deadline {
            if let Some(greeter) = Greeter::connect(&bench.path("greeter.sock")) {
                greeter.send(Request::Hello {
                    version: protocol::VERSION,
                });
                let mut buffer = [0u8; protocol::MAX_DATAGRAM];
                let length = unsafe {
                    libc::recv(
                        greeter.socket.as_raw_fd(),
                        buffer.as_mut_ptr().cast(),
                        buffer.len(),
                        0,
                    )
                };
                if length > 0 {
                    use protocol::Message as _;
                    return (greeter, Event::decode(&buffer[..length as usize]).unwrap());
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("no greeter connection was served");
    }

    fn send(&self, request: Request) {
        protocol::send(self.socket.as_fd(), &request).unwrap();
    }

    fn next(&self) -> Event {
        protocol::receive::<Event>(self.socket.as_fd())
            .expect("no event within 15 s")
            .expect("the display manager closed the connection")
            .unwrap()
    }

    /// `true` when the display manager closed the connection.
    fn closed(&self) -> bool {
        matches!(protocol::receive::<Event>(self.socket.as_fd()), Ok(None))
    }

    fn begin(&self, generation: u32, method: Method, login: &Login, session: &str) {
        self.send(Request::Begin {
            generation,
            method,
            login: login.clone(),
            session: SessionId::parse(session).unwrap(),
        });
    }

    fn answer(&self, generation: u32, text: &str) {
        let mut password = Password::new();
        text.chars().for_each(|c| assert!(password.push(c)));
        self.send(Request::Answer {
            generation,
            secret: Secret::new(password),
        });
    }

    fn report(&self, generation: u32, method: Method) -> Report {
        match self.next() {
            Event::Report {
                generation: g,
                method: m,
                report,
            } if (g, m) == (generation, method) => report,
            other => panic!("expected a report for {generation}, got {other:?}"),
        }
    }
}

fn init_sessions() -> Vec<SessionEntry> {
    vec![SessionEntry {
        id: SessionId::parse("test-desktop").unwrap(),
        name: "Test Desktop".into(),
    }]
}

#[test]
fn a_login_runs_the_session_and_the_greeter_comes_back() {
    let Some(login) = login_or_skip() else { return };
    let bench = Bench::new();
    let dm = bench.standard_dm();

    let (greeter, init) = Greeter::hello(&bench);
    assert_eq!(
        init,
        Event::Init {
            last_login: None,
            last_session: None,
            sessions: init_sessions()
        }
    );
    greeter.begin(1, Method::Password, &login, "test-desktop");
    assert_eq!(
        greeter.report(1, Method::Password),
        Report::Prompt { echo: false }
    );
    greeter.answer(1, SECRET);
    assert_eq!(
        greeter.report(1, Method::Password),
        Report::Finished(PamOutcome::Success)
    );
    greeter.send(Request::StartSession { generation: 1 });
    assert!(
        greeter.closed(),
        "the greeter is torn down before the session starts"
    );

    let env = bench.session_env();
    assert_eq!(env["USER"], login.as_str());
    assert_eq!(env["XDG_SESSION_TYPE"], "wayland");
    assert_eq!(env["XDG_SESSION_CLASS"], "user");
    assert_eq!(env["XDG_SESSION_DESKTOP"], "test-desktop");
    assert_eq!(env["XDG_CURRENT_DESKTOP"], "niri:GNOME");
    assert_eq!(env["XDG_SEAT"], "seat0");
    assert_eq!(env["XDG_VTNR"], "1");
    let home = dojo::dm::account::lookup(&login).unwrap().home;
    assert_eq!(
        env["cwd"],
        fs::canonicalize(home).unwrap().to_str().unwrap()
    );

    let (again, init) = Greeter::hello(&bench);
    assert_eq!(
        init,
        Event::Init {
            last_login: Some(login.clone()),
            last_session: Some(SessionId::parse("test-desktop").unwrap()),
            sessions: init_sessions()
        },
        "the greeter returns, remembering who logged in and what they chose"
    );
    assert_eq!(
        bench.phases(),
        [
            "account",
            "open_session desktop=test-desktop vtnr=1 type=wayland class=user seat=seat0 tty=tty1",
            "close_session"
        ]
    );
    let user = bench.path("state/users").join(login.as_str());
    assert!(
        user.is_dir(),
        "a look snapshot was stored for {}",
        login.as_str()
    );

    let greeter_pid: i32 = fs::read_to_string(bench.path("greeter.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    drop(again);
    assert_eq!(dm.terminate(), Some(0));
    assert!(
        !bench.path("greeter.sock").exists(),
        "the socket is removed"
    );
    bench.wait_for("the greeter to be stopped", || unsafe {
        libc::kill(greeter_pid, 0) != 0
    });
}

#[test]
fn failures_races_and_unknown_sessions_leave_the_greeter_usable() {
    let Some(login) = login_or_skip() else { return };
    let bench = Bench::new();
    let dm = bench.standard_dm();
    let (greeter, _) = Greeter::hello(&bench);

    greeter.begin(1, Method::Password, &login, "test-desktop");
    assert_eq!(
        greeter.report(1, Method::Password),
        Report::Prompt { echo: false }
    );
    greeter.answer(1, "wrong");
    assert_eq!(
        greeter.report(1, Method::Password),
        Report::Finished(PamOutcome::Rejected)
    );

    greeter.begin(2, Method::Password, &login, "no-such-session");
    assert_eq!(
        greeter.report(2, Method::Password),
        Report::Prompt { echo: false }
    );
    greeter.answer(2, SECRET);
    assert_eq!(
        greeter.report(2, Method::Password),
        Report::Finished(PamOutcome::Success)
    );
    greeter.send(Request::StartSession { generation: 2 });
    assert_eq!(
        greeter.next(),
        Event::SessionFailed {
            reason: SessionFailure::UnknownSession
        }
    );

    greeter.begin(3, Method::Fingerprint, &login, "test-desktop");
    greeter.begin(4, Method::Password, &login, "test-desktop");
    let mut finished = false;
    while !finished {
        match greeter.next() {
            Event::Report {
                generation: 3,
                method: Method::Fingerprint,
                report: Report::Notice(Notice::Hint { text, .. }),
            } => assert_eq!(text, FINGER_HINT),
            Event::Report {
                generation: 3,
                report: Report::Finished(PamOutcome::Success),
                ..
            } => finished = true,
            Event::Report {
                generation: 4,
                report: Report::Prompt { .. },
                ..
            } => {}
            other => panic!("unexpected {other:?}"),
        }
    }
    // The loser of the race is already gone; the greeter's cancel for it is no cause for complaint.
    greeter.send(Request::Cancel { generation: 4 });
    greeter.send(Request::StartSession { generation: 3 });
    assert!(greeter.closed());
    let env = bench.session_env();
    assert_eq!(env["XDG_SESSION_DESKTOP"], "test-desktop");

    let (_again, _) = Greeter::hello(&bench);
    assert_eq!(
        bench.phases(),
        [
            "account",
            "account",
            "open_session desktop=test-desktop vtnr=1 type=wayland class=user seat=seat0 tty=tty1",
            "close_session"
        ],
        "the loser of the race never reached the account phase"
    );
    let log = fs::read_to_string(bench.path("dm.log")).unwrap();
    assert!(
        !log.contains("ignored a request"),
        "a cancel for an attempt that is already over is not logged:\n{log}"
    );
    assert_eq!(dm.terminate(), Some(0));
}

#[test]
fn a_session_that_cannot_be_created_is_told_to_the_next_greeter_once() {
    let Some(login) = login_or_skip() else { return };
    let bench = Bench::new();
    let greeter_command = bench.path("greeter.sh");
    let dm = bench.dm(
        &["--session-shell", "/nonexistent/shell"],
        &[greeter_command.to_str().unwrap()],
    );
    let (greeter, _) = Greeter::hello(&bench);
    greeter.begin(1, Method::Password, &login, "test-desktop");
    assert_eq!(
        greeter.report(1, Method::Password),
        Report::Prompt { echo: false }
    );
    greeter.answer(1, SECRET);
    assert_eq!(
        greeter.report(1, Method::Password),
        Report::Finished(PamOutcome::Success)
    );
    greeter.send(Request::StartSession { generation: 1 });
    assert!(greeter.closed(), "the greeter is gone before the session is tried");

    let (again, init) = Greeter::hello(&bench);
    assert!(matches!(init, Event::Init { .. }));
    assert_eq!(
        again.next(),
        Event::SessionFailed {
            reason: SessionFailure::SpawnFailed
        },
        "the reason follows the fresh Init"
    );
    assert!(
        !bench.path("session.out").exists(),
        "no session ran"
    );

    drop(again);
    let (third, _) = Greeter::hello(&bench);
    third.send(Request::SelectUser {
        login: login.clone(),
    });
    assert!(
        matches!(third.next(), Event::User { .. }),
        "a reason is told once: the next thing the third greeter hears is its own answer"
    );
    assert_eq!(dm.terminate(), Some(0));
}

impl Greeter {
    /// A password login that succeeds, up to the point where the session would be started.
    fn authenticate(&self, generation: u32, login: &Login) {
        self.begin(generation, Method::Password, login, "test-desktop");
        assert_eq!(
            self.report(generation, Method::Password),
            Report::Prompt { echo: false }
        );
        self.answer(generation, SECRET);
        assert_eq!(
            self.report(generation, Method::Password),
            Report::Finished(PamOutcome::Success)
        );
    }
}

#[test]
fn a_greeter_that_leaves_processes_behind_never_hands_over_to_a_session() {
    let Some(login) = login_or_skip() else { return };
    let bench = Bench::new();
    let survivor = bench.path("survivor");
    fs::write(&survivor, "").unwrap();
    let greeter_command = bench.path("greeter.sh");
    let dm = bench.dm(
        &["--greeter-survivor", survivor.to_str().unwrap()],
        &[greeter_command.to_str().unwrap()],
    );
    let (greeter, _) = Greeter::hello(&bench);
    greeter.authenticate(1, &login);
    greeter.send(Request::StartSession { generation: 1 });
    assert!(greeter.closed());

    let (again, init) = Greeter::hello(&bench);
    assert!(matches!(init, Event::Init { .. }));
    assert_eq!(
        again.next(),
        Event::SessionFailed {
            reason: SessionFailure::Refused
        },
        "the fresh greeter is told the session was refused"
    );
    assert!(!bench.path("session.out").exists(), "no session ran");
    assert_eq!(bench.phases(), ["account"], "no PAM session was opened");

    fs::remove_file(&survivor).unwrap();
    again.authenticate(2, &login);
    again.send(Request::StartSession { generation: 2 });
    assert!(again.closed());
    assert_eq!(bench.session_env()["USER"], login.as_str());
    assert_eq!(dm.terminate(), Some(0));
}

#[test]
fn users_fingerprints_and_power_are_answered_and_requests_before_hello_are_ignored() {
    let Some(login) = login_or_skip() else { return };
    let bench = Bench::new();
    let power_log = bench.path("power.log");
    let greeter_command = bench.path("greeter.sh");
    let dm = bench.dm(
        &[
            "--fingerprint",
            login.as_str(),
            "--power-log",
            power_log.to_str().unwrap(),
        ],
        &[greeter_command.to_str().unwrap()],
    );

    let mut connected = None;
    let deadline = Instant::now() + Duration::from_secs(20);
    while connected.is_none() && Instant::now() < deadline {
        connected = Greeter::connect(&bench.path("greeter.sock"));
        std::thread::sleep(Duration::from_millis(20));
    }
    let greeter = connected.expect("the display manager is listening");
    greeter.send(Request::SelectUser {
        login: login.clone(),
    });
    greeter.send(Request::Power {
        occasion: PowerOccasion::Suspend,
    });
    greeter.send(Request::Hello {
        version: protocol::VERSION,
    });
    assert!(matches!(greeter.next(), Event::Init { .. }));
    assert!(!power_log.exists(), "power before hello did nothing");

    greeter.send(Request::SelectUser {
        login: login.clone(),
    });
    assert_eq!(
        greeter.next(),
        Event::User {
            login: login.clone(),
            display_name: None,
            fingerprint: true
        }
    );
    let stranger = Login::parse("nobody-has-this-name").unwrap();
    greeter.send(Request::SelectUser {
        login: stranger.clone(),
    });
    assert_eq!(
        greeter.next(),
        Event::User {
            login: stranger,
            display_name: None,
            fingerprint: false
        }
    );

    greeter.send(Request::Power {
        occasion: PowerOccasion::Shutdown,
    });
    bench.wait_for("the power action", || power_log.exists());
    assert_eq!(fs::read_to_string(&power_log).unwrap(), "PowerOff\n");
    assert_eq!(dm.terminate(), Some(0));
}

#[test]
fn the_login_screen_may_only_suspend_while_someone_else_is_logged_in() {
    let Some(login) = login_or_skip() else { return };
    let bench = Bench::new();
    let power_log = bench.path("power.log");
    let greeter_command = bench.path("greeter.sh");
    let dm = bench.dm(
        &["--other-session", "--power-log", power_log.to_str().unwrap()],
        &[greeter_command.to_str().unwrap()],
    );
    let (greeter, _) = Greeter::hello(&bench);
    for occasion in [
        PowerOccasion::Shutdown,
        PowerOccasion::Restart,
        PowerOccasion::Hibernate,
        PowerOccasion::Suspend,
    ] {
        greeter.send(Request::Power { occasion });
    }
    bench.wait_for("the suspend", || power_log.exists());
    bench.wait_for("three refusals in the log", || {
        fs::read_to_string(bench.path("dm.log")).is_ok_and(|log| log.matches("refused").count() == 3)
    });
    assert_eq!(fs::read_to_string(&power_log).unwrap(), "Suspend\n");

    greeter.send(Request::SelectUser { login: login.clone() });
    assert!(
        matches!(greeter.next(), Event::User { .. }),
        "the loop carries on after a refusal"
    );
    assert_eq!(dm.terminate(), Some(0));
}

#[test]
fn a_greeter_that_cannot_start_ends_the_service_after_three_tries() {
    let bench = Bench::new();
    let mut dm = bench.dm(&[], &["/bin/false"]);
    assert_eq!(dm.exit_code(Duration::from_secs(30)), Some(69));
    assert!(!bench.path("greeter.sock").exists());
}

#[test]
fn a_command_that_does_not_exist_is_a_failed_start_too() {
    let bench = Bench::new();
    let mut dm = bench.dm(&[], &["/no/such/greeter"]);
    assert_eq!(dm.exit_code(Duration::from_secs(30)), Some(69));
}

#[test]
fn a_run_without_the_test_options_refuses_to_start() {
    let status = Command::new(env!("CARGO_BIN_EXE_dojo-dm"))
        .arg("--bogus")
        .env_clear()
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(78));
}

#[test]
fn nothing_the_display_manager_runs_names_devenv_or_a_home_directory() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut files = vec![
        root.join("src/bin/dojo-dm.rs"),
        root.join("src/bin/dojo-dm-pam.rs"),
        root.join("system-config/systemd/system/dojo-dm.service"),
        root.join("system-config/dojo-dm/greeter.kdl"),
    ];
    for entry in fs::read_dir(root.join("src/dm")).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(fs::read_dir(&path).unwrap().map(|e| e.unwrap().path()));
        } else {
            files.push(path);
        }
    }
    for file in files {
        let text = fs::read_to_string(&file).unwrap();
        assert!(!text.contains("devenv"), "{} names devenv", file.display());
    }
    let unit =
        fs::read_to_string(root.join("system-config/systemd/system/dojo-dm.service")).unwrap();
    assert!(unit.contains("ExecStart=/usr/local/lib/dojo-dm/current/run"));
    assert!(!unit.contains("/home"));
}
