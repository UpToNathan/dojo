use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{DesktopAction, DesktopSession, validate_name};

const VERSION: u32 = 1;
const MAX_LINE: u64 = 65536;
const EMPTY_GRACE: Duration = Duration::from_millis(250);

#[derive(Deserialize, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum Message {
    Hello {
        version: u32,
        name: String,
        idle_timeout_seconds: u64,
    },
    Action {
        action: DesktopAction,
    },
    Status {},
    Release {},
}

type Reply = std::result::Result<Value, String>;

pub(super) struct ActiveSession {
    stream: BufReader<UnixStream>,
    pid: u32,
    pub(super) seat_name: Option<String>,
}

pub(super) fn paths(socket: &Path) -> Result<(PathBuf, PathBuf)> {
    let runtime =
        PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR is required")?);
    let directory = runtime.join("dojo-desktop");
    match std::fs::DirBuilder::new().mode(0o700).create(&directory) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    let metadata = std::fs::symlink_metadata(&directory)?;
    if !metadata.is_dir() || metadata.permissions().mode() & 0o777 != 0o700 {
        bail!("desktop service directory must be a private directory with mode 0700");
    }
    let digest = Sha256::digest(socket.as_os_str().as_encoded_bytes());
    let key = format!("{digest:x}");
    Ok((
        directory.join(format!("{}.sock", &key[..24])),
        directory.join(format!("{}.lock", &key[..24])),
    ))
}

fn read_message<T: serde::de::DeserializeOwned>(reader: &mut impl BufRead) -> Result<Option<T>> {
    let mut line = Vec::new();
    let length = reader.take(MAX_LINE + 1).read_until(b'\n', &mut line)?;
    if length == 0 {
        return Ok(None);
    }
    if length as u64 > MAX_LINE || line.last() != Some(&b'\n') {
        bail!("desktop message exceeds limit or is incomplete");
    }
    Ok(Some(serde_json::from_slice(&line)?))
}

