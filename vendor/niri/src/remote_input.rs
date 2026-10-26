//! A peer's keyboard and pointer, delivered to this desktop as if they were its own hardware.
//!
//! This is the opposite of an agent seat. A seat is a second, sandboxed cursor that cannot touch
//! Niri's binds; remote input takes the real seat, so it must not be available to just any process
//! running as the user (the AI agent is one). The gate is the kernel's process tree: a caller is
//! accepted only if it runs inside an SSH session that a root-owned daemon started for a connection
//! from another machine. No user process can fake that ancestry. Two daemons are recognised:
//! `tailscaled`, whose helper's command line carries the identity Tailscale authenticated
//! (`--remote-user`, `--remote-ip`), and the root `sshd`, whose per-session process holds the
//! connection's socket, from which the remote address is read. The rest of the policy (which nodes
//! may connect at all, and as whom) belongs to the daemon's own configuration.
use std::collections::{BTreeSet, HashMap, HashSet};
use std::net::UdpSocket;
use std::path::PathBuf;

use niri_ipc::{AgentInputState, RemoteInput};
use smithay::backend::input::{
    Axis, AxisRelativeDirection, AxisSource, ButtonState, Device, DeviceCapability, Event,
    InputBackend, InputEvent, InputTime, KeyState, KeyboardKeyEvent, Keycode, PointerAxisEvent,
    PointerButtonEvent, PointerMotionEvent, UnusedEvent,
};
use smithay::output::Output;
use tracing::{info, warn};

use crate::input::backend_ext::NiriInputDevice;
use crate::niri::{LockState, State};

/// The largest Linux key code, `KEY_MAX`.
const KEY_MAX: u32 = 0x2ff;
/// BTN_LEFT to BTN_TASK.
const BUTTONS: std::ops::RangeInclusive<u32> = 0x110..=0x117;
/// The most one event may move or scroll, in logical pixels.
const MAX_STEP: f64 = 10_000.;
/// Events a single request may carry.
const MAX_BATCH: usize = 512;

/// Who a session belongs to, as Tailscale told the SSH helper.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Origin {
    /// The authenticated identity of the connecting node, such as `tag:venus`.
    pub user: String,
    /// Its tailnet address.
    pub ip: String,
}

impl std::fmt::Display for Origin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.user, self.ip)
    }
}

/// What the gate needs to know about processes. `/proc` in production, a table in tests.
pub trait ProcessView {
    fn parent(&self, pid: u32) -> Option<u32>;
    fn uid(&self, pid: u32) -> Option<u32>;
    fn comm(&self, pid: u32) -> Option<String>;
    fn cmdline(&self, pid: u32) -> Option<Vec<String>>;
    /// Whether this address belongs to this machine (so the "remote" end is really us).
    fn is_local_address(&self, address: &str) -> bool;
    /// The remote address of the established TCP connection this process holds open, if any.
    fn peer_address(&self, pid: u32) -> Option<String>;
    /// The remote host `systemd-logind` recorded for the SSH login session this process belongs to.
    /// OpenSSH keeps its connection handle where its own user cannot read it (the worker is not
    /// dumpable), but its PAM session registers the peer with logind, which root owns.
    fn logged_remote_host(&self, pid: u32) -> Option<String>;
}

pub struct Proc;

impl Proc {
    fn read(pid: u32, file: &str) -> Option<Vec<u8>> {
        std::fs::read(PathBuf::from(format!("/proc/{pid}")).join(file)).ok()
    }
}

impl ProcessView for Proc {
    fn parent(&self, pid: u32) -> Option<u32> {
        let stat = String::from_utf8(Self::read(pid, "stat")?).ok()?;
        // The name can contain spaces and parentheses; fields start after the last ')'.
        stat.rsplit_once(')')?.1.split_whitespace().nth(1)?.parse().ok()
    }

    fn uid(&self, pid: u32) -> Option<u32> {
        let status = String::from_utf8(Self::read(pid, "status")?).ok()?;
        let line = status.lines().find(|line| line.starts_with("Uid:"))?;
        // Real uid; the daemon is root throughout.
        line.split_whitespace().nth(1)?.parse().ok()
    }

    fn comm(&self, pid: u32) -> Option<String> {
        Some(String::from_utf8(Self::read(pid, "comm")?).ok()?.trim().to_owned())
    }

    fn cmdline(&self, pid: u32) -> Option<Vec<String>> {
        let bytes = Self::read(pid, "cmdline")?;
        Some(bytes.split(|b| *b == 0).filter(|part| !part.is_empty()).map(|part| String::from_utf8_lossy(part).into_owned()).collect())
    }

    fn is_local_address(&self, address: &str) -> bool {
        // Binding succeeds only for an address one of our interfaces owns.
        address.parse::<std::net::IpAddr>().is_ok_and(|ip| UdpSocket::bind((ip, 0)).is_ok())
    }

