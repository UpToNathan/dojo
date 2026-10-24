//! Native client for the hold-to-talk keybinds.
//!
//! `tools/speech/voice.mjs` is both the voice service and its command-line client, so every
//! keypress used to load the whole service (Pi runtime, memory, sign-in) just to write one line
//! to a socket: about 250 ms. This client does the same exchange in a few milliseconds.
//!
//! It handles only the actions a keybind or status check needs. Anything else, a missing
//! service, or an unusable environment exits with `FALLBACK` so `tools/speech/dojo-voice` runs
//! the full client, which also starts the service on demand.

use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

/// EX_TEMPFAIL: tells the wrapper to run the full client instead.
const FALLBACK: u8 = 75;
const ACTIONS: [&str; 5] = ["begin", "finish", "cancel", "toggle", "status"];
/// Matches the full client's socket timeout.
const TIMEOUT: Duration = Duration::from_secs(45);
const MAX_REPLY: u64 = 1 << 20;

#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    /// The service answered; print this line.
    Reply { line: String, error: bool },
    /// The service is not running and the action needs nothing from it.
    Nothing,
    /// Hand over to the full client.
    Fallback,
}

fn held_path(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join("dojo-voice-held")
}

fn socket_path(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join("dojo-voice.sock")
}

/// Every error reply from the service is `{"error":"..."}`.
fn is_error(reply: &str) -> bool {
    reply.starts_with("{\"error\":")
}

fn exchange(socket: &Path, action: &str) -> std::io::Result<String> {
    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    writeln!(stream, "{{\"action\":\"{action}\"}}")?;
    let mut reply = String::new();
    stream.take(MAX_REPLY).read_to_string(&mut reply)?;
    let line = reply.lines().next().unwrap_or_default().to_owned();
    if line.is_empty() {
        return Err(std::io::Error::new(
            ErrorKind::UnexpectedEof,
            "Local socket closed without a response.",
        ));
    }
    Ok(line)
}

/// Whether `dojo-voice.service` is up. The runtime is socket activated, so connecting to its
/// socket starts it; a release or cancel that finds it idle must not do that.
fn service_active() -> bool {
    std::process::Command::new("systemctl")
        .args(["--user", "is-active", "--quiet", "dojo-voice.service"])
        .status()
        .is_ok_and(|status| status.success())
}

/// The held marker is what lets a release that races its press cancel the press, so it is
/// written before the request and removed before the request, exactly as the full client does.
fn run(runtime_dir: &Path, action: &str) -> std::io::Result<Outcome> {
    run_with(runtime_dir, action, service_active)
}

fn run_with(
    runtime_dir: &Path,
    action: &str,
    service_active: impl Fn() -> bool,
) -> std::io::Result<Outcome> {
    if !ACTIONS.contains(&action) {
        return Ok(Outcome::Fallback);
    }
    let held = held_path(runtime_dir);
    match action {
        "begin" => {
            fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&held)?;
        }
        "finish" | "cancel" => match fs::remove_file(&held) {
            Err(error) if error.kind() != ErrorKind::NotFound => return Err(error),
            _ => {}
        },
        _ => {}
    }
    // Nothing to finish or cancel in a runtime that is not running, and asking would start it. A
    // runtime that is still starting counts as not running: the press that started it sees the
    // cleared held marker and cancels itself.
    if matches!(action, "finish" | "cancel") && !service_active() {
        return Ok(Outcome::Nothing);
    }
    match exchange(&socket_path(runtime_dir), action) {
        Ok(line) => Ok(Outcome::Reply {
            error: is_error(&line),
            line,
        }),
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::NotFound | ErrorKind::ConnectionRefused
            ) =>
        {
            Ok(if matches!(action, "finish" | "cancel") {
                Outcome::Nothing
            } else {
                Outcome::Fallback
            })
        }
        Err(error) => Err(error),
    }
}