fn write_message(writer: &mut impl Write, value: &impl Serialize) -> Result<()> {
    serde_json::to_writer(&mut *writer, value)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

impl ActiveSession {
    pub(super) fn spawn(socket: Option<&Path>, name: &str, idle_timeout: Duration) -> Result<Self> {
        let socket = super::niri_socket(socket)?;
        let (path, _) = paths(&socket)?;
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut last_spawn = None;
        loop {
            match UnixStream::connect(&path) {
                Ok(stream) => {
                    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
                    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
                    let mut session = Self {
                        stream: BufReader::new(stream),
                        pid: 0,
                        seat_name: None,
                    };
                    match session.request(&Message::Hello {
                        version: VERSION,
                        name: name.into(),
                        idle_timeout_seconds: idle_timeout.as_secs(),
                    }) {
                        Ok(hello) => {
                            session.pid = hello["service_pid"]
                                .as_u64()
                                .context("missing service PID")?
                                .try_into()?;
                            return Ok(session);
                        }
                        Err(error)
                            if Instant::now() < deadline
                                && error.downcast_ref::<std::io::Error>().is_some() => {}
                        Err(error) => return Err(error).context("initialize desktop session"),
                    }
                }
                Err(error)
                    if Instant::now() < deadline
                        && matches!(
                            error.kind(),
                            std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                        ) =>
                {
                    if last_spawn
                        .is_none_or(|last: Instant| last.elapsed() >= Duration::from_millis(100))
                    {
                        let mut child = Command::new(format!("/proc/{}/exe", std::process::id()))
                            .arg("--service")
                            .arg("--socket")
                            .arg(&socket)
                            .process_group(0)
                            .stdin(Stdio::null())
                            .stdout(Stdio::null())
                            .stderr(Stdio::inherit())
                            .spawn()
                            .context("start desktop service")?;
                        std::thread::spawn(move || {
                            let _ = child.wait();
                        });
                        last_spawn = Some(Instant::now());
                    }
                }
                Err(error) => return Err(error).context("connect to desktop service"),
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn request(&mut self, message: &Message) -> Result<Value> {
        write_message(self.stream.get_mut(), message)?;
        let reply: Reply = read_message(&mut self.stream)?.ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "desktop service stopped; select a fresh target before continuing",
            )
        })?;
        reply.map_err(anyhow::Error::msg)
    }

    pub(super) fn pid(&self) -> u32 {
        self.pid
    }

    pub(super) fn inactive(&mut self) -> Result<bool> {
        match self.request(&Message::Status {}) {
            Ok(value) => {
                self.seat_name = value["seat_name"].as_str().map(str::to_owned);
                Ok(self.seat_name.is_none())
            }
            Err(_) => Ok(true),
        }
    }

    pub(super) fn action(&mut self, action: DesktopAction) -> Result<Value> {
        let value = self.request(&Message::Action { action })?;
        self.seat_name = value["seat_name"].as_str().map(str::to_owned);
        Ok(value)
    }

    pub(super) fn release(mut self) {
        let _ = self.request(&Message::Release {});
    }
}

struct Session {
    desktop: DesktopSession,
    idle_timeout: Duration,
    last_action: Instant,
}

impl Session {
    fn handle(&mut self, message: Message) -> Result<Value> {
        match message {
            Message::Hello { .. } => bail!("session is already initialized"),
            Message::Status {} => Ok(
                json!({"seat_name": self.desktop.controller.as_ref().map(|controller| controller.seat_name())}),
            ),
            Message::Release {} => {
                self.desktop.controller = None;
                Ok(json!({"released": true}))
            }
            Message::Action { action } => {
                action.validate()?;
                let result = self.desktop.action(action);
                self.last_action = Instant::now();
                result
            }
        }
    }
}

enum Event {
    Request {
        id: u64,
        message: Message,
        reply: mpsc::SyncSender<Reply>,
    },
    Disconnect(u64),
}

fn connection(stream: UnixStream, id: u64, sender: mpsc::SyncSender<Event>) {
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let mut reader = BufReader::new(stream);
    let result = (|| -> Result<()> {
        while let Some(message) = read_message::<Message>(&mut reader)? {
            let (reply, response) = mpsc::sync_channel(1);
            sender.send(Event::Request { id, message, reply })?;
            let response = response.recv()?;
            write_message(reader.get_mut(), &response)?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        let reply: Reply = Err(format!("{error:#}"));
        let _ = write_message(reader.get_mut(), &reply);
    }
    let _ = sender.send(Event::Disconnect(id));
}

struct SocketFile(PathBuf);
impl Drop for SocketFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

pub(super) fn run(socket: PathBuf) -> Result<()> {
    let (path, lock_path) = paths(&socket)?;
    let lock: File = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(lock_path)?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => return Ok(()),
        Err(error) => return Err(error.into()),
    }
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let listener = UnixListener::bind(&path)?;
    let _socket_file = SocketFile(path.clone());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    let (sender, receiver) = mpsc::sync_channel(64);
    std::thread::spawn(move || {
        for (id, stream) in listener.incoming().enumerate() {
            let Ok(stream) = stream else {
                break;
            };
            let sender = sender.clone();
            std::thread::spawn(move || connection(stream, id as u64, sender));
        }
    });
    let mut sessions: HashMap<u64, Session> = HashMap::new();
    let mut empty_since = Instant::now();
    loop {
        let now = Instant::now();
        for session in sessions.values_mut() {
            if now.duration_since(session.last_action) >= session.idle_timeout {
                session.desktop.controller = None;
            }
        }
        let active = sessions
            .values()
            .any(|session| session.desktop.controller.is_some());
        if active {
            empty_since = now;
        }
        if !active && now.duration_since(empty_since) >= EMPTY_GRACE {
            break;
        }
        let deadline = sessions
            .values()
            .filter(|session| session.desktop.controller.is_some())
            .map(|session| session.last_action + session.idle_timeout)
            .min()
            .unwrap_or(empty_since + EMPTY_GRACE);
        let event = match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now()))
        {
            Ok(event) => event,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        match event {
            Event::Disconnect(id) => {
                sessions.remove(&id);
            }
            Event::Request { id, message, reply } => {
                let result = if let Some(session) = sessions.get_mut(&id) {
                    session.handle(message)
                } else {
                    match message {
                        Message::Hello {
                            version,
                            name,
                            idle_timeout_seconds,
                        } => (|| -> Result<Value> {
                            if version != VERSION {
                                bail!("unsupported desktop protocol version");
                            }
                            validate_name(&name)?;
                            if !(1..=86400).contains(&idle_timeout_seconds) {
                                bail!("idle timeout must be between 1 and 86400 seconds");
                            }
                            sessions.insert(
                                id,
                                Session {
                                    desktop: DesktopSession {
                                        socket: Some(socket.clone()),
                                        name,
                                        controller: None,
                                    },
                                    idle_timeout: Duration::from_secs(idle_timeout_seconds),
                                    last_action: Instant::now(),
                                },
                            );
                            Ok(json!({"version": VERSION, "service_pid": std::process::id()}))
                        })(),
                        _ => Err(anyhow::anyhow!("initialize the session with hello first")),
                    }
                };
                let _ = reply.send(result.map_err(|error| format!("{error:#}")));
            }
        }
    }
    drop(sessions);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_protocol_requires_bounded_complete_messages() {
        let valid = b"{\"operation\":\"status\"}\n";
        assert!(matches!(
            read_message::<Message>(&mut &valid[..]).unwrap(),
            Some(Message::Status {})
        ));
        assert!(read_message::<Message>(&mut &valid[..valid.len() - 1]).is_err());
        assert!(
            read_message::<Message>(&mut &b"{\"operation\":\"status\",\"extra\":1}\n"[..]).is_err()
        );
        assert!(read_message::<Message>(&mut &vec![b' '; MAX_LINE as usize + 1][..]).is_err());
        assert!(read_message::<Message>(&mut &b""[..]).unwrap().is_none());
    }

    #[test]
    fn status_release_and_invalid_actions_do_not_renew_session() {
        let last_action = Instant::now() - Duration::from_secs(5);
        let mut session = Session {
            desktop: DesktopSession {
                socket: Some(PathBuf::from("/missing.sock")),
                name: "test".into(),
                controller: None,
            },
            idle_timeout: Duration::from_secs(2),
            last_action,
        };
        assert_eq!(
            session.handle(Message::Status {}).unwrap(),
            json!({"seat_name":null})
        );
        assert_eq!(
            session.handle(Message::Release {}).unwrap(),
            json!({"released":true})
        );
        assert!(
            session
                .handle(Message::Action {
                    action: DesktopAction::Key {
                        evdev: 0,
                        state: super::super::KeyState::Press
                    }
                })
                .is_err()
        );
        assert_eq!(session.last_action, last_action);
        assert!(session.desktop.controller.is_none());
    }
}
