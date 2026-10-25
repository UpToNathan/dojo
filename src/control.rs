use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context as _, bail};
use async_channel::Sender;

/// How long a farewell request waits for the closing's timeline to end. Its caller
/// holds a logind delay inhibitor (5 s by default), so this stays well under that.
pub const FAREWELL_WAIT: Duration = Duration::from_millis(2500);

/// Whether the shell has a closing whose timeline is still playing, or a farewell
/// request it has not yet looked at. The shell clears it; a farewell request sets it
/// before queueing and waits for it to clear.
pub static CLOSING_PLAYING: AtomicBool = AtomicBool::new(false);

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
    /// Open the launcher straight into one command's view.
    LauncherView(crate::launcher_core::commands::CommandId),
    LauncherNavigation(LauncherNavigation),
    Theme,
    Displays,
    Wifi,
    SystemMenu,
    Models,
    Terminal,
    TerminalClosed,
    /// Close the frame for an occasion, then run its action.
    Leave(crate::session::action::Departure),
    /// Close the frame for an occasion whose action something else performs, and
    /// reply once the closing has played out.
    Farewell(crate::session::Occasion),
    Speech(crate::speech::SpeechEvent),
    /// Open the panel that sends a window to another machine.
    SendWindow,
}

impl ControlCommand {
    fn as_str(&self) -> String {
        match self {
            Self::Speech(event) => {
                return serde_json::to_string(event).expect("speech events serialize");
            }
            Self::LauncherView(command) => return format!("launcher-view {}", command.key()),
            Self::Leave(departure) => return format!("leave {}", departure.word()),
            Self::Farewell(occasion) => return format!("farewell {}", occasion.key()),
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
            Self::SendWindow => "send-window",
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
            "send-window" => Some(Self::SendWindow),
            _ => {
                use crate::session::action::Departure;
                if let Some(key) = value.strip_prefix("launcher-view ") {
                    return crate::launcher_core::commands::CommandId::from_key(key)
                        .map(Self::LauncherView);
                }
                if let Some(word) = value.strip_prefix("leave ") {
                    return Departure::parse(word).map(Self::Leave);
                }
                if let Some(word) = value.strip_prefix("farewell ") {
                    return Departure::parse_farewell(word).map(Self::Farewell);
                }
                serde_json::from_str::<crate::speech::SpeechEvent>(value)
                    .ok()
                    .filter(|event| event.validate().is_ok())
                    .map(Self::Speech)
            }
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
    let mut reader = BufReader::new(stream);
    reader
        .read_line(&mut reply)
        .context("reading Dojo control response")?;
    if reply.trim() != "queued" {
        bail!("Dojo rejected control command: {}", reply.trim());
    }
    if matches!(command, ControlCommand::Farewell(_)) {
        reader
            .get_ref()
            .set_read_timeout(Some(FAREWELL_WAIT + Duration::from_millis(500)))?;
        reply.clear();
        if reader.read_line(&mut reply).is_err() || reply.trim() != "settled" {
            eprintln!("Dojo did not report the farewell finished; continuing");
        }
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
                        handle_client(stream, &sender, &CLOSING_PLAYING);
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

/// Blocks until `playing` clears or `bound` passes, whichever is first.
fn wait_until_settled(playing: &AtomicBool, bound: Duration) {
    let deadline = Instant::now() + bound;
    while playing.load(Ordering::Acquire) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
}

fn handle_client(
    mut stream: UnixStream,
    sender: &Sender<ControlCommand>,
    playing: &'static AtomicBool,
) {
    if let Err(error) = stream.set_read_timeout(Some(Duration::from_secs(1))) {
        eprintln!("Failed to set Dojo control timeout: {error}");
        return;
    }
    let mut command = String::new();
    let result = BufReader::new((&mut stream).take(16_385)).read_line(&mut command);
    let mut farewell = false;
    let response = match result {
        Ok(_) if command.len() <= 16_384 && command.ends_with('\n') => {
            match ControlCommand::parse(command.trim()) {
                Some(command) => {
                    farewell = matches!(command, ControlCommand::Farewell(_));
                    if farewell {
                        playing.store(true, Ordering::Release);
                    }
                    if sender.try_send(command).is_ok() {
                        "queued\n"
                    } else {
                        if farewell {
                            playing.store(false, Ordering::Release);
                            farewell = false;
                        }
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
        return;
    }
    if farewell {
        thread::spawn(move || {
            wait_until_settled(playing, FAREWELL_WAIT);
            let _ = stream.write_all(b"settled\n");
        });
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
            ("send-window", ControlCommand::SendWindow),
        ] {
            assert_eq!(ControlCommand::parse(token), Some(expected.clone()));
            let (sender, receiver) = async_channel::bounded(1);
            let (server, mut client) = UnixStream::pair()?;
            client.write_all(format!("{token}\n").as_bytes())?;
            handle_client(server, &sender, &CLOSING_PLAYING);
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
        handle_client(server, &sender, &CLOSING_PLAYING);
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
        handle_client(server, &sender, &CLOSING_PLAYING);
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
        handle_client(server, &sender, &CLOSING_PLAYING);
        assert_eq!(receiver.try_recv()?, ControlCommand::Launcher);
        let mut response = String::new();
        BufReader::new(client).read_line(&mut response)?;
        assert_eq!(response, "queued\n");
        Ok(())
    }

    #[test]
    fn leave_names_exactly_the_closing_departures() -> anyhow::Result<()> {
        use crate::session::Occasion;
        use crate::session::action::{Action, Departure};
        for word in [
            "lock",
            "logout",
            "suspend",
            "suspend-then-hibernate",
            "hibernate",
            "restart",
            "shutdown",
        ] {
            let command = ControlCommand::Leave(Departure::parse(word).unwrap());
            assert_eq!(command.as_str(), format!("leave {word}"));
            assert_eq!(ControlCommand::parse(&command.as_str()), Some(command));
        }
        assert_eq!(
            ControlCommand::parse("leave shutdown"),
            Some(ControlCommand::Leave(
                Departure::of(Occasion::Shutdown).unwrap()
            ))
        );
        let Some(ControlCommand::Leave(departure)) =
            ControlCommand::parse("leave suspend-then-hibernate")
        else {
            panic!("suspend-then-hibernate is a departure");
        };
        assert_eq!(departure.occasion(), Occasion::Suspend);
        assert_eq!(departure.action(), Action::SuspendThenHibernate);
        for invalid in [
            "leave",
            "leave ",
            "leave unlock",
            "leave login",
            "leave reboot",
            "leave  lock",
            "leave Lock",
        ] {
            assert!(ControlCommand::parse(invalid).is_none(), "{invalid:?}");
        }
        let (sender, receiver) = async_channel::bounded(1);
        let (server, mut client) = UnixStream::pair()?;
        client.write_all(b"leave lock\n")?;
        handle_client(server, &sender, &CLOSING_PLAYING);
        assert_eq!(
            receiver.try_recv()?,
            ControlCommand::Leave(Departure::of(Occasion::Lock).unwrap())
        );
        let mut response = String::new();
        BufReader::new(client).read_line(&mut response)?;
        assert_eq!(response, "queued\n");
        Ok(())
    }

    #[test]
    fn farewell_names_exactly_the_occasions_that_end_the_session_or_the_machine() {
        use crate::session::Occasion;
        for occasion in [
            Occasion::Logout,
            Occasion::Suspend,
            Occasion::Hibernate,
            Occasion::Restart,
            Occasion::Shutdown,
        ] {
            let command = ControlCommand::Farewell(occasion);
            assert_eq!(command.as_str(), format!("farewell {}", occasion.key()));
            assert_eq!(ControlCommand::parse(&command.as_str()), Some(command));
        }
        for invalid in [
            "farewell",
            "farewell ",
            "farewell lock",
            "farewell unlock",
            "farewell login",
            "farewell suspend-then-hibernate",
            "farewell  shutdown",
            "farewell Shutdown",
            "farewell reboot",
        ] {
            assert!(ControlCommand::parse(invalid).is_none(), "{invalid:?}");
        }
    }

    #[test]
    fn a_farewell_is_acknowledged_at_once_and_settled_when_the_closing_has_played()
    -> anyhow::Result<()> {
        static PLAYING: AtomicBool = AtomicBool::new(false);
        let (sender, receiver) = async_channel::bounded(1);
        let (server, client) = UnixStream::pair()?;
        let mut writer = client.try_clone()?;
        writer.write_all(b"farewell shutdown\n")?;
        handle_client(server, &sender, &PLAYING);
        assert_eq!(
            receiver.try_recv()?,
            ControlCommand::Farewell(crate::session::Occasion::Shutdown)
        );
        assert!(
            PLAYING.load(Ordering::Acquire),
            "set before the shell looks at it"
        );
        let mut reader = BufReader::new(client);
        let mut line = String::new();
        reader.read_line(&mut line)?;
        assert_eq!(line, "queued\n");
        reader
            .get_ref()
            .set_read_timeout(Some(Duration::from_millis(100)))?;
        line.clear();
        assert!(
            reader.read_line(&mut line).is_err(),
            "not settled while playing"
        );
        PLAYING.store(false, Ordering::Release);
        reader
            .get_ref()
            .set_read_timeout(Some(Duration::from_secs(2)))?;
        reader.read_line(&mut line)?;
        assert_eq!(line, "settled\n");
        Ok(())
    }

    #[test]
    fn a_farewell_to_a_stopping_shell_leaves_nothing_playing() -> anyhow::Result<()> {
        static PLAYING: AtomicBool = AtomicBool::new(false);
        let (sender, receiver) = async_channel::bounded(1);
        drop(receiver);
        let (server, mut client) = UnixStream::pair()?;
        client.write_all(b"farewell restart\n")?;
        handle_client(server, &sender, &PLAYING);
        let mut response = String::new();
        BufReader::new(client).read_line(&mut response)?;
        assert_eq!(response, "stopping\n");
        assert!(!PLAYING.load(Ordering::Acquire));
        Ok(())
    }

    #[test]
    fn waiting_for_a_closing_is_bounded() {
        let playing = AtomicBool::new(true);
        let started = Instant::now();
        wait_until_settled(&playing, Duration::from_millis(60));
        assert!(started.elapsed() >= Duration::from_millis(60));
        assert!(started.elapsed() < Duration::from_secs(2));
        playing.store(false, Ordering::Release);
        let started = Instant::now();
        wait_until_settled(&playing, Duration::from_secs(5));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn the_farewell_wait_stays_well_under_loginds_inhibit_delay() {
        assert!(FAREWELL_WAIT + Duration::from_millis(500) <= Duration::from_secs(3));
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
    fn launcher_view_commands_name_a_known_command_exactly() {
        use crate::launcher_core::commands::{ALL, CommandId};
        for command in ALL {
            let token = format!("launcher-view {}", command.id.key());
            assert_eq!(
                ControlCommand::parse(&token),
                Some(ControlCommand::LauncherView(command.id))
            );
            assert_eq!(ControlCommand::LauncherView(command.id).as_str(), token);
        }
        assert_eq!(CommandId::from_key("emoji"), Some(CommandId::Emoji));
        for bad in [
            "launcher-view",
            "launcher-view ",
            "launcher-view nonsense",
            "launcher-view emoji ",
        ] {
            assert!(ControlCommand::parse(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn launcher_navigation_command_is_delivered_and_acknowledged() -> anyhow::Result<()> {
        let (sender, receiver) = async_channel::bounded(1);
        let (server, mut client) = UnixStream::pair()?;
        client.write_all(b"launcher-activate\n")?;
        handle_client(server, &sender, &CLOSING_PLAYING);
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