    fn logged_remote_host(&self, pid: u32) -> Option<String> {
        let id = login_session_of(&String::from_utf8(Self::read(pid, "cgroup")?).ok()?)?;
        let output = ["/usr/bin/loginctl", "/bin/loginctl"]
            .iter()
            .find(|path| std::path::Path::new(path).exists())
            .and_then(|path| std::process::Command::new(path).args(["show-session", &id, "-p", "Remote", "-p", "RemoteHost", "-p", "Service"]).stdin(std::process::Stdio::null()).output().ok())?;
        if !output.status.success() {
            return None;
        }
        remote_host_of(&String::from_utf8_lossy(&output.stdout))
    }

    fn peer_address(&self, pid: u32) -> Option<String> {
        let inodes: HashSet<u64> = std::fs::read_dir(format!("/proc/{pid}/fd"))
            .ok()?
            .flatten()
            .filter_map(|entry| std::fs::read_link(entry.path()).ok())
            .filter_map(|target| target.to_str()?.strip_prefix("socket:[")?.strip_suffix(']')?.parse().ok())
            .collect();
        ["/proc/net/tcp", "/proc/net/tcp6"].iter().find_map(|table| {
            let text = std::fs::read_to_string(table).ok()?;
            established_peer(&text, &inodes, table.ends_with('6'))
        })
    }
}

/// The login session a process belongs to, from its `/proc/PID/cgroup`: the `session-ID.scope` that
/// `pam_systemd` put it in. Only root can place a process there.
fn login_session_of(cgroup: &str) -> Option<String> {
    cgroup.lines().find_map(|line| {
        let scope = line.rsplit('/').next()?;
        let id = scope.strip_prefix("session-")?.strip_suffix(".scope")?;
        (!id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric())).then(|| id.to_owned())
    })
}

/// The peer logind recorded, from `loginctl show-session -p Remote -p RemoteHost -p Service`: only for
/// a remote session `sshd` opened.
fn remote_host_of(show: &str) -> Option<String> {
    let value = |key: &str| show.lines().find_map(|line| line.strip_prefix(key)?.strip_prefix('=')).map(str::trim);
    (value("Remote")? == "yes" && value("Service")? == "sshd").then(|| value("RemoteHost").filter(|host| !host.is_empty()).map(str::to_owned)).flatten()
}

/// The remote address of an established connection among `inodes`, from the text of
/// `/proc/net/tcp` (`v6` false) or `/proc/net/tcp6`.
fn established_peer(table: &str, inodes: &HashSet<u64>, v6: bool) -> Option<String> {
    table.lines().skip(1).find_map(|line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        // sl local_address rem_address st tx:rx tr:when retrnsmt uid timeout inode
        if fields.len() < 10 || fields[3] != "01" || !inodes.contains(&fields[9].parse::<u64>().ok()?) {
            return None;
        }
        let (host, _) = fields[2].split_once(':')?;
        Some(hex_address(host, v6)?.to_string())
    })
}

/// An address as `/proc/net/tcp*` prints it: each 32-bit word in the machine's byte order.
fn hex_address(hex: &str, v6: bool) -> Option<std::net::IpAddr> {
    let word = |at: usize| u32::from_str_radix(hex.get(at..at + 8)?, 16).ok().map(|value| value.to_ne_bytes());
    if !v6 {
        let [a, b, c, d] = word(0)?;
        return Some(std::net::IpAddr::V4(std::net::Ipv4Addr::new(a, b, c, d)));
    }
    let mut octets = [0u8; 16];
    for index in 0..4 {
        octets[index * 4..index * 4 + 4].copy_from_slice(&word(index * 8)?);
    }
    let ip = std::net::Ipv6Addr::from(octets);
    // A v4 peer on a dual-stack socket shows as ::ffff:a.b.c.d.
    Some(ip.to_ipv4_mapped().map_or(std::net::IpAddr::V6(ip), std::net::IpAddr::V4))
}

