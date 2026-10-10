use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{Context as _, bail};
use async_channel::Sender;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlCommand {
    Toggle,
    Enable,
    Disable,
    Quit,
    Launcher,
    Theme,
}

impl ControlCommand {
    fn as_str(self) -> &'static str {
        match self {
            Self::Toggle => "toggle",
            Self::Enable => "enable",
            Self::Disable => "disable",
            Self::Quit => "quit",
            Self::Launcher => "launcher",
            Self::Theme => "theme",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "toggle" => Some(Self::Toggle),
            "enable" => Some(Self::Enable),
            "disable" => Some(Self::Disable),
            "quit" => Some(Self::Quit),
            "launcher" => Some(Self::Launcher),
            "theme" => Some(Self::Theme),
            _ => None,
        }
    }
}

pub fn send(command: ControlCommand) -> anyhow::Result<bool> {
    let path = socket_path()?;
    let mut stream = match UnixStream::connect(&path) {
        Ok(stream) => stream,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ) =>
        {
            return Ok(false);
        }
        Err(error) => {
            return Err(error).with_context(|| format!("connecting to {}", path.display()));
        }
    };

    writeln!(stream, "{}", command.as_str()).context("sending Dojo control command")?;
    let mut reply = String::new();
    BufReader::new(stream)
        .read_line(&mut reply)
        .context("reading Dojo control response")?;
    if reply.trim() != "queued" {
        bail!("Dojo rejected control command: {}", reply.trim());
    }
    Ok(true)
}

pub struct ControlServer {
    path: PathBuf,
    shutdown: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ControlServer {
    pub fn start(sender: Sender<ControlCommand>) -> anyhow::Result<Self> {
        let path = socket_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating runtime directory {}", parent.display()))?;
        }

        if path.exists() {
            match UnixStream::connect(&path) {
                Ok(_) => bail!("another Dojo instance owns {}", path.display()),
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                    ) =>
                {
                    fs::remove_file(&path)
                        .with_context(|| format!("removing stale socket {}", path.display()))?;
                }
                Err(error) => {
                    return Err(error).with_context(|| format!("checking {}", path.display()));
                }
            }
        }

        let listener = UnixListener::bind(&path)
            .with_context(|| format!("binding Dojo control socket {}", path.display()))?;
        let shutdown = Arc::new(AtomicBool::new(false));
        let thread_shutdown = shutdown.clone();
        let thread = thread::spawn(move || {
            while !thread_shutdown.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        if thread_shutdown.load(Ordering::Relaxed) {
                            break;
                        }
                        handle_client(stream, &sender);
                    }
                    Err(error) => {
                        eprintln!("Dojo control socket failed: {error}");
                        break;
                    }
                }
            }
        });

        Ok(Self {
            path,
            shutdown,
            thread: Some(thread),
        })
    }

    pub fn shutdown(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        let _ = UnixStream::connect(&self.path);
        if let Some(thread) = self.thread.take() {
            if let Err(error) = thread.join() {
                eprintln!("Dojo control thread panicked: {error:?}");
            }
        }
        if let Err(error) = fs::remove_file(&self.path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                eprintln!("Failed to remove Dojo control socket: {error}");
            }
        }
    }
}

impl Drop for ControlServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn handle_client(mut stream: UnixStream, sender: &Sender<ControlCommand>) {
    if let Err(error) = stream.set_read_timeout(Some(Duration::from_secs(1))) {
        eprintln!("Failed to set Dojo control timeout: {error}");
        return;
    }
    let mut command = String::new();
    let result = BufReader::new(&mut stream).read_line(&mut command);
    let response = match result {
        Ok(_) => match ControlCommand::parse(command.trim()) {
            Some(command) if sender.try_send(command).is_ok() => "queued\n",
            Some(_) => "stopping\n",
            None => "invalid\n",
        },
        Err(error) => {
            eprintln!("Failed to read Dojo control command: {error}");
            "invalid\n"
        }
    };
    if let Err(error) = stream.write_all(response.as_bytes()) {
        eprintln!("Failed to reply to Dojo control client: {error}");
    }
}

fn socket_path() -> anyhow::Result<PathBuf> {
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .context("XDG_RUNTIME_DIR is not set; run Dojo inside a desktop session")?;
    if !runtime_dir.is_absolute() {
        bail!("XDG_RUNTIME_DIR must be absolute");
    }
    Ok(runtime_dir.join("dojo.sock"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launcher_command_is_delivered_and_acknowledged() -> anyhow::Result<()> {
        let (sender, receiver) = async_channel::bounded(1);
        let (server, mut client) = UnixStream::pair()?;
        client.write_all(b"launcher\n")?;
        handle_client(server, &sender);
        assert_eq!(receiver.try_recv()?, ControlCommand::Launcher);
        let mut response = String::new();
        BufReader::new(client).read_line(&mut response)?;
        assert_eq!(response, "queued\n");
        Ok(())
    }
}
