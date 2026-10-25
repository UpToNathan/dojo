//! Carries a call to a peer with `tailscale ssh`: the identity and permission are Tailscale's.
//! The one thing added is noticing when Tailscale asks a human to sign in again, which would
//! otherwise leave the call waiting forever.

use crate::transport::check_url;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub enum Input {
    Inherit,
    Null,
    File(File),
}

pub enum Output {
    Inherit,
    /// Collected and returned in [`Done::stdout`].
    Capture,
    File(File),
}

pub struct Call {
    pub host: String,
    /// The single command string the peer's shell runs.
    pub remote: String,
    pub input: Input,
    pub output: Output,
    pub timeout: Option<Duration>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Ended {
    Exited(i32),
    /// Ended by a signal.
    Killed,
    /// Tailscale wants a person to approve this connection at the given page.
    NeedsHuman(String),
    TimedOut,
}

#[derive(Debug)]
pub struct Done {
    pub ended: Ended,
    pub stdout: Vec<u8>,
}

/// Runs the call. `Err` means `tailscale` could not be started at all.
pub fn run(call: Call) -> Result<Done, String> {
    let mut command = Command::new("tailscale");
    command.args(["ssh", &call.host, &call.remote]);
    command.stdin(match call.input {
        Input::Inherit => Stdio::inherit(),
        Input::Null => Stdio::null(),
        Input::File(file) => Stdio::from(file),
    });
    let capture = matches!(call.output, Output::Capture);
    command.stdout(match call.output {
        Output::Inherit => Stdio::inherit(),
        Output::Capture => Stdio::piped(),
        Output::File(file) => Stdio::from(file),
    });
    command.stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound { "tailscale is not installed".to_string() } else { format!("could not run tailscale: {error}") }
    })?;

    let (url_sender, url_receiver) = mpsc::channel::<String>();
    let stderr = child.stderr.take().expect("stderr was piped");
    let stderr_reader = std::thread::spawn(move || {
        let mut reader = BufReader::new(stderr);
        let mut line = Vec::new();
        let mut out = std::io::stderr();
        loop {
            line.clear();
            match reader.read_until(b'\n', &mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let _ = out.write_all(&line);
                    if let Some(url) = check_url(&String::from_utf8_lossy(&line)) {
                        let _ = url_sender.send(url);
                    }
                }
            }
        }
    });
    let stdout_reader = capture.then(|| {
        let mut pipe = child.stdout.take().expect("stdout was piped");
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = pipe.read_to_end(&mut bytes);
            bytes
        })
    });

    let started = Instant::now();
    let ended = loop {
        if let Ok(url) = url_receiver.try_recv() {
            let _ = child.kill();
            let _ = child.wait();
            break Ended::NeedsHuman(url);
        }
        match child.try_wait() {
            Ok(Some(status)) => break status.code().map(Ended::Exited).unwrap_or(Ended::Killed),
            Ok(None) => {}
            Err(error) => return Err(format!("lost track of tailscale: {error}")),
        }
        if call.timeout.is_some_and(|limit| started.elapsed() >= limit) {
            let _ = child.kill();
            let _ = child.wait();
            break Ended::TimedOut;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    // A killed client can leave a helper holding the pipes open for a while. Waiting for them to
    // close would make a call that was ended wait as long as the helper lives, so they are let go.
    let stopped = matches!(ended, Ended::NeedsHuman(_) | Ended::TimedOut);
    let stdout = if stopped {
        Vec::new()
    } else {
        let _ = stderr_reader.join();
        stdout_reader.map(|reader| reader.join().unwrap_or_default()).unwrap_or_default()
    };
    // A page that arrived in the last moments still means a person was needed.
    let ended = match (ended, url_receiver.try_recv()) {
        (Ended::Exited(code), Ok(url)) if code != 0 => Ended::NeedsHuman(url),
        (ended, _) => ended,
    };
    Ok(Done { ended, stdout })
}