/// Finds the Tailscale SSH session `pid` runs in, and the node that connected to it.
///
/// The session's helper (`tailscaled be-child ssh ...`) is a direct child of the root-owned
/// `tailscaled`; the helper's arguments were written by that daemon. Only the root daemon can start
/// such a child, so an ancestor of that shape cannot be forged by a process of this user.
pub fn ssh_origin(view: &impl ProcessView, pid: u32, own_uid: u32) -> Result<Origin, String> {
    let mut child = pid;
    for _ in 0..64 {
        let Some(parent) = view.parent(child) else { break };
        if parent <= 1 {
            break;
        }
        if view.uid(parent) == Some(0) && view.comm(parent).as_deref() == Some("tailscaled") {
            let args = view.cmdline(child).unwrap_or_default();
            let is_ssh_helper = args.iter().any(|arg| arg == "be-child") && args.iter().any(|arg| arg == "ssh");
            if !is_ssh_helper {
                return Err("not inside a Tailscale SSH session".into());
            }
            let value = |name: &str| args.iter().find_map(|arg| arg.strip_prefix(&format!("--{name}="))).map(str::to_owned);
            if value("uid").and_then(|uid| uid.parse::<u32>().ok()) != Some(own_uid) {
                return Err("the SSH session belongs to another user".into());
            }
            let (Some(user), Some(ip)) = (value("remote-user"), value("remote-ip")) else {
                return Err("the SSH session does not say who connected".into());
            };
            if view.is_local_address(&ip) {
                return Err("the SSH session is from this machine itself, not a peer".into());
            }
            return Ok(Origin { user, ip });
        }
        child = parent;
    }
    Err("not inside a Tailscale SSH session".into())
}

/// The names a root-owned `sshd` process goes by: `sshd` before OpenSSH 9.8, `sshd-session` after.
fn is_sshd(comm: Option<String>) -> bool {
    matches!(comm.as_deref(), Some("sshd" | "sshd-session"))
}

/// Finds the OpenSSH session `pid` runs in, and the address that connected to it.
///
/// OpenSSH starts a session as a root-owned `sshd` process (the privilege-separation monitor) whose
/// child runs as the user and holds the connection's socket. A process of this user cannot become a
/// child of a root-owned process, so a caller below such a child is inside a real session. The
/// address comes from that child's socket, not from anything the caller can set, such as
/// `SSH_CONNECTION`. A name given to the session (`dojo-mesh inject --as NAME`, which a peer's
/// `authorized_keys` entry forces) is shown when the caller's command line has one; it only labels
/// the session, and the receiving program decides what to accept.
pub fn sshd_origin(view: &impl ProcessView, pid: u32, own_uid: u32) -> Result<Origin, String> {
    let mut child = pid;
    for _ in 0..64 {
        let Some(parent) = view.parent(child) else { break };
        if parent <= 1 {
            break;
        }
        if view.uid(parent) == Some(0) && is_sshd(view.comm(parent)) {
            if view.uid(child) != Some(own_uid) {
                return Err("the SSH session belongs to another user".into());
            }
            if !is_sshd(view.comm(child)) {
                return Err("not inside an OpenSSH session".into());
            }
            // The connection's own socket when it can be read, else what logind recorded for the session.
            let Some(ip) = view.peer_address(child).or_else(|| view.logged_remote_host(pid)) else {
                return Err("the SSH session does not say who connected".into());
            };
            if view.is_local_address(&ip) {
                return Err("the SSH session is from this machine itself, not a peer".into());
            }
            let args = view.cmdline(pid).unwrap_or_default();
            let name = args.iter().position(|arg| arg == "--as").and_then(|at| args.get(at + 1)).cloned();
            return Ok(Origin { user: name.unwrap_or_else(|| "an OpenSSH session".into()), ip });
        }
        child = parent;
    }
    Err("not inside an OpenSSH session".into())
}

/// The session `pid` runs in, if it is inside one a peer started: Tailscale SSH or OpenSSH.
pub fn session_origin(view: &impl ProcessView, pid: u32, own_uid: u32) -> Result<Origin, String> {
    match ssh_origin(view, pid, own_uid) {
        Ok(origin) => Ok(origin),
        Err(tailscale) => match sshd_origin(view, pid, own_uid) {
            Ok(origin) => Ok(origin),
            // Each gate says why it refused; the one that found a session of its kind is the useful one.
            Err(openssh) if openssh.starts_with("not inside") => Err(format!("not inside an SSH session a peer started ({tailscale}; {openssh})")),
            Err(openssh) => Err(openssh),
        },
    }
}

/// A session's held input, so that ending it can let go.
#[derive(Debug)]
pub struct Session {
    pub origin: Origin,
    keys: BTreeSet<u32>,
    buttons: BTreeSet<u32>,
}

#[derive(Debug, Default)]
pub struct Sessions {
    active: HashMap<u64, Session>,
    /// Connections whose input was cut off on this machine; they stay cut off.
    stopped: HashSet<u64>,
}

impl Sessions {
    pub fn is_empty(&self) -> bool {
        self.active.is_empty()
    }
}

pub struct RemoteInputBackend;

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct RemoteInputDevice;

impl Device for RemoteInputDevice {
    fn id(&self) -> String {
        String::from("remote-input")
    }
    fn name(&self) -> String {
        String::from("remote input")
    }
    fn has_capability(&self, capability: DeviceCapability) -> bool {
        matches!(capability, DeviceCapability::Keyboard | DeviceCapability::Pointer)
    }
    fn usb_id(&self) -> Option<(u32, u32)> {
        None
    }
    fn syspath(&self) -> Option<PathBuf> {
        None
    }
}