fn main() -> ExitCode {
    let mut arguments = std::env::args().skip(1);
    let action = arguments.next().unwrap_or_else(|| "begin".to_owned());
    let Some(runtime_dir) = std::env::var_os("XDG_RUNTIME_DIR").filter(|value| !value.is_empty())
    else {
        return ExitCode::from(FALLBACK);
    };
    if arguments.next().is_some() {
        return ExitCode::from(FALLBACK);
    }
    match run(Path::new(&runtime_dir), &action) {
        Ok(Outcome::Reply { line, error }) => {
            println!("{line}");
            ExitCode::from(u8::from(error))
        }
        Ok(Outcome::Nothing) => {
            println!("{{}}");
            ExitCode::SUCCESS
        }
        Ok(Outcome::Fallback) => ExitCode::from(FALLBACK),
        Err(error) => {
            eprintln!("dojo-voice: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixListener;
    use std::thread;

    struct Runtime(PathBuf);

    impl Runtime {
        fn new(name: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("dojo-voice-ctl-{}-{name}", std::process::id()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Runtime {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn serve_once(runtime: &Path, reply: &'static str) -> thread::JoinHandle<String> {
        let listener = UnixListener::bind(socket_path(runtime)).unwrap();
        thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            BufReader::new(&stream).read_line(&mut request).unwrap();
            (&stream).write_all(reply.as_bytes()).unwrap();
            request
        })
    }

    #[test]
    fn begin_marks_the_press_held_and_forwards_the_action() {
        let runtime = Runtime::new("begin");
        let server = serve_once(&runtime.0, "{\"recording\":true}\n");
        let outcome = run(&runtime.0, "begin").unwrap();
        assert_eq!(
            outcome,
            Outcome::Reply {
                line: "{\"recording\":true}".into(),
                error: false
            }
        );
        assert_eq!(server.join().unwrap(), "{\"action\":\"begin\"}\n");
        assert!(held_path(&runtime.0).exists());
    }

    #[test]
    fn finish_and_cancel_clear_the_marker_even_without_a_service() {
        for action in ["finish", "cancel"] {
            let runtime = Runtime::new(action);
            fs::write(held_path(&runtime.0), "").unwrap();
            assert_eq!(
                run_with(&runtime.0, action, || true).unwrap(),
                Outcome::Nothing
            );
            assert!(!held_path(&runtime.0).exists());
        }
    }

    #[test]
    fn finish_and_cancel_never_connect_to_an_idle_runtime() {
        // A socket that would answer: reaching it would start the socket-activated runtime.
        for action in ["finish", "cancel"] {
            let runtime = Runtime::new(&format!("idle-{action}"));
            let listener = UnixListener::bind(socket_path(&runtime.0)).unwrap();
            listener.set_nonblocking(true).unwrap();
            fs::write(held_path(&runtime.0), "").unwrap();
            assert_eq!(
                run_with(&runtime.0, action, || false).unwrap(),
                Outcome::Nothing
            );
            assert!(
                listener.accept().is_err(),
                "{action} connected to the socket"
            );
            assert!(
                !held_path(&runtime.0).exists(),
                "the marker is still cleared"
            );
        }
    }

    #[test]
    fn finish_and_cancel_reach_a_running_runtime() {
        let runtime = Runtime::new("running");
        let server = serve_once(&runtime.0, "{}\n");
        assert_eq!(
            run_with(&runtime.0, "finish", || true).unwrap(),
            Outcome::Reply {
                line: "{}".into(),
                error: false
            }
        );
        assert_eq!(server.join().unwrap(), "{\"action\":\"finish\"}\n");
    }

    #[test]
    fn begin_toggle_and_status_wake_an_idle_runtime_because_they_need_it() {
        let runtime = Runtime::new("wake");
        let server = serve_once(&runtime.0, "{\"recording\":true}\n");
        // The activity check is not consulted for them: the connection is the wake-up.
        let outcome = run_with(&runtime.0, "toggle", || panic!("must not ask")).unwrap();
        assert!(matches!(outcome, Outcome::Reply { .. }));
        server.join().unwrap();
    }

    #[test]
    fn a_missing_service_falls_back_so_the_full_client_can_start_it() {
        let runtime = Runtime::new("missing");
        assert_eq!(run(&runtime.0, "toggle").unwrap(), Outcome::Fallback);
        assert_eq!(run(&runtime.0, "status").unwrap(), Outcome::Fallback);
    }

    #[test]
    fn a_stale_socket_counts_as_a_missing_service() {
        let runtime = Runtime::new("stale");
        drop(UnixListener::bind(socket_path(&runtime.0)).unwrap());
        assert_eq!(run(&runtime.0, "toggle").unwrap(), Outcome::Fallback);
    }

    #[test]
    fn other_actions_are_left_to_the_full_client() {
        let runtime = Runtime::new("other");
        for action in [
            "send",
            "resume",
            "select-model",
            "sign-out",
            "trace",
            "stop",
            "",
        ] {
            assert_eq!(run(&runtime.0, action).unwrap(), Outcome::Fallback);
        }
        assert!(!held_path(&runtime.0).exists());
    }

    #[test]
    fn service_errors_are_reported_as_errors() {
        let runtime = Runtime::new("error");
        let server = serve_once(&runtime.0, "{\"error\":\"Unknown voice command.\"}\n");
        let Outcome::Reply { error, .. } = run(&runtime.0, "status").unwrap() else {
            panic!("expected a reply");
        };
        assert!(error);
        server.join().unwrap();
        assert!(!is_error("{\"recording\":false}"));
    }

    #[test]
    fn a_silent_close_is_an_error() {
        let runtime = Runtime::new("silent");
        let server = serve_once(&runtime.0, "");
        assert_eq!(
            run(&runtime.0, "status").unwrap_err().kind(),
            ErrorKind::UnexpectedEof
        );
        server.join().unwrap();
    }
}
