//! Carries a call to a peer with `tailscale ssh`: the identity and permission are Tailscale's.
//! The one thing added is noticing when Tailscale asks a human to sign in again, which would
//! otherwise leave the call waiting forever.

use crate::channel::{Call, Done, Ended, Input, Output};
use crate::method::Route;
use crate::transport::check_url;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Runs the call. `Err` means `tailscale` could not be started at all.
pub fn run(call: Call) -> Result<Done, String> {
    let host = match &call.route {
        Route::Host { name, port: None } => name.clone(),
        Route::Host { port: Some(_), .. } => return Err("Tailscale SSH does not take a port".into()),
        Route::Command(_) => return Err("Tailscale SSH cannot use a command route".into()),
    };
    let mut command = Command::new("tailscale");
    command.args(["ssh", &host, &call.remote]);
    carry(command, "tailscale", call.input, call.output, call.timeout, check_url)
}

/// Starts `command`, feeds and collects it as the call says, and waits for it to end. `watch` reads
/// the program's error output line by line and returns a page or a step when a person is needed, which
/// ends the call at once instead of leaving it waiting. `Err` means the program could not be started.
pub fn carry(mut command: Command, name: &str, input: Input, output: Output, timeout: Option<Duration>, watch: fn(&str) -> Option<String>) -> Result<Done, String> {
    let mut feed = None;
    command.stdin(match input {
        Input::Inherit => Stdio::inherit(),
        Input::Null => Stdio::null(),
        Input::File(file) => Stdio::from(file),
        Input::Bytes(bytes) => {
            feed = Some(bytes);
            Stdio::piped()
        }
        Input::Stdio(stdio) => stdio,
    });
    let capture = matches!(output, Output::Capture);
    command.stdout(match output {
        Output::Inherit => Stdio::inherit(),
        Output::Capture => Stdio::piped(),
        Output::File(file) => Stdio::from(file),
    });
    command.stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound { format!("{name} is not installed") } else { format!("could not run {name}: {error}") }
    })?;

    if let Some(bytes) = feed {
        let mut stdin = child.stdin.take().expect("stdin was piped");
        std::thread::spawn(move || {
            let _ = stdin.write_all(&bytes);
        });
    }
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
                    if let Some(url) = watch(&String::from_utf8_lossy(&line)) {
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
            Err(error) => return Err(format!("lost track of {name}: {error}")),
        }
        if timeout.is_some_and(|limit| started.elapsed() >= limit) {
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