impl NiriInputDevice for RemoteInputDevice {
    fn output(&self, _state: &State) -> Option<Output> {
        None
    }
}

pub struct RemoteKeyEvent {
    code: Keycode,
    state: KeyState,
    count: u32,
}
pub struct RemoteMotionEvent {
    dx: f64,
    dy: f64,
}
pub struct RemoteButtonEvent {
    code: u32,
    state: ButtonState,
}
pub struct RemoteAxisEvent {
    horizontal: f64,
    vertical: f64,
}

macro_rules! remote_event {
    ($($event:ty),*) => {$(
        impl Event<RemoteInputBackend> for $event {
            fn time(&self) -> InputTime {
                InputTime::now()
            }
            fn device(&self) -> RemoteInputDevice {
                RemoteInputDevice
            }
        }
    )*};
}
remote_event!(RemoteKeyEvent, RemoteMotionEvent, RemoteButtonEvent, RemoteAxisEvent);

impl KeyboardKeyEvent<RemoteInputBackend> for RemoteKeyEvent {
    fn key_code(&self) -> Keycode {
        self.code
    }
    fn state(&self) -> KeyState {
        self.state
    }
    fn count(&self) -> u32 {
        self.count
    }
}

impl PointerMotionEvent<RemoteInputBackend> for RemoteMotionEvent {
    fn delta_x(&self) -> f64 {
        self.dx
    }
    fn delta_y(&self) -> f64 {
        self.dy
    }
    // The sender already applied its own acceleration, so there is no separate raw delta.
    fn delta_x_unaccel(&self) -> f64 {
        self.dx
    }
    fn delta_y_unaccel(&self) -> f64 {
        self.dy
    }
}

impl PointerButtonEvent<RemoteInputBackend> for RemoteButtonEvent {
    fn button_code(&self) -> u32 {
        self.code
    }
    fn state(&self) -> ButtonState {
        self.state
    }
}

impl PointerAxisEvent<RemoteInputBackend> for RemoteAxisEvent {
    fn amount(&self, axis: Axis) -> Option<f64> {
        Some(match axis {
            Axis::Horizontal => self.horizontal,
            Axis::Vertical => self.vertical,
        })
    }
    fn amount_v120(&self, _axis: Axis) -> Option<f64> {
        None
    }
    fn source(&self) -> AxisSource {
        AxisSource::Continuous
    }
    fn relative_direction(&self, _axis: Axis) -> AxisRelativeDirection {
        AxisRelativeDirection::Identical
    }
}

impl InputBackend for RemoteInputBackend {
    type Device = RemoteInputDevice;

    type KeyboardKeyEvent = RemoteKeyEvent;
    type PointerAxisEvent = RemoteAxisEvent;
    type PointerButtonEvent = RemoteButtonEvent;
    type PointerMotionEvent = RemoteMotionEvent;
    type PointerMotionAbsoluteEvent = UnusedEvent;

    type GestureSwipeBeginEvent = UnusedEvent;
    type GestureSwipeUpdateEvent = UnusedEvent;
    type GestureSwipeEndEvent = UnusedEvent;
    type GesturePinchBeginEvent = UnusedEvent;
    type GesturePinchUpdateEvent = UnusedEvent;
    type GesturePinchEndEvent = UnusedEvent;
    type GestureHoldBeginEvent = UnusedEvent;
    type GestureHoldEndEvent = UnusedEvent;

    type TouchDownEvent = UnusedEvent;
    type TouchUpEvent = UnusedEvent;
    type TouchMotionEvent = UnusedEvent;
    type TouchCancelEvent = UnusedEvent;
    type TouchFrameEvent = UnusedEvent;
    type TabletToolAxisEvent = UnusedEvent;
    type TabletToolProximityEvent = UnusedEvent;
    type TabletToolTipEvent = UnusedEvent;
    type TabletToolButtonEvent = UnusedEvent;

    type SwitchToggleEvent = UnusedEvent;

    type SpecialEvent = UnusedEvent;
}

fn validate(event: &RemoteInput) -> Result<(), String> {
    match event {
        RemoteInput::Key { evdev, .. } if !(1..=KEY_MAX).contains(evdev) => {
            Err(format!("evdev keyboard code must be between 1 and {KEY_MAX}"))
        }
        RemoteInput::Button { button, .. } if !BUTTONS.contains(button) => {
            Err("button must be a Linux mouse button code between 272 and 279".into())
        }
        RemoteInput::Motion { dx, dy } if !(dx.is_finite() && dy.is_finite() && dx.abs() <= MAX_STEP && dy.abs() <= MAX_STEP) => {
            Err("pointer motion must be finite and at most 10000 pixels".into())
        }
        RemoteInput::Axis { horizontal, vertical } if !(horizontal.is_finite() && vertical.is_finite() && horizontal.abs() <= MAX_STEP && vertical.abs() <= MAX_STEP) => {
            Err("scroll amounts must be finite and at most 10000 pixels".into())
        }
        _ => Ok(()),
    }
}

