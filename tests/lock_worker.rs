//! The PAM worker mechanism against private `pam_start_confdir` stacks.
//! Ports the spike scenarios A to H (`.scratch/lockscreen/spikes/pam/bin/run_scenarios.py`).
//! Stand-ins only: the real pam_fprintd stack would claim the sensor.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use async_channel::{Receiver, Sender};
use dojo::lock::pam::{Method, Notice, PamOutcome, Report, WorkerId};
use dojo::lock::secret::Password;
use dojo::lock::worker::{Helper, Worker};

const SECRET: &str = "fake-lock-secret-0123456789";
const FINGER_HINT: &str = "Place your finger on the reader (stand-in)";

const CHECK_PASSWORD: &str = r#"#!/bin/bash
# Compares stdin with a fake constant, never a real password. Holds until DIR/check.release exists.
IFS= read -r -d '' pw
touch "$1/check.running"
for _ in $(seq 500); do [ -e "$1/check.release" ] && break; sleep 0.01; done
[ "$pw" = "fake-lock-secret-0123456789" ]
"#;

const SLOW_FINGER: &str = r#"#!/bin/bash
# usage: slow_finger.sh SECONDS DIR. Holds DIR/sensor like fprintd holds the reader.
exec 9>"$2/sensor"
flock -n 9 || exit 1
echo $$ > "$2/finger.pid"
sleep "$1" &
echo $! > "$2/finger-sleep.pid"
touch "$2/finger.started"
wait
touch "$2/finger.completed"
"#;

const CRASH_PARENT: &str = r#"#!/bin/bash
# Kills the PAM worker, its parent, the way a segfault would.
sleep 0.3
kill -9 "$PPID"
"#;

fn password_stack() -> String {
    "auth [success=1 default=ignore] pam_exec.so quiet expose_authtok $DIR/check_password.sh $DIR\n\
     auth requisite pam_deny.so\n\
     auth required pam_permit.so\n"
        .into()
}

fn slow_finger_stack(seconds: f32) -> String {
    format!(
        "auth optional pam_echo.so file=$DIR/finger.txt\n\
         auth required pam_exec.so quiet $DIR/slow_finger.sh {seconds} $DIR\n"
    )
}

const DENY: &str = "auth required pam_deny.so\n";
const CRASH: &str = "auth optional pam_echo.so file=$DIR/finger.txt\nauth required pam_exec.so quiet $DIR/crash_parent.sh\n";

fn uid() -> u32 {
    unsafe { libc::getuid() }
}

fn password(text: &str) -> Password {
    let mut password = Password::new();
    for c in text.chars() {
        assert!(password.push(c));
    }
    password
}

struct Bench {
    dir: tempfile::TempDir,
    helper: Helper,
    sender: Sender<(WorkerId, Report)>,
    receiver: Receiver<(WorkerId, Report)>,
    pending: RefCell<VecDeque<(WorkerId, Report)>>,
}

