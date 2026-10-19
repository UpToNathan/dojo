use super::*;
use crate::dm::protocol::{Login, SessionId};
use crate::lock::pam::Method;
use crate::lock::worker::libpam::PAM_CONV_ERR;
use std::cell::RefCell;
use std::rc::Rc;

type Log = Rc<RefCell<Vec<String>>>;

struct FakePam {
    log: Log,
    authenticate: c_int,
    account: c_int,
    establish: c_int,
    open: c_int,
}

impl FakePam {
    fn new(log: &Log) -> FakePam {
        FakePam {
            log: log.clone(),
            authenticate: 0,
            account: 0,
            establish: 0,
            open: 0,
        }
    }

    fn note(&self, line: impl Into<String>) {
        self.log.borrow_mut().push(line.into());
    }
}

impl Pam for FakePam {
    fn authenticate(&mut self) -> c_int {
        self.note("authenticate");
        self.authenticate
    }
    fn account(&mut self) -> c_int {
        self.note("acct_mgmt");
        self.account
    }
    fn establish_credentials(&mut self) -> c_int {
        self.note("setcred establish");
        self.establish
    }
    fn set_tty(&mut self, tty: &str) -> c_int {
        self.note(format!("tty {tty}"));
        0
    }
    fn put_environment(&mut self, name: &str, value: &str) -> c_int {
        self.note(format!("putenv {name}={value}"));
        0
    }
    fn open_session(&mut self) -> c_int {
        self.note("open_session");
        self.open
    }
    fn environment(&mut self) -> Vec<Vec<u8>> {
        self.note("getenvlist");
        vec![b"XDG_RUNTIME_DIR=/run/user/1000".to_vec()]
    }
    fn close_session(&mut self) -> c_int {
        self.note("close_session");
        0
    }
    fn delete_credentials(&mut self) -> c_int {
        self.note("setcred delete");
        0
    }
    fn end(&mut self, status: c_int) {
        self.note(format!("end {status}"));
    }
}

struct FakePeer {
    log: Log,
    wait: Option<Waited>,
    send_fails: bool,
}

impl Peer for FakePeer {
    fn finished(&mut self, code: c_int) -> io::Result<()> {
        self.log.borrow_mut().push(format!("finished {code}"));
        if self.send_fails {
            Err(io::ErrorKind::BrokenPipe.into())
        } else {
            Ok(())
        }
    }
    fn next_start(&mut self) -> Waited {
        self.log.borrow_mut().push("wait for start".into());
        self.wait.take().unwrap_or(Waited::Gone)
    }
    fn session(&mut self, event: SessionEvent) -> io::Result<()> {
        self.log.borrow_mut().push(format!("session {event:?}"));
        Ok(())
    }
}

struct FakeLauncher {
    log: Log,
    fails: bool,
    plans: Vec<SessionPlan>,
}

impl Launcher for FakeLauncher {
    fn spawn(&mut self, plan: &SessionPlan) -> io::Result<()> {
        self.log.borrow_mut().push("spawn".into());
        self.plans.push(plan.clone());
        if self.fails {
            Err(io::ErrorKind::NotFound.into())
        } else {
            Ok(())
        }
    }
    fn wait(&mut self) {
        self.log.borrow_mut().push("session exits".into());
    }
}

fn account() -> Account {
    Account {
        login: Login::parse("nathan").unwrap(),
        name: c"nathan".into(),
        uid: 1000,
        gid: 1000,
        home: "/home/nathan".into(),
        shell: "/usr/bin/fish".into(),
    }
}

fn start() -> Start {
    Start {
        exec: "uwsm start -F -- niri".into(),
        desktop: SessionId::parse("niri-uwsm").unwrap(),
        desktop_names: vec!["niri".into()],
        vtnr: 1,
        tty: "tty1".into(),
    }
}

struct Run {
    log: Vec<String>,
    exit: Exit,
    plans: Vec<SessionPlan>,
}

fn run(
    configure: impl FnOnce(&mut FakePam, &mut FakePeer, &mut FakeLauncher),
    wait: Waited,
) -> Run {
    let log: Log = Rc::default();
    let mut pam = FakePam::new(&log);
    let mut peer = FakePeer {
        log: log.clone(),
        wait: Some(wait),
        send_fails: false,
    };
    let mut launcher = FakeLauncher {
        log: log.clone(),
        fails: false,
        plans: vec![],
    };
    configure(&mut pam, &mut peer, &mut launcher);
    let exit = serve(&mut pam, &mut peer, &mut launcher, &account(), None);
    let log = log.borrow().clone();
    Run {
        log,
        exit,
        plans: launcher.plans,
    }
}