impl State {
    /// `Request::RemoteInput`: checks who is asking, then delivers the events.
    pub(crate) fn request_remote_input(
        &mut self,
        peer: u64,
        owner: Option<&crate::agent_input::peer::KernelPeer>,
        events: Vec<RemoteInput>,
    ) -> Result<(), String> {
        if self.niri.remote_input.stopped.contains(&peer) {
            return Err("remote input ended: it was stopped on this machine".into());
        }
        if !matches!(self.niri.lock_state, LockState::Unlocked) {
            self.end_remote_input(peer);
            return Err("remote input ended: the screen is locked".into());
        }
        if events.len() > MAX_BATCH {
            return Err(format!("at most {MAX_BATCH} events per request"));
        }
        for event in &events {
            validate(event)?;
        }
        if !self.niri.remote_input.active.contains_key(&peer) {
            let owner = owner.filter(|owner| owner.is_live()).ok_or("could not identify the process asking")?;
            let origin = session_origin(&Proc, owner.pid(), unsafe { libc::geteuid() }).map_err(|reason| format!("remote input refused: {reason}"))?;
            self.begin_remote_session(peer, origin);
        }
        for event in events {
            self.deliver_remote_input(peer, event);
        }
        Ok(())
    }

    /// Starts a session for an origin that has already been vetted.
    pub(crate) fn begin_remote_session(&mut self, peer: u64, origin: Origin) {
        info!("remote input from {origin} started");
        self.niri.remote_input.active.insert(peer, Session { origin, keys: BTreeSet::new(), buttons: BTreeSet::new() });
    }

    /// Delivers vetted events for a session that has begun.
    pub(crate) fn deliver_remote_events(&mut self, peer: u64, events: Vec<RemoteInput>) {
        for event in events {
            self.deliver_remote_input(peer, event);
        }
    }

    /// A key from this machine's own hardware: the stop chord ends remote control.
    pub(crate) fn hardware_key_for_remote_input(&mut self, evdev: u32, pressed: bool) {
        if !pressed || evdev != 1 || self.niri.remote_input.is_empty() {
            return;
        }
        let mods = self.niri.seat.get_keyboard().unwrap().modifier_state();
        if mods.ctrl && mods.alt && mods.shift {
            self.stop_all_remote_input("Ctrl+Alt+Shift+Esc on this machine's own keyboard");
        }
    }

    fn deliver_remote_input(&mut self, peer: u64, event: RemoteInput) {
        let Some(session) = self.niri.remote_input.active.get_mut(&peer) else { return };
        match event {
            RemoteInput::Key { evdev, state } => {
                let pressed = state == AgentInputState::Pressed;
                // A press of a key already down, or a release of one never pressed, would confuse
                // the keyboard state; both are dropped.
                if pressed == session.keys.contains(&evdev) {
                    return;
                }
                if pressed {
                    session.keys.insert(evdev);
                } else {
                    session.keys.remove(&evdev);
                }
                let count = session.keys.len() as u32;
                let event = RemoteKeyEvent {
                    code: Keycode::from(evdev + 8),
                    state: if pressed { KeyState::Pressed } else { KeyState::Released },
                    count,
                };
                self.process_input_event(InputEvent::<RemoteInputBackend>::Keyboard { event });
            }
            RemoteInput::Button { button, state } => {
                let pressed = state == AgentInputState::Pressed;
                if pressed == session.buttons.contains(&button) {
                    return;
                }
                if pressed {
                    session.buttons.insert(button);
                } else {
                    session.buttons.remove(&button);
                }
                let event = RemoteButtonEvent {
                    code: button,
                    state: if pressed { ButtonState::Pressed } else { ButtonState::Released },
                };
                self.process_input_event(InputEvent::<RemoteInputBackend>::PointerButton { event });
            }
            RemoteInput::Motion { dx, dy } => {
                self.process_input_event(InputEvent::<RemoteInputBackend>::PointerMotion { event: RemoteMotionEvent { dx, dy } });
            }
            RemoteInput::Axis { horizontal, vertical } => {
                self.process_input_event(InputEvent::<RemoteInputBackend>::PointerAxis { event: RemoteAxisEvent { horizontal, vertical } });
            }
        }
    }