impl Bench {
    fn new(fingerprint: &str, password: &str) -> Bench {
        let dir = tempfile::Builder::new()
            .prefix("lock-worker-")
            .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
            .unwrap();
        let root = dir.path().to_str().unwrap().to_owned();
        let confdir = dir.path().join("pam.d");
        fs::create_dir(&confdir).unwrap();
        fs::create_dir(dir.path().join("tally")).unwrap();
        fs::write(
            confdir.join(Method::Fingerprint.service()),
            fingerprint.replace("$DIR", &root),
        )
        .unwrap();
        fs::write(
            confdir.join(Method::Password.service()),
            password.replace("$DIR", &root),
        )
        .unwrap();
        fs::write(dir.path().join("finger.txt"), FINGER_HINT).unwrap();
        fs::write(dir.path().join("check.release"), "").unwrap();
        for (name, body) in [
            ("check_password.sh", CHECK_PASSWORD),
            ("slow_finger.sh", SLOW_FINGER),
            ("crash_parent.sh", CRASH_PARENT),
        ] {
            let path = dir.path().join(name);
            fs::write(&path, body).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let helper = Helper::new(env!("CARGO_BIN_EXE_dojo-lock-pam").into()).with_confdir(confdir);
        let (sender, receiver) = async_channel::unbounded();
        Bench {
            dir,
            helper,
            sender,
            receiver,
            pending: RefCell::default(),
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn spawn(&self, method: Method, generation: u32) -> Worker {
        self.helper
            .spawn(WorkerId { method, generation }, &self.sender)
            .unwrap()
    }

    /// The next report from `worker`, keeping other workers' reports in order for later.
    fn next(&self, worker: &Worker) -> Report {
        let mut pending = self.pending.borrow_mut();
        if let Some(index) = pending.iter().position(|(id, _)| *id == worker.id()) {
            return pending.remove(index).unwrap().1;
        }
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            match self.receiver.try_recv() {
                Ok((id, report)) if id == worker.id() => return report,
                Ok(other) => pending.push_back(other),
                Err(_) if Instant::now() > deadline => panic!("no report from {worker:?}"),
                Err(_) => std::thread::sleep(Duration::from_millis(5)),
            }
        }
    }

    fn expect(&self, worker: &Worker, expected: Report) {
        assert_eq!(self.next(worker), expected, "from {worker:?}");
    }

    fn assert_silent(&self, wait: Duration) {
        std::thread::sleep(wait);
        let mut late: Vec<_> = self.pending.borrow_mut().drain(..).collect();
        late.extend(std::iter::from_fn(|| self.receiver.try_recv().ok()));
        assert_eq!(late, vec![]);
    }

    fn wait_for(&self, name: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !self.path(name).exists() {
            assert!(Instant::now() < deadline, "{name} never appeared");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn pid_file(&self, name: &str) -> i32 {
        fs::read_to_string(self.path(name))
            .unwrap()
            .trim()
            .parse()
            .unwrap()
    }

    fn sensor_is_free(&self) -> bool {
        let file = fs::File::open(self.path("sensor")).unwrap();
        use std::os::fd::AsRawFd;
        unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) == 0 }
    }
}

fn hint(text: &str) -> Report {
    Report::Notice(Notice::Hint {
        text: text.into(),
        error: false,
    })
}

/// Live and not a zombie, read straight from /proc.
fn alive(pid: i32) -> bool {
    match fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => !matches!(
            stat.rsplit_once(") ").map(|(_, rest)| &rest[..1]),
            Some("Z" | "X")
        ),
        Err(_) => false,
    }
}

/// `root` and its descendants by parent link, from /proc.
fn descendants(root: i32) -> Vec<i32> {
    let parents: Vec<(i32, i32)> = fs::read_dir("/proc")
        .unwrap()
        .flatten()
        .filter_map(|entry| entry.file_name().to_str()?.parse::<i32>().ok())
        .filter_map(|pid| {
            let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
            let parent = stat.rsplit_once(") ")?.1.split(' ').nth(1)?.parse().ok()?;
            Some((pid, parent))
        })
        .collect();
    let mut tree = vec![root];
    let mut index = 0;
    while index < tree.len() {
        let parent = tree[index];
        tree.extend(
            parents
                .iter()
                .filter(|(_, p)| *p == parent)
                .map(|(pid, _)| *pid),
        );
        index += 1;
    }
    tree
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn assert_dead(pids: &[i32]) {
    let living: Vec<i32> = pids.iter().copied().filter(|&pid| alive(pid)).collect();
    assert_eq!(living, Vec::<i32>::new(), "still alive");
}

#[test]
fn a_password_wins_while_the_fingerprint_waits() {
    let bench = Bench::new(&slow_finger_stack(4.0), &password_stack());
    fs::remove_file(bench.path("check.release")).unwrap();
    let mut finger = bench.spawn(Method::Fingerprint, 1);
    let mut word = bench.spawn(Method::Password, 1);
    bench.expect(&finger, hint(FINGER_HINT));
    bench.wait_for("finger.started");
    bench.expect(&word, Report::Prompt { echo: false });

    let cmdline = fs::read(format!("/proc/{}/cmdline", word.pid())).unwrap();
    let confdir = bench.path("pam.d");
    let expected = [
        "/usr/lib64/ld-linux-x86-64.so.2",
        "--library-path",
        "/usr/lib",
        env!("CARGO_BIN_EXE_dojo-lock-pam"),
        "--pam-confdir",
        confdir.to_str().unwrap(),
        "dojo-lock-password",
    ]
    .map(|argument| format!("{argument}\0"))
    .concat();
    assert_eq!(String::from_utf8_lossy(&cmdline), expected);
    let environ = fs::read(format!("/proc/{}/environ", word.pid()));
    assert_eq!(
        environ.unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied
    );

    word.answer(password(SECRET)).unwrap();
    bench.wait_for("check.running");
    let holders = descendants(word.pid() as i32);
    assert!(
        holders.len() >= 2,
        "the checking script runs under the worker: {holders:?}"
    );
    for pid in &holders {
        let cmdline = fs::read(format!("/proc/{pid}/cmdline")).unwrap();
        assert!(
            !contains(&cmdline, SECRET.as_bytes()),
            "secret in cmdline of {pid}"
        );
        match fs::read(format!("/proc/{pid}/environ")) {
            Ok(environ) => assert!(
                !contains(&environ, SECRET.as_bytes()),
                "secret in environ of {pid}"
            ),
            Err(error) => assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied),
        }
    }
    fs::write(bench.path("check.release"), "").unwrap();
    bench.expect(&word, Report::Finished(PamOutcome::Success));

    let finger_processes = [
        finger.pid() as i32,
        bench.pid_file("finger.pid"),
        bench.pid_file("finger-sleep.pid"),
    ];
    assert_eq!(finger.kill(), Vec::<u32>::new());
    assert_eq!(word.kill(), Vec::<u32>::new());
    assert_dead(&finger_processes);
    assert_dead(&holders);
    assert!(bench.sensor_is_free(), "sensor released after the kill");
    assert!(!bench.path("finger.completed").exists());
    bench.assert_silent(Duration::from_millis(200));
}

#[test]
fn b_fingerprint_wins_while_the_password_waits_at_its_prompt() {
    let bench = Bench::new(&slow_finger_stack(1.5), &password_stack());
    let mut finger = bench.spawn(Method::Fingerprint, 1);
    let mut word = bench.spawn(Method::Password, 1);
    bench.expect(&word, Report::Prompt { echo: false });
    bench.expect(&finger, hint(FINGER_HINT));
    bench.expect(&finger, Report::Finished(PamOutcome::Success));
    let word_pid = word.pid() as i32;
    assert!(
        alive(word_pid),
        "password worker still blocked on the prompt"
    );
    assert_eq!(word.kill(), Vec::<u32>::new());
    assert_eq!(finger.kill(), Vec::<u32>::new());
    assert_dead(&[word_pid, finger.pid() as i32]);
    assert!(bench.path("finger.completed").exists());
    assert!(bench.sensor_is_free());
    bench.assert_silent(Duration::from_millis(100));
}

#[test]
fn c_both_fail() {
    let bench = Bench::new(DENY, &password_stack());
    let finger = bench.spawn(Method::Fingerprint, 1);
    let word = bench.spawn(Method::Password, 1);
    bench.expect(&finger, Report::Finished(PamOutcome::Rejected));
    bench.expect(&word, Report::Prompt { echo: false });
    word.answer(password("wrong")).unwrap();
    bench.expect(&word, Report::Finished(PamOutcome::Rejected));
}

#[test]
fn d_fingerprint_worker_crashes_then_password_wins() {
    let bench = Bench::new(CRASH, &password_stack());
    let mut finger = bench.spawn(Method::Fingerprint, 1);
    let word = bench.spawn(Method::Password, 1);
    bench.expect(&finger, hint(FINGER_HINT));
    bench.expect(&finger, Report::Crashed);
    bench.expect(&word, Report::Prompt { echo: false });
    word.answer(password(SECRET)).unwrap();
    bench.expect(&word, Report::Finished(PamOutcome::Success));
    assert_eq!(finger.kill(), Vec::<u32>::new());
    assert_eq!(finger.kill(), Vec::<u32>::new(), "kill is idempotent");
    assert_dead(&[finger.pid() as i32]);
}

#[test]
fn e_both_crash() {
    let bench = Bench::new(CRASH, CRASH);
    let finger = bench.spawn(Method::Fingerprint, 1);
    let word = bench.spawn(Method::Password, 1);
    for worker in [&finger, &word] {
        bench.expect(worker, hint(FINGER_HINT));
        bench.expect(worker, Report::Crashed);
    }
}

#[test]
fn f_killing_blocked_workers_leaves_nothing_and_reports_nothing() {
    let bench = Bench::new(&slow_finger_stack(30.0), &password_stack());
    let mut finger = bench.spawn(Method::Fingerprint, 1);
    let mut word = bench.spawn(Method::Password, 1);
    bench.expect(&finger, hint(FINGER_HINT));
    bench.expect(&word, Report::Prompt { echo: false });
    bench.wait_for("finger.started");
    std::thread::sleep(Duration::from_secs(2));
    let everything = [
        finger.pid() as i32,
        word.pid() as i32,
        bench.pid_file("finger.pid"),
        bench.pid_file("finger-sleep.pid"),
    ];
    let started = Instant::now();
    assert_eq!(finger.kill(), Vec::<u32>::new());
    assert_eq!(word.kill(), Vec::<u32>::new());
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "kill took {:?}",
        started.elapsed()
    );
    assert_dead(&everything);
    assert!(bench.sensor_is_free());
    bench.assert_silent(Duration::from_millis(300));
    assert!(!bench.path("finger.completed").exists());
}