#[test]
fn the_greeter_session_is_class_greeter_and_runs_the_given_command() {
    let log: Log = Rc::default();
    let mut pam = FakePam::new(&log);
    let mut peer = FakePeer {
        log: log.clone(),
        wait: Some(Waited::Start(Box::new(Start {
            exec: "exec /usr/local/lib/dojo-dm/r/niri".into(),
            desktop: SessionId::parse("dojo-greeter").unwrap(),
            desktop_names: vec![],
            vtnr: 1,
            tty: "tty1".into(),
        }))),
        send_fails: false,
    };
    let mut launcher = FakeLauncher {
        log: log.clone(),
        fails: false,
        plans: vec![],
    };
    let greeter = Account {
        login: Login::parse("dojo-greeter").unwrap(),
        name: c"dojo-greeter".into(),
        uid: 975,
        gid: 975,
        home: "/".into(),
        shell: "/bin/sh".into(),
    };
    assert_eq!(
        serve(&mut pam, &mut peer, &mut launcher, &greeter, None),
        Exit::Clean
    );
    assert!(
        log.borrow()
            .contains(&"putenv XDG_SESSION_CLASS=greeter".to_owned())
    );
    assert_eq!(launcher.plans[0].shell, PathBuf::from("/bin/sh"));
    assert_eq!(
        launcher.plans[0].arguments,
        ["-c", "exec /usr/local/lib/dojo-dm/r/niri"].map(OsString::from)
    );
}

fn go() -> Waited {
    Waited::Start(Box::new(start()))
}

#[test]
fn a_session_runs_in_pams_required_order() {
    let run = run(|_, _, _| {}, go());
    assert_eq!(
        run.log,
        [
            "authenticate",
            "acct_mgmt",
            "finished 0",
            "wait for start",
            "setcred establish",
            "tty tty1",
            "putenv XDG_SESSION_TYPE=wayland",
            "putenv XDG_SESSION_CLASS=user",
            "putenv XDG_SESSION_DESKTOP=niri-uwsm",
            "putenv XDG_SEAT=seat0",
            "putenv XDG_VTNR=1",
            "open_session",
            "getenvlist",
            "spawn",
            "session Started",
            "session exits",
            "close_session",
            "setcred delete",
            "session Ended",
            "end 0",
        ]
    );
    assert_eq!(run.exit, Exit::Clean);
}

#[test]
fn the_session_gets_pams_environment_and_the_chosen_command() {
    let run = run(|_, _, _| {}, go());
    let plan = &run.plans[0];
    assert!(
        plan.environment
            .contains(&("XDG_RUNTIME_DIR".into(), "/run/user/1000".into()))
    );
    assert_eq!(plan.arguments[2], "exec uwsm start -F -- niri");
    assert_eq!(plan.shell, PathBuf::from("/usr/bin/fish"));
}

#[test]
fn a_wrong_password_reports_and_ends_without_waiting() {
    let run = run(|pam, _, _| pam.authenticate = 7, go());
    assert_eq!(run.log, ["authenticate", "finished 7", "end 7"]);
    assert_eq!(run.exit, Exit::Clean);
}

#[test]
fn an_account_pam_refuses_is_reported_as_the_failure() {
    let run = run(|pam, _, _| pam.account = 13, go());
    assert_eq!(
        run.log,
        ["authenticate", "acct_mgmt", "finished 13", "end 13"]
    );
}

#[test]
fn a_loser_whose_socket_closes_never_opens_a_session() {
    let run = run(|_, _, _| {}, Waited::Gone);
    assert_eq!(
        run.log,
        [
            "authenticate",
            "acct_mgmt",
            "finished 0",
            "wait for start",
            "end 0"
        ]
    );
    assert_eq!(run.exit, Exit::Clean);
}

#[test]
fn a_malformed_start_ends_the_helper_without_a_session() {
    let run = run(|_, _, _| {}, Waited::Invalid);
    assert!(!run.log.iter().any(|line| line.contains("session")));
    assert_eq!(run.exit, Exit::Software);
}

#[test]
fn a_lost_result_ends_the_helper() {
    let run = run(|_, peer, _| peer.send_fails = true, go());
    assert_eq!(
        run.log,
        ["authenticate", "acct_mgmt", "finished 0", "end 0"]
    );
    assert_eq!(run.exit, Exit::Software);
}

#[test]
fn refused_credentials_report_a_refused_session() {
    let run = run(|pam, _, _| pam.establish = PAM_CONV_ERR, go());
    assert_eq!(
        run.log[3..],
        [
            "wait for start",
            "setcred establish",
            "session Failed(Refused)",
            "end 0"
        ]
    );
}

#[test]
fn a_session_pam_will_not_open_deletes_the_credentials_and_never_closes() {
    let run = run(|pam, _, _| pam.open = 4, go());
    assert!(run.log.contains(&"open_session".to_owned()));
    assert!(!run.log.contains(&"close_session".to_owned()));
    assert!(!run.log.contains(&"spawn".to_owned()));
    assert_eq!(
        run.log[run.log.len() - 3..],
        ["setcred delete", "session Failed(Refused)", "end 0"]
    );
}

