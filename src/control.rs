use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{Context as _, bail};
use async_channel::Sender;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LauncherNavigation {
    Up,
    Down,
    Activate,
    Dismiss,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ControlCommand {
    Toggle,
    Enable,
    Disable,
    Quit,
    Launcher,
    LauncherNavigation(LauncherNavigation),
    Theme,
    Displays,
    Wifi,
    SystemMenu,
    Models,
    Terminal,
    TerminalClosed,
    /// Close the frame for an occasion, then run its action.
    Leave(crate::session::Occasion),
    Speech(crate::speech::SpeechEvent),
}

impl ControlCommand {
    fn as_str(&self) -> String {
        match self {
            Self::Speech(event) => {
                return serde_json::to_string(event).expect("speech events serialize");
            }
            Self::Leave(occasion) => return format!("leave {}", occasion.key()),
            Self::Toggle => "toggle",
            Self::Enable => "enable",
            Self::Disable => "disable",
            Self::Quit => "quit",
            Self::Launcher => "launcher",
            Self::LauncherNavigation(LauncherNavigation::Up) => "launcher-up",
            Self::LauncherNavigation(LauncherNavigation::Down) => "launcher-down",
            Self::LauncherNavigation(LauncherNavigation::Activate) => "launcher-activate",
            Self::LauncherNavigation(LauncherNavigation::Dismiss) => "launcher-dismiss",
            Self::Theme => "theme",
            Self::Displays => "displays",
            Self::Wifi => "wifi",
            Self::SystemMenu => "system-menu",
            Self::Models => "models",
            Self::Terminal => "terminal",
            Self::TerminalClosed => "terminal-closed",
        }
        .to_owned()
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "toggle" => Some(Self::Toggle),
            "enable" => Some(Self::Enable),
            "disable" => Some(Self::Disable),
            "quit" => Some(Self::Quit),
            "launcher" => Some(Self::Launcher),
            "launcher-up" => Some(Self::LauncherNavigation(LauncherNavigation::Up)),
            "launcher-down" => Some(Self::LauncherNavigation(LauncherNavigation::Down)),
            "launcher-activate" => Some(Self::LauncherNavigation(LauncherNavigation::Activate)),
            "launcher-dismiss" => Some(Self::LauncherNavigation(LauncherNavigation::Dismiss)),
            "theme" => Some(Self::Theme),
            "displays" => Some(Self::Displays),
            "wifi" => Some(Self::Wifi),
            "system-menu" => Some(Self::SystemMenu),
            "models" => Some(Self::Models),
            "terminal" => Some(Self::Terminal),
            "terminal-closed" => Some(Self::TerminalClosed),
            _ => value
                .strip_prefix("leave ")
                .map(|word| crate::session::Occasion::parse_closing(word).map(Self::Leave))
                .unwrap_or_else(|| {
                    serde_json::from_str::<crate::speech::SpeechEvent>(value)
                        .ok()
                        .filter(|event| event.validate().is_ok())
                        .map(Self::Speech)
                }),
        }
    }
}

