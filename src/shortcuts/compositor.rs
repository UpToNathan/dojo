//! The compositor's side of the portal: a connection to Niri that holds this service's grabs and
//! reports when one is pressed or released.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use anyhow::{Context as _, bail};
use niri_ipc::socket::{RetirementHandle, Socket};
use niri_ipc::{Event, GlobalShortcut, Request, Response};

use super::registry::Grab;

const IPC_TIMEOUT: Duration = Duration::from_secs(5);

/// A grab pressed or released.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyEvent {
    pub grab_id: String,
    pub pressed: bool,
    pub time_ms: u64,
}

/// A grab the compositor would not take, with its reason.
pub type Refusal = (String, String);

pub trait Compositor: Send + Sync + 'static {
    /// Replace every grab this service holds. Blocks for the compositor's reply.
    fn set_grabs(&self, grabs: &[Grab]) -> anyhow::Result<Vec<Refusal>>;

    /// For each key (in Niri's syntax), why this service could not grab it, or `None` if it
    /// could. Grabs nothing.
    fn check(&self, keys: &[String]) -> anyhow::Result<Vec<Option<String>>>;
}

pub struct NiriLink {
    control: Mutex<Socket>,
    _events: RetirementHandle,
}

impl NiriLink {
    /// Connect to the running Niri through `NIRI_SOCKET`.
    pub fn connect() -> anyhow::Result<(Self, async_channel::Receiver<KeyEvent>)> {
        let path = std::env::var_os(niri_ipc::socket::SOCKET_PATH_ENV)
            .map(PathBuf::from)
            .with_context(|| {
                format!("{} is not set; is this running in Niri?", niri_ipc::socket::SOCKET_PATH_ENV)
            })?;
        Self::connect_to(&path)
    }

    pub fn connect_to(path: &Path) -> anyhow::Result<(Self, async_channel::Receiver<KeyEvent>)> {
        let mut control = Socket::connect_to(path).context("connecting to Niri")?;
        control.set_read_timeout(Some(IPC_TIMEOUT))?;
        control.set_write_timeout(Some(IPC_TIMEOUT))?;
        // An empty set teaches us the owner id before any event can name it.
        let owner = match request(&mut control, Vec::new())? {
            Response::GlobalShortcuts(grants) => grants.owner,
            other => bail!("Niri answered a shortcut request with {other:?}"),
        };

        let mut events = Socket::connect_to(path).context("connecting to Niri's event stream")?;
        events.set_read_timeout(None)?;
        events.set_write_timeout(Some(IPC_TIMEOUT))?;
        let retirement = events.retirement_handle()?;
        match events.send(Request::EventStream)?.map_err(anyhow::Error::msg)? {
            Response::Handled => {}
            other => bail!("Niri answered EventStream with {other:?}"),
        }
        let (sender, receiver) = async_channel::unbounded();
        thread::Builder::new()
            .name("dojo-shortcut-events".into())
            .spawn(move || {
                let mut read_event = events.read_events();
                while let Ok(event) = read_event() {
                    let (event_owner, id, time, pressed) = match event {
                        Event::ShortcutActivated { owner, id, time } => (owner, id, time, true),
                        Event::ShortcutDeactivated { owner, id, time } => (owner, id, time, false),
                        _ => continue,
                    };
                    if event_owner != owner {
                        continue;
                    }
                    let time_ms = time.secs * 1000 + u64::from(time.nanos) / 1_000_000;
                    let event = KeyEvent {
                        grab_id: id,
                        pressed,
                        time_ms,
                    };
                    if sender.send_blocking(event).is_err() {
                        break;
                    }
                }
                // Dropping the sender tells the service Niri is gone.
            })
            .context("starting the shortcut event listener")?;
        Ok((
            Self {
                control: Mutex::new(control),
                _events: retirement,
            },
            receiver,
        ))
    }
}

fn request(socket: &mut Socket, grabs: Vec<GlobalShortcut>) -> anyhow::Result<Response> {
    socket
        .send(Request::GlobalShortcuts { shortcuts: grabs })
        .context("talking to Niri")?
        .map_err(anyhow::Error::msg)
}

impl Compositor for NiriLink {
    fn set_grabs(&self, grabs: &[Grab]) -> anyhow::Result<Vec<Refusal>> {
        let shortcuts = grabs
            .iter()
            .map(|grab| GlobalShortcut {
                id: grab.id.clone(),
                key: grab.key.clone(),
            })
            .collect();
        let mut control = self.control.lock().expect("Niri control lock poisoned");
        match request(&mut control, shortcuts)? {
            Response::GlobalShortcuts(grants) => Ok(grants
                .refused
                .into_iter()
                .map(|refused| (refused.id, refused.reason))
                .collect()),
            other => bail!("Niri answered a shortcut request with {other:?}"),
        }
    }