    /// Lets go of everything a connection held, and forgets it. The connection may start again.
    pub fn end_remote_input(&mut self, peer: u64) {
        let Some(session) = self.niri.remote_input.active.remove(&peer) else { return };
        info!("remote input from {} ended", session.origin);
        for evdev in session.keys {
            let event = RemoteKeyEvent { code: Keycode::from(evdev + 8), state: KeyState::Released, count: 0 };
            self.process_input_event(InputEvent::<RemoteInputBackend>::Keyboard { event });
        }
        for button in session.buttons {
            let event = RemoteButtonEvent { code: button, state: ButtonState::Released };
            self.process_input_event(InputEvent::<RemoteInputBackend>::PointerButton { event });
        }
    }

    /// Ends every session and refuses those connections from now on. This is the person at this
    /// machine saying no, so it also holds against a sender that tries again.
    pub fn stop_all_remote_input(&mut self, reason: &str) {
        let peers: Vec<u64> = self.niri.remote_input.active.keys().copied().collect();
        if peers.is_empty() {
            return;
        }
        warn!("remote input stopped: {reason}");
        for peer in peers {
            self.end_remote_input(peer);
            self.niri.remote_input.stopped.insert(peer);
        }
    }

    /// Called as the screen locks, and periodically: nothing remote may type at a lock screen.
    pub fn refresh_remote_input(&mut self) {
        if !self.niri.remote_input.is_empty() && !matches!(self.niri.lock_state, LockState::Unlocked) {
            self.stop_all_remote_input("the screen locked");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Table {
        // pid -> (parent, uid, comm, cmdline)
        rows: HashMap<u32, (u32, u32, &'static str, Vec<&'static str>)>,
        local: Vec<&'static str>,
        // pid -> the remote address of the connection it holds
        peers: HashMap<u32, &'static str>,
        // pid -> the remote host logind recorded for its login session
        logged: HashMap<u32, &'static str>,
    }

    impl ProcessView for Table {
        fn parent(&self, pid: u32) -> Option<u32> {
            self.rows.get(&pid).map(|row| row.0)
        }
        fn uid(&self, pid: u32) -> Option<u32> {
            self.rows.get(&pid).map(|row| row.1)
        }
        fn comm(&self, pid: u32) -> Option<String> {
            self.rows.get(&pid).map(|row| row.2.to_owned())
        }
        fn cmdline(&self, pid: u32) -> Option<Vec<String>> {
            self.rows.get(&pid).map(|row| row.3.iter().map(|part| part.to_string()).collect())
        }
        fn is_local_address(&self, address: &str) -> bool {
            self.local.contains(&address)
        }
        fn peer_address(&self, pid: u32) -> Option<String> {
            self.peers.get(&pid).map(|address| address.to_string())
        }
        fn logged_remote_host(&self, pid: u32) -> Option<String> {
            self.logged.get(&pid).map(|address| address.to_string())
        }
    }

    const HELPER: [&str; 8] = [
        "/usr/bin/tailscaled",
        "be-child",
        "ssh",
        "--uid=1000",
        "--local-user=nathan",
        "--remote-user=tag:venus",
        "--remote-ip=100.71.60.101",
        "--has-tty=false",
    ];

    /// niri(1) <- systemd(1) ; tailscaled(10, root) <- helper(20, user) <- shell(30) <- inject(40)
    fn ssh_session() -> Table {
        let mut table = Table::default();
        table.rows.insert(10, (1, 0, "tailscaled", vec!["/usr/bin/tailscaled"]));
        table.rows.insert(20, (10, 1000, "tailscaled", HELPER.to_vec()));
        table.rows.insert(30, (20, 1000, "bash", vec!["bash"]));
        table.rows.insert(40, (30, 1000, "dojo-mesh", vec!["dojo-mesh", "inject"]));
        table
    }

    #[test]
    fn a_process_inside_a_peers_ssh_session_is_identified_by_what_tailscale_recorded() {
        let origin = ssh_origin(&ssh_session(), 40, 1000).unwrap();
        assert_eq!(origin, Origin { user: "tag:venus".into(), ip: "100.71.60.101".into() });
        assert_eq!(origin.to_string(), "tag:venus (100.71.60.101)");
    }

    #[test]
    fn a_process_outside_any_ssh_session_is_refused() {
        let mut table = ssh_session();
        // The agent's own shell: user processes up to the user's systemd and pid 1.
        table.rows.insert(50, (1, 1000, "systemd", vec!["systemd", "--user"]));
        table.rows.insert(51, (50, 1000, "claude", vec!["claude"]));
        table.rows.insert(52, (51, 1000, "dojo-mesh", vec!["dojo-mesh", "inject"]));
        assert!(ssh_origin(&table, 52, 1000).unwrap_err().contains("not inside a Tailscale SSH session"));
    }

    #[test]
    fn a_user_process_posing_as_tailscaled_is_not_a_root_ancestor() {
        let mut table = Table::default();
        // Same names and arguments, but every process belongs to the user: forged.
        table.rows.insert(10, (1, 1000, "tailscaled", vec!["tailscaled"]));
        table.rows.insert(20, (10, 1000, "tailscaled", HELPER.to_vec()));
        table.rows.insert(40, (20, 1000, "dojo-mesh", vec!["dojo-mesh"]));
        assert!(ssh_origin(&table, 40, 1000).is_err());
    }

    #[test]
    fn an_ssh_session_from_this_machine_is_not_a_peer() {
        let mut table = ssh_session();
        table.local.push("100.71.60.101");
        assert!(ssh_origin(&table, 40, 1000).unwrap_err().contains("from this machine itself"));
    }

    #[test]
    fn a_session_for_another_user_or_without_an_identity_is_refused() {
        assert!(ssh_origin(&ssh_session(), 40, 1001).unwrap_err().contains("another user"));
        let mut table = ssh_session();
        table.rows.get_mut(&20).unwrap().3 = vec!["/usr/bin/tailscaled", "be-child", "ssh", "--uid=1000"];
        assert!(ssh_origin(&table, 40, 1000).unwrap_err().contains("does not say who connected"));
    }

    #[test]
    fn a_child_of_the_daemon_that_is_not_an_ssh_helper_is_refused() {
        let mut table = ssh_session();
        table.rows.get_mut(&20).unwrap().3 = vec!["/usr/bin/tailscaled", "serve"];
        assert!(ssh_origin(&table, 40, 1000).is_err());
    }

    /// systemd(1) ; sshd(10, root) <- sshd-session [priv](11, root) <- sshd-session user@notty(12, user) <- bash(30) <- inject(40)
    fn sshd_session() -> Table {
        let mut table = Table::default();
        table.rows.insert(10, (1, 0, "sshd", vec!["sshd"]));
        table.rows.insert(11, (10, 0, "sshd-session", vec!["sshd-session: nathan [priv]"]));
        table.rows.insert(12, (11, 1000, "sshd-session", vec!["sshd-session: nathan@notty"]));
        table.rows.insert(30, (12, 1000, "bash", vec!["bash", "-c", "/home/nathan/.local/bin/dojo-mesh inject --as venus"]));
        table.rows.insert(40, (30, 1000, "dojo-mesh", vec!["/home/nathan/.local/bin/dojo-mesh", "inject", "--as", "venus"]));
        table.peers.insert(12, "203.0.113.5");
        table
    }

    #[test]
    fn a_process_inside_an_openssh_session_is_identified_by_the_connections_socket() {
        let origin = sshd_origin(&sshd_session(), 40, 1000).unwrap();
        assert_eq!(origin, Origin { user: "venus".into(), ip: "203.0.113.5".into() });
        // Without a name on the command line the session is still a session, unnamed.
        let mut table = sshd_session();
        table.rows.get_mut(&40).unwrap().3 = vec!["dojo-mesh", "inject"];
        assert_eq!(sshd_origin(&table, 40, 1000).unwrap().user, "an OpenSSH session");
        // The combined gate finds it too.
        assert_eq!(session_origin(&sshd_session(), 40, 1000).unwrap().ip, "203.0.113.5");
    }

    #[test]
    fn an_openssh_session_whose_socket_cannot_be_read_is_attributed_by_what_logind_recorded() {
        let mut table = sshd_session();
        table.peers.clear();
        table.logged.insert(40, "203.0.113.5");
        assert_eq!(sshd_origin(&table, 40, 1000).unwrap(), Origin { user: "venus".into(), ip: "203.0.113.5".into() });
        table.local.push("203.0.113.5");
        assert!(sshd_origin(&table, 40, 1000).unwrap_err().contains("from this machine itself"));
    }

    #[test]
    fn the_login_session_and_its_remote_host_are_read_only_from_what_root_recorded() {
        assert_eq!(login_session_of("0::/user.slice/user-1000.slice/session-20.scope\n").as_deref(), Some("20"));
        assert_eq!(login_session_of("0::/user.slice/user-1000.slice/user@1000.service/app.slice/x.service\n"), None);
        assert_eq!(login_session_of("0::/user.slice/session-.scope"), None);
        assert_eq!(login_session_of("0::/user.slice/session-2;rm.scope"), None);
        let show = "Remote=yes\nRemoteHost=100.71.60.101\nService=sshd\n";
        assert_eq!(remote_host_of(show).as_deref(), Some("100.71.60.101"));
        // A local graphical session, a remote one that is not sshd's, or one with no host says nothing.
        assert_eq!(remote_host_of("Remote=no\nRemoteHost=\nService=dojo-login\n"), None);
        assert_eq!(remote_host_of("Remote=yes\nRemoteHost=198.51.100.1\nService=cockpit\n"), None);
        assert_eq!(remote_host_of("Remote=yes\nRemoteHost=\nService=sshd\n"), None);
    }

    #[test]
    fn an_openssh_session_from_this_machine_or_for_another_user_or_unattributable_is_refused() {
        let mut table = sshd_session();
        table.local.push("203.0.113.5");
        assert!(sshd_origin(&table, 40, 1000).unwrap_err().contains("from this machine itself"));
        assert!(sshd_origin(&sshd_session(), 40, 1001).unwrap_err().contains("another user"));
        let mut table = sshd_session();
        table.peers.clear();
        assert!(sshd_origin(&table, 40, 1000).unwrap_err().contains("does not say who connected"));
    }

    #[test]
    fn a_user_process_posing_as_sshd_is_not_a_root_ancestor() {
        let mut table = Table::default();
        table.rows.insert(10, (1, 1000, "sshd", vec!["sshd"]));
        table.rows.insert(11, (10, 1000, "sshd-session", vec!["sshd-session: nathan [priv]"]));
        table.rows.insert(12, (11, 1000, "sshd-session", vec!["sshd-session: nathan@notty"]));
        table.rows.insert(40, (12, 1000, "dojo-mesh", vec!["dojo-mesh", "inject", "--as", "venus"]));
        table.peers.insert(12, "203.0.113.5");
        assert!(session_origin(&table, 40, 1000).is_err());
    }

    #[test]
    fn a_caller_outside_every_session_is_refused_with_both_reasons() {
        let mut table = sshd_session();
        table.rows.insert(50, (1, 1000, "systemd", vec!["systemd", "--user"]));
        table.rows.insert(52, (50, 1000, "dojo-mesh", vec!["dojo-mesh", "inject", "--as", "venus"]));
        let error = session_origin(&table, 52, 1000).unwrap_err();
        assert!(error.contains("not inside an SSH session a peer started"), "{error}");
        // A root sshd's own non-session descendants (not the user's child) do not count.
        let mut table = sshd_session();
        table.rows.insert(60, (11, 1000, "bash", vec!["bash"]));
        table.rows.insert(61, (60, 1000, "dojo-mesh", vec!["dojo-mesh", "inject"]));
        assert!(sshd_origin(&table, 61, 1000).unwrap_err().contains("not inside an OpenSSH session"));
    }

    #[test]
    fn the_remote_address_is_read_from_the_established_connection_a_process_holds() {
        let v4 = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n   \
            0: 0100007F:0016 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 11111 1 0\n   \
            1: 0A01A8C0:0016 0500CB0B:C350 01 00000000:00000000 02:000A7214 00000000     0        0 22222 2 0\n";
        let held: HashSet<u64> = [22222].into();
        assert_eq!(established_peer(v4, &held, false).as_deref(), Some("11.203.0.5"));
        // A listening socket, or one the process does not hold, says nothing.
        assert_eq!(established_peer(v4, &[11111].into(), false), None);
        assert_eq!(established_peer(v4, &[99999].into(), false), None);
        let v6 = "  sl  local_address                         remote_address                        st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n   \
            0: 0000000000000000FFFF00000A01A8C0:0016 0000000000000000FFFF00000500CB0B:C350 01 00000000:00000000 02:000A7214 00000000     0        0 33333 2 0\n   \
            1: 000080FE0000000000000000010000FD:0016 000080FE00000000000000000200A2D1:C350 01 00000000:00000000 02:000A7214 00000000     0        0 44444 2 0\n";
        assert_eq!(established_peer(v6, &[33333].into(), true).as_deref(), Some("11.203.0.5"));
        assert_eq!(established_peer(v6, &[44444].into(), true).as_deref(), Some("fe80::d1a2:2"));
    }

    #[test]
    fn events_outside_the_valid_ranges_are_rejected() {
        let key = |evdev| RemoteInput::Key { evdev, state: AgentInputState::Pressed };
        assert!(validate(&key(0)).is_err());
        assert!(validate(&key(1)).is_ok() && validate(&key(0x1d0)).is_ok() && validate(&key(KEY_MAX)).is_ok());
        assert!(validate(&key(KEY_MAX + 1)).is_err());
        assert!(validate(&RemoteInput::Button { button: 0x110, state: AgentInputState::Pressed }).is_ok());
        assert!(validate(&RemoteInput::Button { button: 0x118, state: AgentInputState::Pressed }).is_err());
        assert!(validate(&RemoteInput::Motion { dx: f64::NAN, dy: 0. }).is_err());
        assert!(validate(&RemoteInput::Motion { dx: 20_000., dy: 0. }).is_err());
        assert!(validate(&RemoteInput::Axis { horizontal: 0., vertical: f64::INFINITY }).is_err());
    }
}