pub fn send(command: &ControlCommand) -> anyhow::Result<bool> {
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

    stream.set_read_timeout(Some(Duration::from_secs(1)))?;
    stream.set_write_timeout(Some(Duration::from_secs(1)))?;
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
    let result = BufReader::new((&mut stream).take(16_385)).read_line(&mut command);
    let response = match result {
        Ok(_) if command.len() <= 16_384 && command.ends_with('\n') => {
            match ControlCommand::parse(command.trim()) {
                Some(command) => {
                    if sender.try_send(command).is_ok() {
                        "queued\n"
                    } else {
                        "stopping\n"
                    }
                }
                None => "invalid\n",
            }
        }
        Ok(_) => "invalid\n",
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
    fn shell_panel_commands_are_delivered() -> anyhow::Result<()> {
        for (token, expected) in [
            ("wifi", ControlCommand::Wifi),
            ("system-menu", ControlCommand::SystemMenu),
            ("models", ControlCommand::Models),
        ] {
            assert_eq!(ControlCommand::parse(token), Some(expected.clone()));
            let (sender, receiver) = async_channel::bounded(1);
            let (server, mut client) = UnixStream::pair()?;
            client.write_all(format!("{token}\n").as_bytes())?;
            handle_client(server, &sender);
            assert_eq!(receiver.try_recv()?, expected);
            let mut response = String::new();
            BufReader::new(client).read_line(&mut response)?;
            assert_eq!(response, "queued\n");
        }
        Ok(())
    }

    #[test]
    fn displays_command_is_delivered_and_acknowledged() -> anyhow::Result<()> {
        assert_eq!(
            ControlCommand::parse("displays"),
            Some(ControlCommand::Displays)
        );
        let (sender, receiver) = async_channel::bounded(1);
        let (server, mut client) = UnixStream::pair()?;
        client.write_all(b"displays\n")?;
        handle_client(server, &sender);
        assert_eq!(receiver.try_recv()?, ControlCommand::Displays);
        let mut response = String::new();
        BufReader::new(client).read_line(&mut response)?;
        assert_eq!(response, "queued\n");
        Ok(())
    }

    #[test]
    fn speech_response_survives_socket_delivery() -> anyhow::Result<()> {
        let (sender, receiver) = async_channel::bounded(1);
        let (server, mut client) = UnixStream::pair()?;
        client.write_all(b"{\"event\":\"response\",\"value\":\"Opened your bill.\"}\n")?;
        handle_client(server, &sender);
        assert_eq!(
            receiver.try_recv()?,
            ControlCommand::Speech(crate::speech::SpeechEvent::Response(
                "Opened your bill.".into()
            ))
        );
        Ok(())
    }

    #[test]
    fn invalid_phase_and_empty_response_are_rejected() {
        assert!(ControlCommand::parse(r#"{"event":"phase","value":"response"}"#).is_none());
        assert!(ControlCommand::parse(r#"{"event":"response","value":" "}"#).is_none());
    }

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

    #[test]
    fn leave_names_exactly_the_closing_occasions() -> anyhow::Result<()> {
        use crate::session::Occasion;
        for occasion in [
            Occasion::Lock,
            Occasion::Logout,
            Occasion::Suspend,
            Occasion::Hibernate,
            Occasion::Restart,
            Occasion::Shutdown,
        ] {
            let command = ControlCommand::Leave(occasion);
            assert_eq!(ControlCommand::parse(&command.as_str()), Some(command));
        }
        assert_eq!(
            ControlCommand::parse("leave shutdown"),
            Some(ControlCommand::Leave(Occasion::Shutdown))
        );
        for invalid in ["leave", "leave ", "leave unlock", "leave login", "leave reboot", "leave  lock", "leave Lock"] {
            assert!(ControlCommand::parse(invalid).is_none(), "{invalid:?}");
        }
        let (sender, receiver) = async_channel::bounded(1);
        let (server, mut client) = UnixStream::pair()?;
        client.write_all(b"leave lock\n")?;
        handle_client(server, &sender);
        assert_eq!(receiver.try_recv()?, ControlCommand::Leave(Occasion::Lock));
        let mut response = String::new();
        BufReader::new(client).read_line(&mut response)?;
        assert_eq!(response, "queued\n");
        Ok(())
    }

    #[test]
    fn launcher_navigation_tokens_are_exact() {
        for (token, navigation) in [
            ("launcher-up", LauncherNavigation::Up),
            ("launcher-down", LauncherNavigation::Down),
            ("launcher-activate", LauncherNavigation::Activate),
            ("launcher-dismiss", LauncherNavigation::Dismiss),
        ] {
            assert_eq!(
                ControlCommand::parse(token),
                Some(ControlCommand::LauncherNavigation(navigation))
            );
            assert!(ControlCommand::parse(&format!("{token} ")).is_none());
        }
        assert!(ControlCommand::parse("launcher-next").is_none());
    }

    #[test]
    fn launcher_navigation_command_is_delivered_and_acknowledged() -> anyhow::Result<()> {
        let (sender, receiver) = async_channel::bounded(1);
        let (server, mut client) = UnixStream::pair()?;
        client.write_all(b"launcher-activate\n")?;
        handle_client(server, &sender);
        assert_eq!(
            receiver.try_recv()?,
            ControlCommand::LauncherNavigation(LauncherNavigation::Activate)
        );
        let mut response = String::new();
        BufReader::new(client).read_line(&mut response)?;
        assert_eq!(response, "queued\n");
        Ok(())
    }
}
