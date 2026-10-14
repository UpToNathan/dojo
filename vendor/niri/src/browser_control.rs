//! Browser Control's extension transport. Only this input actor writes frames.
use crate::{
    niri::{LockState, State},
    utils::with_toplevel_role,
};
use anyhow::{ensure, Context};
use calloop::{
    generic::Generic,
    timer::{TimeoutAction, Timer},
    Interest, LoopHandle, Mode, PostAction,
};
use dojo_browser_control::{unix, write_frame, Event, Gate, Request};
use smithay::reexports::wayland_server::{backend::ClientId, Resource};
use std::os::unix::net::UnixDatagram;
use std::{
    cell::RefCell,
    fs, io,
    net::TcpStream,
    os::{
        fd::OwnedFd,
        unix::fs::{MetadataExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

pub(crate) struct Channel {
    gate: Gate,
    writer: Option<OwnedFd>,
    broker: Option<(i32, u64)>,
    reply: Option<PathBuf>,
    socket: Rc<UnixDatagram>,
    guarded_clients: Vec<ClientId>,
}
struct SocketPath(PathBuf);
impl Drop for SocketPath {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
impl Channel {
    fn events(&self, events: Vec<Event>) {
        for event in events {
            self.event(&event);
        }
    }
    fn event(&self, event: &Event) {
        if let (Some(reply), Ok(bytes)) = (&self.reply, serde_json::to_vec(event)) {
            let _ = self.socket.send_to(&bytes, reply);
        }
    }
    fn disconnect(&mut self) {
        let events = self.gate.disconnect();
        self.events(events);
        if let Some(writer) = self.writer.take() {
            use std::os::fd::AsRawFd;
            unsafe {
                libc::shutdown(writer.as_raw_fd(), libc::SHUT_RDWR);
            }
        }
    }
    fn retire_all(&mut self) {
        let events = self.gate.takeover();
        self.events(events);
    }
}
fn browser_app(app: Option<&str>) -> bool {
    matches!(
        app.map(str::to_ascii_lowercase).as_deref(),
        Some(
            "helium"
                | "net.imput.helium"
                | "chromium"
                | "chromium-browser"
                | "google-chrome"
                | "google-chrome-beta"
                | "google-chrome-unstable"
                | "com.google.chrome"
        )
    )
}
impl State {
    fn browser_clients(&self) -> Vec<ClientId> {
        let mut clients = Vec::new();
        self.niri.layout.with_windows(|window, output, _, _| {
            if output.is_some()
                && window.credentials().is_some()
                && with_toplevel_role(window.toplevel(), |role| {
                    browser_app(role.app_id.as_deref())
                })
            {
                if let Some(client) = window.toplevel().wl_surface().client() {
                    if !clients.contains(&client.id()) {
                        clients.push(client.id());
                    }
                }
            }
        });
        clients
    }
    fn browser_permitted(&self) -> bool {
        if !self.niri.agent_input_enabled || !matches!(self.niri.lock_state, LockState::Unlocked) {
            return false;
        }
        let clients = self.browser_clients();
        !clients.is_empty()
            && !clients
                .iter()
                .any(|client| self.niri.input_coordination.human_owns(client))
    }
    pub(crate) fn browser_human_activity(&mut self, client: Option<&ClientId>) {
        let known = client.is_some_and(|client| {
            self.browser_clients().contains(client)
                || self
                    .niri
                    .browser_control
                    .as_ref()
                    .is_some_and(|channel| channel.borrow().guarded_clients.contains(client))
        });
        if known {
            if let Some(channel) = &self.niri.browser_control {
                channel.borrow_mut().retire_all();
            }
        }
    }
    fn browser_tick(&mut self) {
        let permitted = self.browser_permitted();
        let clients = self.browser_clients();
        if let Some(channel) = &self.niri.browser_control {
            let mut channel = channel.borrow_mut();
            if !channel.guarded_clients.is_empty()
                && (channel.guarded_clients.len() != clients.len()
                    || channel
                        .guarded_clients
                        .iter()
                        .any(|client| !clients.contains(client)))
            {
                channel.retire_all();
            }
            channel.guarded_clients = clients;
            if let Some((pid, start)) = channel.broker {
                if process_start(pid).ok() != Some(start) {
                    channel.disconnect();
                    return;
                }
            }
            let Channel { gate, writer, .. } = &mut *channel;
            let Some(writer) = writer.as_ref() else {
                return;
            };
            let events = gate.dispatch(permitted, |frame| write_frame(writer, frame));
            channel.events(events);
            if channel.gate.is_disconnected() {
                channel.disconnect();
            }
        }
    }
}
fn process_start(pid: i32) -> io::Result<u64> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat"))?;
    stat.rsplit_once(')')
        .and_then(|(_, rest)| rest.split_whitespace().nth(19))
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "process start unavailable"))
}
fn immutable_broker(path: &Path) -> anyhow::Result<PathBuf> {
    let path = fs::canonicalize(path)?;
    let meta = fs::metadata(&path)?;
    ensure!(
        path.starts_with("/nix/store")
            && meta.uid() == 0
            && meta.mode() & 0o222 == 0
            && meta.is_file(),
        "broker must be an immutable root-owned Nix store executable"
    );
    Ok(path)
}
fn extension_socket(fd: OwnedFd) -> anyhow::Result<OwnedFd> {
    let socket = TcpStream::from(fd);
    dojo_browser_control::configure_socket_buffers(&socket)?;
    let peer = socket.peer_addr()?;
    let local = socket.local_addr()?;
    ensure!(
        peer.ip().is_loopback() && local.ip().is_loopback() && local.port() == 19988,
        "not the extension transport socket"
    );
    let peer_entry = format!("0100007F:{:04X}", peer.port());
    let local_entry = format!("0100007F:{:04X}", local.port());
    let table = fs::read_to_string("/proc/net/tcp")?;
    ensure!(
        table.len() < 1024 * 1024,
        "socket table exceeds bounded attestation budget"
    );
    let uid = unsafe { libc::geteuid() };
    ensure!(
        table.lines().skip(1).any(|line| {
            let columns: Vec<_> = line.split_whitespace().collect();
            columns.get(1) == Some(&peer_entry.as_str())
                && columns.get(2) == Some(&local_entry.as_str())
                && columns.get(7).and_then(|s| s.parse::<u32>().ok()) == Some(uid)
        }),
        "extension peer is not same UID"
    );
    socket.set_nonblocking(true)?;
    Ok(socket.into())
}
pub fn start(
    handle: &LoopHandle<'static, State>,
    path: PathBuf,
    broker: PathBuf,
) -> anyhow::Result<()> {
    let broker = immutable_broker(&broker)?;
    if let Ok(metadata) = fs::symlink_metadata(&path) {
        use std::os::unix::fs::FileTypeExt;
        ensure!(
            metadata.uid() == unsafe { libc::geteuid() } && metadata.file_type().is_socket(),
            "admission path is not an owned socket"
        );
        let probe = UnixDatagram::unbound()?;
        match probe.connect(&path) {
            Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => {
                fs::remove_file(&path)?
            }
            _ => anyhow::bail!("admission socket is already active"),
        }
    }
    let socket = UnixDatagram::bind(&path).context("bind browser admission")?;
    dojo_browser_control::configure_socket_buffers(&socket)?;
    let cleanup = SocketPath(path.clone());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    socket.set_nonblocking(true)?;
    unix::enable_credentials(&socket)?;
    let socket = Rc::new(socket);
    let receive = socket.try_clone()?;
    let channel = Rc::new(RefCell::new(Channel {
        gate: Gate::default(),
        writer: None,
        broker: None,
        reply: None,
        socket: socket.clone(),
        guarded_clients: Vec::new(),
    }));
    let incoming = channel.clone();
    handle.insert_source(
        Generic::new(receive, Interest::READ, Mode::Level),
        move |_, socket, state| {
            let _keep_path = &cleanup;
            let packet = unix::receive(socket);
            let Ok(Some((request, fd, credentials, reply))) = packet else {
                return Ok(PostAction::Continue);
            };
            if credentials.uid != unsafe { libc::geteuid() }
                || fs::read_link(format!("/proc/{}/exe", credentials.pid))
                    .ok()
                    .as_ref()
                    != Some(&broker)
            {
                return Ok(PostAction::Continue);
            }
            let Ok(start) = process_start(credentials.pid) else {
                return Ok(PostAction::Continue);
            };
            let mut channel = incoming.borrow_mut();
            if matches!(request, Request::Register { .. }) {
                if channel.writer.is_some() {
                    let _ = socket.send_to(
                        &serde_json::to_vec(&Event::result(
                            0,
                            "refused",
                            Some("channel already registered"),
                        ))
                        .unwrap(),
                        reply,
                    );
                    return Ok(PostAction::Continue);
                }
                let Some(fd) = fd else {
                    return Ok(PostAction::Continue);
                };
                let Ok(fd) = extension_socket(fd) else {
                    return Ok(PostAction::Continue);
                };
                // A fresh connection cannot discard old-generation tombstones.
                channel.gate.reconnect();
                channel.writer = Some(fd);
                channel.broker = Some((credentials.pid, start));
                channel.reply = Some(reply);
                channel.event(&Event::result(0, "registered", None));
                state.niri.browser_control = Some(incoming.clone());
            } else {
                if fd.is_some()
                    || channel.broker != Some((credentials.pid, start))
                    || channel.reply.as_ref() != Some(&reply)
                {
                    return Ok(PostAction::Continue);
                }
                match request {
                    Request::Bind {
                        id,
                        lease,
                        generation,
                    } => {
                        let result = if state.browser_permitted() {
                            channel.gate.bind(lease, generation)
                        } else {
                            Err("human browser ownership, missing identity or locked/disabled seat")
                        };
                        match result {
                            Ok(()) => channel.event(&Event::result(id, "bound", None)),
                            Err(reason) => {
                                channel.event(&Event::result(id, "refused", Some(reason)))
                            }
                        }
                    }
                    Request::Send {
                        id,
                        lease,
                        generation,
                        command,
                    } => {
                        if let Err(reason) = channel.gate.enqueue(id, lease, generation, command) {
                            channel.event(&Event::result(id, "refused", Some(reason)));
                        }
                    }
                    Request::Retire {
                        id,
                        lease,
                        generation,
                    } => {
                        let events = channel.gate.retire(&lease, generation);
                        channel.events(events);
                        channel.event(&Event::result(id, "retired", None));
                    }
                    Request::Settle {
                        extension_id,
                        completed,
                    } => {
                        let events = channel.gate.settle(extension_id, completed);
                        channel.events(events);
                    }
                    Request::Pong { payload } => {
                        if channel.gate.pong(payload).is_err() {
                            channel.disconnect();
                        }
                    }
                    Request::Disconnect => channel.disconnect(),
                    Request::Register { .. } => unreachable!(),
                }
            }
            drop(channel);
            state.browser_tick();
            Ok(PostAction::Continue)
        },
    )?;
    handle
        .insert_source(
            Timer::from_duration(Duration::from_millis(5)),
            move |_, _, state| {
                state.browser_tick();
                TimeoutAction::ToDuration(Duration::from_millis(5))
            },
        )
        .map_err(|error| anyhow::anyhow!("browser timer registration: {}", error.error))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{
        fixture::Fixture,
        test_input_backend::{TestInputBackend, TestKeyboardKeyEvent},
    };
    use smithay::backend::input::{InputEvent, InputTime, KeyState, Keycode};
    use std::{io::Read, os::unix::net::UnixStream};
    fn fixture() -> (
        Fixture,
        crate::tests::client::ClientId,
        wayland_client::protocol::wl_surface::WlSurface,
        UnixStream,
    ) {
        let mut f = Fixture::with_agent_input();
        f.add_output(1, (800, 600));
        let client = f.add_client();
        let window = f.client(client).create_window();
        window.xdg_toplevel.set_app_id("helium".into());
        let surface = window.surface.clone();
        window.commit();
        f.double_roundtrip(client);
        let window = f.client(client).window(&surface);
        window.attach_new_buffer();
        window.ack_last_and_commit();
        f.double_roundtrip(client);
        f.niri_complete_animations();
        f.niri_state().yield_human_input().unwrap();
        let (writer, reader) = UnixStream::pair().unwrap();
        reader.set_nonblocking(true).unwrap();
        let (socket, _) = UnixDatagram::pair().unwrap();
        let mut gate = Gate::default();
        gate.bind("task".into(), 1).unwrap();
        f.niri_state().niri.browser_control = Some(Rc::new(RefCell::new(Channel {
            gate,
            writer: Some(writer.into()),
            broker: None,
            reply: None,
            socket: Rc::new(socket),
            guarded_clients: Vec::new(),
        })));
        (f, client, surface, reader)
    }
    fn enqueue(f: &mut Fixture, id: u64) {
        f.niri().browser_control.as_ref().unwrap().borrow_mut().gate.enqueue(id,"task".into(),1,serde_json::json!({"id":id,"method":"Runtime.evaluate","dojoLease":"task","dojoGeneration":1})).unwrap();
    }
    fn human_key(f: &mut Fixture, state: KeyState) {
        f.niri_state()
            .process_input_event(InputEvent::<TestInputBackend>::Keyboard {
                event: TestKeyboardKeyEvent {
                    time: InputTime::now(),
                    code: Keycode::from(48_u32 + 8),
                    state,
                    count: if state == KeyState::Pressed { 1 } else { 0 },
                },
            });
    }
    #[test]
    fn native_takeover_before_dispatch_drops_command_and_delivers_human_key() {
        let (mut f, client, surface, mut reader) = fixture();
        enqueue(&mut f, 7);
        human_key(&mut f, KeyState::Pressed);
        f.niri_state().browser_tick();
        let mut bytes = [0u8; 4096];
        let n = reader.read(&mut bytes).unwrap();
        let text = String::from_utf8_lossy(&bytes[..n]);
        assert!(text.contains("dojo.retire"));
        assert!(!text.contains("Runtime.evaluate"));
        f.double_roundtrip(client);
        assert!(f
            .client(client)
            .state
            .recent_keyboard_events(&surface)
            .any(|event| matches!(
                event,
                crate::tests::client::KeyboardEvent::Key { key: 48, .. }
            )));
    }
    #[test]
    fn native_takeover_after_dispatch_rejects_queued_follow_on() {
        let (mut f, _, _, mut reader) = fixture();
        enqueue(&mut f, 7);
        enqueue(&mut f, 8);
        f.niri_state().browser_tick();
        let mut bytes = [0u8; 4096];
        let n = reader.read(&mut bytes).unwrap();
        assert!(String::from_utf8_lossy(&bytes[..n]).contains("Runtime.evaluate"));
        human_key(&mut f, KeyState::Pressed);
        f.niri_state().browser_tick();
        let n = reader.read(&mut bytes).unwrap();
        assert!(String::from_utf8_lossy(&bytes[..n]).contains("dojo.retire"));
        f.niri()
            .browser_control
            .as_ref()
            .unwrap()
            .borrow_mut()
            .gate
            .settle(7, true);
        f.niri_state().browser_tick();
        assert_eq!(
            reader.read(&mut bytes).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
    }
    #[test]
    fn native_repeated_human_activity_tombstones_even_without_revision_change() {
        let (mut f, _, _, _) = fixture();
        human_key(&mut f, KeyState::Pressed);
        let revision = f.niri().input_coordination.revision;
        // Inject a new generation to exercise the same-owner activity hook.
        f.niri()
            .browser_control
            .as_ref()
            .unwrap()
            .borrow_mut()
            .gate
            .bind("task".into(), 2)
            .unwrap();
        f.niri().browser_control.as_ref().unwrap().borrow_mut().gate.enqueue(7,"task".into(),2,serde_json::json!({"id":7,"method":"Runtime.evaluate","dojoLease":"task","dojoGeneration":2})).unwrap();
        human_key(&mut f, KeyState::Released);
        assert_eq!(f.niri().input_coordination.revision, revision);
        assert!(f.niri().browser_control.as_ref().unwrap().borrow_mut().gate.enqueue(8,"task".into(),2,serde_json::json!({"id":8,"method":"Runtime.evaluate","dojoLease":"task","dojoGeneration":2})).is_err());
    }
    #[test]
    fn human_typing_in_t3_does_not_revoke_yielded_browser_client() {
        let (mut f, _, _, mut reader) = fixture();
        let client = f.add_client();
        let window = f.client(client).create_window();
        window.xdg_toplevel.set_app_id("t3code".into());
        let surface = window.surface.clone();
        window.commit();
        f.double_roundtrip(client);
        let window = f.client(client).window(&surface);
        window.attach_new_buffer();
        window.ack_last_and_commit();
        f.double_roundtrip(client);
        f.niri_complete_animations();
        enqueue(&mut f, 7);
        human_key(&mut f, KeyState::Pressed);
        f.niri_state().browser_tick();
        let mut bytes = [0u8; 4096];
        let n = reader.read(&mut bytes).unwrap();
        assert!(String::from_utf8_lossy(&bytes[..n]).contains("Runtime.evaluate"));
        f.double_roundtrip(client);
        assert!(f
            .client(client)
            .state
            .recent_keyboard_events(&surface)
            .any(|event| matches!(
                event,
                crate::tests::client::KeyboardEvent::Key { key: 48, .. }
            )));
    }
}