    fn check(&self, keys: &[String]) -> anyhow::Result<Vec<Option<String>>> {
        let mut control = self.control.lock().expect("Niri control lock poisoned");
        let reply = control
            .send(Request::CheckGlobalShortcuts {
                keys: keys.to_vec(),
            })
            .context("talking to Niri")?
            .map_err(anyhow::Error::msg)?;
        match reply {
            Response::GlobalShortcutChecks(checks) if checks.len() == keys.len() => Ok(checks),
            other => bail!("Niri answered a shortcut check with {other:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;

    use niri_ipc::{GlobalShortcutGrants, RefusedShortcut, Reply, Timestamp};

    use super::*;

    fn send_line(stream: &mut impl Write, value: &impl serde::Serialize) {
        let mut line = serde_json::to_string(value).unwrap();
        line.push('\n');
        stream.write_all(line.as_bytes()).unwrap();
    }

    /// A stand-in for Niri: one connection of requests, one event stream.
    fn fake_niri(dir: &Path) -> (PathBuf, thread::JoinHandle<Vec<Vec<GlobalShortcut>>>) {
        let path = dir.join("niri.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let handle = thread::spawn(move || {
            let (control, _) = listener.accept().unwrap();
            let mut events_stream = None;
            let mut control_reader = BufReader::new(control.try_clone().unwrap());
            let mut control = control;
            let mut seen = Vec::new();

            // The owner probe, then one real request.
            for round in 0..2 {
                let mut line = String::new();
                control_reader.read_line(&mut line).unwrap();
                let Request::GlobalShortcuts { shortcuts } = serde_json::from_str(&line).unwrap()
                else {
                    panic!("unexpected request {line}");
                };
                let refused = shortcuts
                    .iter()
                    .filter(|shortcut| shortcut.key == "Ctrl+F1")
                    .map(|shortcut| RefusedShortcut {
                        id: shortcut.id.clone(),
                        reason: "already a Niri key binding".into(),
                    })
                    .collect();
                seen.push(shortcuts);
                let reply: Reply = Ok(Response::GlobalShortcuts(GlobalShortcutGrants {
                    owner: 42,
                    refused,
                }));
                send_line(&mut control, &reply);
                if round == 0 {
                    // The event connection follows the probe.
                    let (mut events, _) = listener.accept().unwrap();
                    let mut line = String::new();
                    BufReader::new(events.try_clone().unwrap())
                        .read_line(&mut line)
                        .unwrap();
                    assert!(matches!(
                        serde_json::from_str(&line).unwrap(),
                        Request::EventStream
                    ));
                    send_line(&mut events, &Reply::Ok(Response::Handled));
                    let time = Timestamp {
                        secs: 5,
                        nanos: 250_000_000,
                    };
                    for event in [
                        // Someone else's grab, and unrelated state, must be ignored.
                        Event::ShortcutActivated {
                            owner: 7,
                            id: "1/theirs".into(),
                            time,
                        },
                        Event::TouchpadToggled { on: true },
                        Event::ShortcutActivated {
                            owner: 42,
                            id: "1/ptt".into(),
                            time,
                        },
                        Event::ShortcutDeactivated {
                            owner: 42,
                            id: "1/ptt".into(),
                            time,
                        },
                    ] {
                        send_line(&mut events, &event);
                    }
                    events_stream = Some(events);
                }
            }
            // A check follows: two keys, the second unavailable.
            let mut line = String::new();
            control_reader.read_line(&mut line).unwrap();
            let Request::CheckGlobalShortcuts { keys } = serde_json::from_str(&line).unwrap()
            else {
                panic!("expected a check, got {line}");
            };
            assert_eq!(keys, ["Ctrl+F5", "Ctrl+F6"]);
            let reply: Reply = Ok(Response::GlobalShortcutChecks(vec![
                None,
                Some("already a Niri key binding".into()),
            ]));
            send_line(&mut control, &reply);
            // Dropping the event stream closes it.
            drop(events_stream);
            seen
        });
        (path, handle)
    }

    #[test]
    fn carries_grabs_out_and_only_our_events_back() {
        let dir = tempfile::tempdir().unwrap();
        let (path, niri) = fake_niri(dir.path());
        let (link, events) = NiriLink::connect_to(&path).unwrap();

        let refused = link
            .set_grabs(&[
                Grab {
                    id: "1/ptt".into(),
                    key: "Ctrl+Alt+K".into(),
                },
                Grab {
                    id: "1/bad".into(),
                    key: "Ctrl+F1".into(),
                },
            ])
            .unwrap();
        assert_eq!(refused, [("1/bad".to_owned(), "already a Niri key binding".to_owned())]);

        let pressed = events.recv_blocking().unwrap();
        assert_eq!(
            pressed,
            KeyEvent {
                grab_id: "1/ptt".into(),
                pressed: true,
                time_ms: 5250
            }
        );
        assert!(!events.recv_blocking().unwrap().pressed);

        let checks = link
            .check(&["Ctrl+F5".to_owned(), "Ctrl+F6".to_owned()])
            .unwrap();
        assert_eq!(checks, [None, Some("already a Niri key binding".to_owned())]);

        let seen = niri.join().unwrap();
        assert!(seen[0].is_empty(), "the first request only learns the owner");
        assert_eq!(seen[1][0].key, "Ctrl+Alt+K");
        // Niri closing its event stream ends ours.
        assert!(events.recv_blocking().is_err());
    }
}
