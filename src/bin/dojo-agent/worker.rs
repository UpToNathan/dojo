use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use super::{DesktopAction, DesktopSession};

pub(super) struct ActiveWorker {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    pub(super) seat_name: Option<String>,
}

impl ActiveWorker {
    pub(super) fn spawn(socket: Option<&Path>, name: &str, idle_timeout: Duration) -> Result<Self> {
        let mut command = Command::new(std::env::current_exe()?);
        command
            .args(["--worker", "--name", name, "--idle-timeout-seconds"])
            .arg(idle_timeout.as_secs().to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        if let Some(socket) = socket {
            command.arg("--socket").arg(socket);
        }
        let mut child = command.spawn().context("start desktop worker")?;
        let input = child.stdin.take().expect("worker stdin is piped");
        let output = BufReader::new(child.stdout.take().expect("worker stdout is piped"));
        Ok(Self {
            child,
            input,
            output,
            seat_name: None,
        })
    }

    pub(super) fn pid(&self) -> u32 {
        self.child.id()
    }

    pub(super) fn exited(&mut self) -> Result<bool> {
        Ok(self.child.try_wait()?.is_some())
    }

    pub(super) fn action(&mut self, action: &DesktopAction) -> Result<Value> {
        serde_json::to_writer(&mut self.input, action)?;
        self.input.write_all(b"\n")?;
        self.input.flush()?;
        let mut reply = String::new();
        if self.output.read_line(&mut reply)? == 0 {
            bail!("desktop worker stopped; focus or click a fresh target before continuing");
        }
        let response: std::result::Result<Value, String> = serde_json::from_str(&reply)?;
        let value = response.map_err(anyhow::Error::msg)?;
        self.seat_name = value
            .get("seat_name")
            .and_then(Value::as_str)
            .map(str::to_owned);
        Ok(value)
    }
}

impl Drop for ActiveWorker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub(super) fn run(socket: Option<PathBuf>, name: String, idle_timeout: Duration) -> Result<()> {
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let action = line.map_err(anyhow::Error::from).and_then(|line| {
                serde_json::from_str::<DesktopAction>(&line).map_err(anyhow::Error::from)
            });
            if sender.send(action).is_err() {
                break;
            }
        }
    });
    let mut session = DesktopSession {
        socket,
        name,
        controller: None,
    };
    let mut output = std::io::stdout().lock();
    while let Ok(action) = receiver.recv_timeout(idle_timeout) {
        let result = action.and_then(|action| session.action(action));
        let failed = result.is_err();
        let reply = result.map_err(|error| format!("{error:#}"));
        serde_json::to_writer(&mut output, &reply)?;
        output.write_all(b"\n")?;
        output.flush()?;
        if failed {
            break;
        }
    }
    Ok(())
}