#[test]
fn a_session_that_cannot_be_spawned_is_closed_again() {
    let run = run(|_, _, launcher| launcher.fails = true, go());
    assert_eq!(
        run.log[run.log.len() - 5..],
        [
            "spawn",
            "close_session",
            "setcred delete",
            "session Failed(SpawnFailed)",
            "end 0"
        ]
    );
}

fn parse(arguments: &[&str]) -> Option<Service> {
    parse_arguments(arguments.iter().map(Into::into).collect()).map(|invocation| invocation.service)
}

#[test]
fn argv_is_exactly_one_known_service() {
    assert_eq!(
        parse(&["dojo-login"]),
        Some(Service::Login(Method::Password))
    );
    assert_eq!(
        parse(&["dojo-login-fingerprint"]),
        Some(Service::Login(Method::Fingerprint))
    );
    assert_eq!(parse(&["dojo-greeter"]), Some(Service::Greeter));
    assert_eq!(parse(&["dojo-greeter", "dojo-greeter"]), None);
    assert_eq!(parse(&[]), None);
    assert_eq!(parse(&["dojo-lock-password"]), None);
    assert_eq!(parse(&["sudo"]), None);
    assert_eq!(parse(&["dojo-login", "nathan"]), None);
    assert_eq!(parse(&["nathan", "dojo-login"]), None);
}

#[test]
fn check_is_only_accepted_alone() {
    let command = |arguments: &[&str]| parse_command(arguments.iter().map(Into::into).collect());
    assert!(matches!(command(&["--check"]), Some(Mode::Check)));
    assert!(command(&["--check", "dojo-login"]).is_none());
    assert!(command(&["dojo-login", "--check"]).is_none());
    assert!(command(&["--Check"]).is_none());
    assert!(matches!(command(&["dojo-login"]), Some(Mode::Serve(_))));
}

#[cfg(not(feature = "fake-pam-stack"))]
#[test]
fn release_builds_refuse_test_options() {
    assert_eq!(parse(&["--pam-confdir", "/tmp/x", "dojo-login"]), None);
    assert_eq!(parse(&["--session-shell", "/bin/sh", "dojo-login"]), None);
}

#[cfg(feature = "fake-pam-stack")]
#[test]
fn test_builds_accept_a_private_confdir_and_shell_before_the_service() {
    assert_eq!(
        parse(&[
            "--pam-confdir",
            "/tmp/x",
            "--session-shell",
            "/bin/sh",
            "dojo-login"
        ]),
        Some(Service::Login(Method::Password))
    );
    assert_eq!(parse(&["dojo-login", "--pam-confdir", "/tmp/x"]), None);
    assert_eq!(parse(&["--pam-confdir", "dojo-login"]), None);
    assert_eq!(parse(&["--other", "x", "dojo-login"]), None);
}

/// The parent a session's double-forked daemon ends up with once the session has exited,
/// with this process standing in for the helper as a child subreaper.
#[cfg(feature = "fake-pam-stack")]
fn parent_of_orphan(own_orphans: bool) -> i32 {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static SUBREAPER: Mutex<()> = Mutex::new(());
    let _flag = SUBREAPER.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) };

    let directory = tempfile::tempdir().unwrap();
    let report = directory.path().join("parent");
    let script = format!(
        "me=$$\n\
         ( while :; do read -r _ _ _ parent _ < /proc/self/stat; \
           [ \"$parent\" != \"$me\" ] && break; sleep 0.01; done; \
           echo \"$parent\" > '{0}.tmp'; mv '{0}.tmp' '{0}' ) &\n",
        report.display()
    );
    let uid = unsafe { libc::geteuid() };
    let plan = SessionPlan {
        shell: "/bin/sh".into(),
        arguments: vec!["-c".into(), script.into()],
        environment: vec![("PATH".into(), "/usr/bin".into())],
        home: "/".into(),
        uid,
        gid: unsafe { libc::getegid() },
        account: c"test".into(),
        own_orphans,
    };
    let mut launcher = ForkLauncher {
        child: None,
        core: libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        },
    };
    launcher.spawn(&plan).unwrap();
    launcher.wait();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !report.exists() {
        assert!(Instant::now() < deadline, "the orphan never reported");
        std::thread::sleep(Duration::from_millis(10));
    }
    std::fs::read_to_string(&report).unwrap().trim().parse().unwrap()
}

#[cfg(feature = "fake-pam-stack")]
#[test]
fn the_greeters_orphans_are_reparented_to_the_helper_and_a_users_are_not() {
    let helper = std::process::id() as i32;
    assert_eq!(parent_of_orphan(true), helper);
    assert_ne!(parent_of_orphan(false), helper);
}