#[test]
fn g_instant_success_and_instant_failure() {
    let bench = Bench::new(
        &format!("auth requisite pam_succeed_if.so uid eq {}\n", uid()),
        &format!("auth requisite pam_succeed_if.so uid ne {}\n", uid()),
    );
    let finger = bench.spawn(Method::Fingerprint, 1);
    let word = bench.spawn(Method::Password, 1);
    bench.expect(&finger, Report::Finished(PamOutcome::Success));
    bench.expect(&word, Report::Finished(PamOutcome::Rejected));
}

#[test]
fn h_fingerprint_fails_fast_and_an_early_answer_still_wins() {
    let bench = Bench::new(DENY, &password_stack());
    let finger = bench.spawn(Method::Fingerprint, 1);
    let word = bench.spawn(Method::Password, 7);
    word.answer(password(SECRET)).unwrap();
    bench.expect(&finger, Report::Finished(PamOutcome::Rejected));
    bench.expect(&word, Report::Prompt { echo: false });
    bench.expect(&word, Report::Finished(PamOutcome::Success));
}

#[test]
fn faillock_pauses_the_password_with_its_own_words() {
    let tally = "dir=$DIR/tally deny=3 unlock_time=600";
    let bench = Bench::new(
        DENY,
        &format!(
            "auth required pam_faillock.so preauth {tally}\n\
             auth [success=1 default=bad] pam_deny.so\n\
             auth [default=die] pam_faillock.so authfail {tally}\n\
             auth required pam_faillock.so authsucc {tally}\n"
        ),
    );
    let failures: Vec<Worker> = (1..=3)
        .map(|generation| bench.spawn(Method::Password, generation))
        .collect();
    for worker in &failures {
        bench.expect(worker, Report::Finished(PamOutcome::Rejected));
    }
    let locked = bench.spawn(Method::Password, 4);
    bench.expect(&locked, Report::Notice(Notice::PasswordLocked));
    bench.expect(
        &locked,
        Report::Notice(Notice::PasswordPaused {
            remaining: Duration::from_secs(600),
        }),
    );
    bench.expect(&locked, Report::Finished(PamOutcome::Rejected));
}

#[test]
fn secrets_print_redacted() {
    let bench = Bench::new(DENY, &password_stack());
    let worker = bench.spawn(Method::Password, 3);
    let secret = password(SECRET);
    assert_eq!(format!("{secret:?}"), "Password(<redacted>)");
    let shown = format!("{worker:?}");
    assert_eq!(
        shown,
        format!(
            "Worker {{ id: WorkerId {{ method: Password, generation: 3 }}, pid: {}, .. }}",
            worker.pid()
        )
    );
    worker.answer(secret).unwrap();
    bench.expect(&worker, Report::Prompt { echo: false });
    bench.expect(&worker, Report::Finished(PamOutcome::Success));
}

#[test]
fn an_unread_answer_does_not_hide_the_result() {
    let bench = Bench::new(DENY, DENY);
    let worker = bench.spawn(Method::Password, 1);
    // The worker may already be gone, in which case the send fails; either way the result arrives.
    let _ = worker.answer(password(SECRET));
    bench.expect(&worker, Report::Finished(PamOutcome::Rejected));
}
