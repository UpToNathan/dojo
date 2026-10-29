use std::collections::{BTreeMap, HashMap};
use std::sync::{
    Arc, RwLock, Weak,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use anyhow::bail;
use futures_lite::StreamExt;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
use zbus::{Connection, MessageStream, Proxy};

const NM: &str = "org.freedesktop.NetworkManager";
const ROOT: &str = "/org/freedesktop/NetworkManager";
const WIRELESS: &str = "org.freedesktop.NetworkManager.Device.Wireless";
const DEVICE: &str = "org.freedesktop.NetworkManager.Device";
const PROFILE: &str = "org.freedesktop.NetworkManager.Settings.Connection";
const SECURITY: &str = "802-11-wireless-security";
const READ_LIMIT: Duration = Duration::from_secs(12);
const ACTION_LIMIT: Duration = Duration::from_secs(45);
const POLL: Duration = Duration::from_secs(5);
type Settings = HashMap<String, HashMap<String, OwnedValue>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Availability {
    Unavailable(String),
    Disabled,
    Enabled,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Security {
    Open,
    Personal,
    Enterprise,
    Unsupported,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct NetworkId {
    device: String,
    ssid: Vec<u8>,
    security: Security,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Network {
    pub id: NetworkId,
    pub label: String,
    pub signal: u8,
    pub security: Security,
    pub connected: bool,
    pub saved: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    SetEnabled,
    Connect,
    Disconnect,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectionKind {
    Wifi,
    Ethernet { usb: bool },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveConnection {
    pub kind: ConnectionKind,
    pub label: String,
    pub interface: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub availability: Availability,
    pub active_connection: Option<ActiveConnection>,
    pub networks: Vec<Network>,
    pub pending: Option<Operation>,
    pub scanning: bool,
    pub password_required: Option<NetworkId>,
    pub error: Option<String>,
    pub observation_error: Option<String>,
}
const CHECKING: &str = "Checking Wi-Fi…";
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            availability: Availability::Unavailable(CHECKING.into()),
            active_connection: None,
            networks: vec![],
            pending: None,
            scanning: false,
            password_required: None,
            error: None,
            observation_error: None,
        }
    }
}
impl Snapshot {
    /// Whether NetworkManager has been read: not the placeholder before the first read, and not
    /// a failed read. Only a settled snapshot says anything about the active connection.
    fn is_settled(&self) -> bool {
        self.observation_error.is_none()
            && !matches!(&self.availability, Availability::Unavailable(m) if m == CHECKING)
    }
    pub fn connection_title(&self) -> &'static str {
        match self
            .active_connection
            .as_ref()
            .map(|connection| &connection.kind)
        {
            Some(ConnectionKind::Ethernet { usb: true }) => "USB Ethernet",
            Some(ConnectionKind::Ethernet { usb: false }) => "Ethernet",
            _ => "Wi-Fi",
        }
    }
    pub fn connection_label(&self) -> String {
        match &self.active_connection {
            Some(connection) => format!(
                "Connected · {} · {}",
                connection.label, connection.interface
            ),
            None => "No active internet connection".into(),
        }
    }
    pub fn icon(&self) -> &'static str {
        match self
            .active_connection
            .as_ref()
            .map(|connection| &connection.kind)
        {
            Some(ConnectionKind::Ethernet { .. }) => "\u{f0200}",
            _ if matches!(self.availability, Availability::Enabled) => "\u{f05a9}",
            _ => "\u{f05aa}",
        }
    }
}
// Actions intentionally have no Debug implementation because Connect carries a password.
pub enum Action {
    SetEnabled(bool),
    Scan,
    Connect(NetworkId, Option<String>),
    Disconnect,
    CancelPassword,
}
enum Event {
    Action(Action),
    Refresh,
}
struct Shared {
    snapshot: RwLock<Arc<Snapshot>>,
    events: async_channel::Sender<Event>,
    changed: async_channel::Sender<()>,
    mutation: AtomicBool,
    scan: AtomicBool,
    refresh: AtomicBool,
}
#[derive(Clone)]
pub struct Controller(Arc<Shared>);
impl Controller {
    pub fn start() -> anyhow::Result<(Self, async_channel::Receiver<()>)> {
        Self::start_with_address(None)
    }
    fn start_with_address(
        address: Option<String>,
    ) -> anyhow::Result<(Self, async_channel::Receiver<()>)> {
        let (events, receiver) = async_channel::bounded(8);
        let (changed, changes) = async_channel::bounded(1);
        let shared = Arc::new(Shared {
            snapshot: RwLock::new(Arc::new(Snapshot::default())),
            events,
            changed,
            mutation: AtomicBool::new(false),
            scan: AtomicBool::new(false),
            refresh: AtomicBool::new(false),
        });
        let weak = Arc::downgrade(&shared);
        std::thread::Builder::new()
            .name("dojo-wifi".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();
                match runtime {
                    Ok(runtime) => runtime.block_on(run(weak, receiver, address)),
                    Err(_) => {
                        if let Some(shared) = weak.upgrade() {
                            let mut state = Snapshot::default();
                            state.availability =
                                Availability::Unavailable("Wi-Fi worker could not start".into());
                            publish(&shared, &state);
                        }
                    }
                }
            })?;
        Ok((Self(shared), changes))
    }
    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.0.snapshot.read().unwrap().clone()
    }
    pub fn request(&self, action: Action) -> Result<(), String> {
        let gate = match &action {
            Action::Scan => Some(&self.0.scan),
            Action::CancelPassword => None,
            _ => Some(&self.0.mutation),
        };
        if let Some(gate) = gate {
            if gate.swap(true, Ordering::AcqRel) {
                return Err("A Wi-Fi operation is already running".into());
            }
        }
        if self.0.events.try_send(Event::Action(action)).is_err() {
            if let Some(gate) = gate {
                gate.store(false, Ordering::Release);
            }
            return Err("Wi-Fi worker is busy or unavailable".into());
        }
        Ok(())
    }
    pub fn refresh(&self) -> Result<(), String> {
        if self.0.refresh.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        if self.0.events.try_send(Event::Refresh).is_err() {
            self.0.refresh.store(false, Ordering::Release);
            return Err("Wi-Fi worker is busy or unavailable".into());
        }
        Ok(())
    }
}
/// The notice for a change of active connection, or none when nothing changed. A move from one
/// connection to another is a single "Connected to" the new one. Ethernet and other kinds are
/// named by their label like a network.
pub fn connection_notice(
    old: Option<&ActiveConnection>,
    new: Option<&ActiveConnection>,
) -> Option<dojo::notify::Notice> {
    use dojo::notify::{Notice, Urgency};
    if old == new {
        return None;
    }
    let summary = match (old, new) {
        (_, Some(now)) => format!("Connected to {}", now.label),
        (Some(was), None) => format!("Disconnected from {}", was.label),
        (None, None) => return None,
    };
    Some(Notice::new("wifi", summary).urgency(Urgency::Low))
}

/// Remembers the active connection between snapshots so the shell can send the notices for what
/// changed. What was already true when the watch first saw a settled snapshot is not reported.
#[derive(Default)]
pub struct ConnectionWatch {
    last: Option<Option<ActiveConnection>>,
}
impl ConnectionWatch {
    pub fn observe(&mut self, snapshot: &Snapshot) -> Option<dojo::notify::Notice> {
        if !snapshot.is_settled() {
            return None;
        }
        let now = snapshot.active_connection.clone();
        let notice = match &self.last {
            Some(before) => connection_notice(before.as_ref(), now.as_ref()),
            None => None,
        };
        self.last = Some(now);
        notice
    }
}
fn publish(shared: &Shared, state: &Snapshot) {
    let mut snapshot = shared.snapshot.write().unwrap();
    if snapshot.as_ref() != state {
        *snapshot = Arc::new(state.clone());
        let _ = shared.changed.try_send(());
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PersonalMode {
    Psk,
    Sae,
}
#[derive(Clone)]
struct AccessPoint {
    id: NetworkId,
    path: OwnedObjectPath,
    strength: u8,
    connected: bool,
    personal: Option<PersonalMode>,
    supports_psk: bool,
}
struct Saved {
    path: OwnedObjectPath,
    ssid: Vec<u8>,
    security: Security,
    mode: Option<PersonalMode>,
    interface: Option<String>,
}
struct WifiDevice {
    path: OwnedObjectPath,
    interface: String,
    active_connection: OwnedObjectPath,
}
struct Observed {
    availability: Availability,
    active_connection: Option<ActiveConnection>,
    devices: Vec<WifiDevice>,
    points: Vec<AccessPoint>,
    saved: Vec<Saved>,
}
impl Observed {
    fn profile(&self, point: &AccessPoint) -> Option<&Saved> {
        let interface = self
            .devices
            .iter()
            .find(|d| d.path.as_str() == point.id.device)?
            .interface
            .as_str();
        self.saved.iter().find(|s| {
            s.ssid == point.id.ssid
                && s.security == point.id.security
                && s.interface.as_deref().is_none_or(|i| i == interface)
                && s.mode.is_none_or(|mode| {
                    point.personal == Some(mode)
                        || (mode == PersonalMode::Psk && point.supports_psk)
                })
        })
    }
    fn point(&self, id: &NetworkId) -> Option<&AccessPoint> {
        self.points
            .iter()
            .filter(|p| &p.id == id)
            .max_by_key(|p| (p.connected, self.profile(p).is_some(), p.strength))
    }
    fn networks(&self) -> Vec<Network> {
        let mut grouped: BTreeMap<NetworkId, &AccessPoint> = BTreeMap::new();
        for point in &self.points {
            grouped
                .entry(point.id.clone())
                .and_modify(|previous| {
                    if (point.connected, point.strength) > (previous.connected, previous.strength) {
                        *previous = point;
                    }
                })
                .or_insert(point);
        }
        let mut networks: Vec<_> = grouped
            .into_values()
            .map(|point| Network {
                id: point.id.clone(),
                label: ssid_label(&point.id.ssid),
                signal: point.strength,
                security: point.id.security,
                connected: point.connected,
                saved: self
                    .points
                    .iter()
                    .filter(|p| p.id == point.id)
                    .any(|p| self.profile(p).is_some()),
            })
            .collect();
        networks.sort_by(|a, b| {
            b.connected
                .cmp(&a.connected)
                .then_with(|| b.signal.cmp(&a.signal))
                .then_with(|| a.label.cmp(&b.label))
        });
        networks
    }
}
fn ssid_label(ssid: &[u8]) -> String {
    String::from_utf8_lossy(ssid)
        .chars()
        .map(|c| if c.is_control() { '�' } else { c })
        .collect()
}
fn classify(flags: u32, wpa: u32, rsn: u32) -> (Security, Option<PersonalMode>) {
    let security = wpa | rsn;
    if security & 0x400 != 0 {
        (Security::Personal, Some(PersonalMode::Sae))
    } else if security & 0x100 != 0 {
        (Security::Personal, Some(PersonalMode::Psk))
    } else if security & (0x200 | 0x2000) != 0 {
        (Security::Enterprise, None)
    } else if flags & 1 == 0 && security == 0 {
        (Security::Open, None)
    } else {
        (Security::Unsupported, None)
    }
}
async fn proxy<'a>(
    bus: &'a Connection,
    path: &str,
    interface: &'static str,
) -> anyhow::Result<Proxy<'a>> {
    Ok(zbus::proxy::Builder::new(bus)
        .destination(NM)?
        .path(OwnedObjectPath::try_from(path.to_owned())?)?
        .interface(interface)?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await?)
}
fn text(settings: &Settings, section: &str, key: &str) -> Option<String> {
    <&str>::try_from(settings.get(section)?.get(key)?)
        .ok()
        .map(str::to_owned)
}
fn bytes(settings: &Settings, section: &str, key: &str) -> Option<Vec<u8>> {
    Vec::<u8>::try_from(settings.get(section)?.get(key)?.try_clone().ok()?).ok()
}
fn saved_profile(path: OwnedObjectPath, settings: &Settings) -> Option<Saved> {
    if text(settings, "connection", "type")?.as_str() != "802-11-wireless" {
        return None;
    }
    let ssid = bytes(settings, "802-11-wireless", "ssid")?;
    let (security, mode) = match text(settings, SECURITY, "key-mgmt").as_deref() {
        Some("wpa-psk") => (Security::Personal, Some(PersonalMode::Psk)),
        Some("sae") => (Security::Personal, Some(PersonalMode::Sae)),
        Some("wpa-eap" | "wpa-eap-suite-b-192" | "ieee8021x") => (Security::Enterprise, None),
        None => (Security::Open, None),
        _ => (Security::Unsupported, None),
    };
    Some(Saved {
        path,
        ssid,
        security,
        mode,
        interface: text(settings, "connection", "interface-name"),
    })
}
async fn active_connection(
    bus: &Connection,
    manager: &Proxy<'_>,
) -> anyhow::Result<Option<ActiveConnection>> {
    let path: OwnedObjectPath = manager.get_property("PrimaryConnection").await?;
    if path.as_str() == "/" {
        return Ok(None);
    }
    let active = proxy(
        bus,
        path.as_str(),
        "org.freedesktop.NetworkManager.Connection.Active",
    )
    .await?;
    let paths: Vec<OwnedObjectPath> = active.get_property("Devices").await?;
    for path in paths.into_iter().take(32) {
        let device = proxy(bus, path.as_str(), DEVICE).await?;
        let device_type: u32 = device.get_property("DeviceType").await?;
        let kind = match device_type {
            1 => {
                let udi: String = device.get_property("Udi").await?;
                ConnectionKind::Ethernet {
                    usb: udi.split('/').any(|part| part.starts_with("usb")),
                }
            }
            2 => ConnectionKind::Wifi,
            _ => continue,
        };
        return Ok(Some(ActiveConnection {
            kind,
            label: active.get_property("Id").await?,
            interface: device.get_property("Interface").await?,
        }));
    }
    Ok(None)
}

async fn observe(bus: &Connection) -> anyhow::Result<Observed> {
    let manager = proxy(bus, ROOT, NM).await?;
    let active_connection = active_connection(bus, &manager).await?;
    let enabled: bool = manager.get_property("WirelessEnabled").await?;
    let hardware: bool = manager.get_property("WirelessHardwareEnabled").await?;
    let paths: Vec<OwnedObjectPath> = manager.call("GetDevices", &()).await?;
    let mut devices = vec![];
    let mut points = vec![];
    for path in paths.into_iter().take(32) {
        let device = proxy(bus, path.as_str(), DEVICE).await?;
        let kind: u32 = device.get_property("DeviceType").await?;
        if kind != 2 {
            continue;
        }
        let managed: bool = device.get_property("Managed").await?;
        if !managed {
            continue;
        }
        let interface: String = device.get_property("Interface").await?;
        let state: u32 = device.get_property("State").await?;
        let active_connection: OwnedObjectPath = device.get_property("ActiveConnection").await?;
        let wireless = proxy(bus, path.as_str(), WIRELESS).await?;
        let active: OwnedObjectPath = wireless.get_property("ActiveAccessPoint").await?;
        let mut access_points: Vec<OwnedObjectPath> =
            wireless.call("GetAllAccessPoints", &()).await?;
        if active.as_str() != "/" && !access_points.contains(&active) {
            access_points.push(active.clone());
        }
        for ap_path in access_points.into_iter().take(512) {
            let ap = proxy(
                bus,
                ap_path.as_str(),
                "org.freedesktop.NetworkManager.AccessPoint",
            )
            .await?;
            let ssid: Vec<u8> = match ap.get_property("Ssid").await {
                Ok(ssid) => ssid,
                Err(_) => continue,
            };
            if ssid.is_empty() {
                continue;
            }
            let flags: u32 = ap.get_property("Flags").await?;
            let wpa: u32 = ap.get_property("WpaFlags").await?;
            let rsn: u32 = ap.get_property("RsnFlags").await?;
            let strength: u8 = ap.get_property("Strength").await?;
            let (security, personal) = classify(flags, wpa, rsn);
            points.push(AccessPoint {
                id: NetworkId {
                    device: path.to_string(),
                    ssid,
                    security,
                },
                connected: state == 100 && active == ap_path,
                path: ap_path,
                strength: strength.min(100),
                personal,
                supports_psk: (wpa | rsn) & 0x100 != 0,
            });
        }
        devices.push(WifiDevice {
            path,
            interface,
            active_connection,
        });
    }
    let settings = proxy(
        bus,
        &format!("{ROOT}/Settings"),
        "org.freedesktop.NetworkManager.Settings",
    )
    .await?;
    let paths: Vec<OwnedObjectPath> = settings.call("ListConnections", &()).await?;
    let mut saved = vec![];
    for path in paths.into_iter().take(256) {
        let connection = proxy(bus, path.as_str(), PROFILE).await?;
        let settings: Settings = match connection.call("GetSettings", &()).await {
            Ok(settings) => settings,
            Err(_) => continue,
        };
        if let Some(profile) = saved_profile(path, &settings) {
            saved.push(profile);
        }
    }
    let availability = if devices.is_empty() {
        Availability::Unavailable("No managed Wi-Fi adapter".into())
    } else if !hardware {
        Availability::Unavailable("Wi-Fi is blocked by a hardware switch".into())
    } else if enabled {
        Availability::Enabled
    } else {
        Availability::Disabled
    };
    Ok(Observed {
        availability,
        active_connection,
        devices,
        points,
        saved,
    })
}
fn update_facts(state: &mut Snapshot, observed: &Observed) {
    state.availability = observed.availability.clone();
    state.active_connection = observed.active_connection.clone();
    state.networks = observed.networks();
}
async fn refresh(bus: &Connection, state: &mut Snapshot) -> Option<Observed> {
    match tokio::time::timeout(READ_LIMIT, observe(bus)).await {
        Ok(Ok(observed)) => {
            state.observation_error = None;
            update_facts(state, &observed);
            Some(observed)
        }
        _ => {
            if matches!(state.availability, Availability::Unavailable(_)) {
                state.availability =
                    Availability::Unavailable("NetworkManager could not be reached".into());
            }
            state.observation_error = Some("Could not refresh network state. Try again.".into());
            None
        }
    }
}
async fn run(
    shared: Weak<Shared>,
    events: async_channel::Receiver<Event>,
    address: Option<String>,
) {
    let mut state = Snapshot::default();
    let mut bus: Option<Connection> = None;
    let mut signals: Option<MessageStream> = None;
    let mut observed: Option<Observed> = None;
    let mut refresh_due = true;
    loop {
        if shared.upgrade().is_none() {
            break;
        }
        if refresh_due {
            if bus.is_none() {
                let connect = async {
                    match &address {
                        Some(address) => {
                            zbus::connection::Builder::address(address.as_str())?
                                .build()
                                .await
                        }
                        None => Connection::system().await,
                    }
                };
                bus = tokio::time::timeout(READ_LIMIT, connect)
                    .await
                    .ok()
                    .and_then(Result::ok);
                if let Some(connection) = &bus {
                    let rule = zbus::MatchRule::builder()
                        .msg_type(zbus::message::Type::Signal)
                        .sender(NM)
                        .expect("NetworkManager bus name")
                        .build();
                    signals = tokio::time::timeout(
                        READ_LIMIT,
                        MessageStream::for_match_rule(rule, connection, Some(64)),
                    )
                    .await
                    .ok()
                    .and_then(Result::ok);
                }
            }
            if let Some(connection) = &bus {
                observed = refresh(connection, &mut state).await;
                if observed.is_none() {
                    signals = None;
                    bus = None;
                }
            } else {
                if matches!(state.availability, Availability::Unavailable(_)) {
                    state.availability =
                        Availability::Unavailable("System D-Bus is unavailable".into());
                }
                state.observation_error = Some("System D-Bus is unavailable. Try again.".into());
            }
            if let Some(shared) = shared.upgrade() {
                shared.refresh.store(false, Ordering::Release);
                publish(&shared, &state);
            }
            refresh_due = false;
        }
        let received = tokio::select! {
            event = tokio::time::timeout(POLL, events.recv()) => Some(event),
            signal = async {
                match &mut signals {
                    Some(stream) => stream.next().await,
                    None => std::future::pending().await,
                }
            } => {
                if !matches!(signal, Some(Ok(_))) {
                    signals = None;
                    bus = None;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
                None
            }
        };
        let Some(received) = received else {
            refresh_due = true;
            continue;
        };
        let event = match received {
            Ok(Ok(event)) => event,
            Ok(Err(_)) => break,
            Err(_) => {
                refresh_due = true;
                continue;
            }
        };
        match event {
            Event::Refresh => {
                refresh_due = true;
            }
            Event::Action(Action::CancelPassword) => {
                state.password_required = None;
            }
            Event::Action(Action::Scan) => {
                state.scanning = true;
                if let Some(shared) = shared.upgrade() {
                    publish(&shared, &state);
                }
                let result = match (&bus, &observed) {
                    (Some(bus), Some(observed)) => {
                        tokio::time::timeout(READ_LIMIT, scan(bus, observed))
                            .await
                            .unwrap_or_else(|_| Err(anyhow::anyhow!("timeout")))
                    }
                    _ => Err(anyhow::anyhow!("unavailable")),
                };
                state.scanning = false;
                if result.is_err() {
                    state.error = Some("Could not scan for Wi-Fi networks. Try again.".into());
                }
                if let Some(shared) = shared.upgrade() {
                    shared.scan.store(false, Ordering::Release);
                }
                refresh_due = true;
            }
            Event::Action(action) => {
                let operation = match &action {
                    Action::SetEnabled(_) => Operation::SetEnabled,
                    Action::Connect(_, _) => Operation::Connect,
                    _ => Operation::Disconnect,
                };
                state.pending = Some(operation);
                state.password_required = None;
                if let Some(shared) = shared.upgrade() {
                    publish(&shared, &state);
                }
                let result = if let Some(bus) = &bus {
                    tokio::time::timeout(ACTION_LIMIT, mutate(bus, action))
                        .await
                        .unwrap_or(Err(MutationError::Message(
                            "Wi-Fi operation timed out. Checking the host state…".into(),
                        )))
                } else {
                    Err(MutationError::Message(
                        "NetworkManager is unavailable".into(),
                    ))
                };
                state.pending = None;
                match result {
                    Ok(()) => state.error = None,
                    Err(MutationError::Password(id, error)) => {
                        state.password_required = Some(id);
                        if let Some(error) = error {
                            state.error = Some(error);
                        }
                    }
                    Err(MutationError::Message(error)) => state.error = Some(error),
                }
                if let Some(shared) = shared.upgrade() {
                    shared.mutation.store(false, Ordering::Release);
                    publish(&shared, &state);
                }
                refresh_due = true;
            }
        }
        if let Some(shared) = shared.upgrade() {
            publish(&shared, &state);
        }
    }
}
async fn scan(bus: &Connection, observed: &Observed) -> anyhow::Result<()> {
    if !matches!(observed.availability, Availability::Enabled) {
        bail!("Wi-Fi disabled");
    }
    for device in &observed.devices {
        let wireless = proxy(bus, device.path.as_str(), WIRELESS).await?;
        let _: () = wireless
            .call("RequestScan", &(HashMap::<String, OwnedValue>::new(),))
            .await?;
    }
    Ok(())
}

enum MutationError {
    Password(NetworkId, Option<String>),
    Message(String),
}
impl From<anyhow::Error> for MutationError {
    fn from(_: anyhow::Error) -> Self {
        Self::Message("NetworkManager could not complete the Wi-Fi operation. Check permissions and try again.".into())
    }
}
fn string(value: &str) -> OwnedValue {
    OwnedValue::from(zbus::zvariant::Str::from(value.to_owned()))
}
fn array(value: Vec<u8>) -> anyhow::Result<OwnedValue> {
    Ok(OwnedValue::try_from(Value::from(value))?)
}
fn validate_password(mode: PersonalMode, password: &str) -> Result<(), MutationError> {
    let valid = match mode {
        PersonalMode::Psk => {
            (password.is_ascii() && (8..=63).contains(&password.len()))
                || (password.len() == 64 && password.bytes().all(|b| b.is_ascii_hexdigit()))
        }
        PersonalMode::Sae => !password.is_empty() && !password.contains('\0'),
    };
    if valid {
        Ok(())
    } else {
        Err(MutationError::Message(match mode {
            PersonalMode::Psk => {
                "Enter 8–63 ASCII characters or a 64-digit hexadecimal WPA2 key".into()
            }
            PersonalMode::Sae => "Enter a nonempty WPA3 password".into(),
        }))
    }
}
fn credentials(
    settings: &mut Settings,
    mode: PersonalMode,
    password: &str,
) -> Result<(), MutationError> {
    validate_password(mode, password)?;
    let security = settings.entry(SECURITY.into()).or_default();
    security.insert(
        "key-mgmt".into(),
        string(match mode {
            PersonalMode::Psk => "wpa-psk",
            PersonalMode::Sae => "sae",
        }),
    );
    security.insert("psk".into(), string(password));
    security.insert("psk-flags".into(), OwnedValue::from(0u32));
    if mode == PersonalMode::Sae {
        security.insert("pmf".into(), OwnedValue::from(3i32));
    }
    Ok(())
}
fn new_settings(point: &AccessPoint, password: Option<&str>) -> Result<Settings, MutationError> {
    let mut settings = Settings::new();
    settings.insert(
        "connection".into(),
        HashMap::from([
            ("id".into(), string(&ssid_label(&point.id.ssid))),
            ("type".into(), string("802-11-wireless")),
            ("uuid".into(), string(&uuid::Uuid::new_v4().to_string())),
        ]),
    );
    settings.insert(
        "802-11-wireless".into(),
        HashMap::from([
            ("ssid".into(), array(point.id.ssid.clone())?),
            ("mode".into(), string("infrastructure")),
        ]),
    );
    settings.insert(
        "ipv4".into(),
        HashMap::from([("method".into(), string("auto"))]),
    );
    settings.insert(
        "ipv6".into(),
        HashMap::from([("method".into(), string("auto"))]),
    );
    if let Some(mode) = point.personal {
        let password = password.ok_or_else(|| MutationError::Password(point.id.clone(), None))?;
        credentials(&mut settings, mode, password)?;
    }
    Ok(settings)
}
async fn mutate(bus: &Connection, action: Action) -> Result<(), MutationError> {
    let observed = observe(bus).await?;
    let manager = proxy(bus, ROOT, NM).await?;
    match action {
        Action::SetEnabled(enabled) => {
            if observed.devices.is_empty() {
                return Err(MutationError::Message("No managed Wi-Fi adapter".into()));
            }
            if enabled && matches!(observed.availability, Availability::Unavailable(_)) {
                return Err(MutationError::Message(
                    "Wi-Fi is blocked by a hardware switch".into(),
                ));
            }
            manager
                .set_property("WirelessEnabled", enabled)
                .await
                .map_err(anyhow::Error::from)?;
        }
        Action::Disconnect => {
            for device in &observed.devices {
                if device.active_connection.as_str() != "/" {
                    let device_proxy = proxy(bus, device.path.as_str(), DEVICE).await?;
                    let _: () = device_proxy
                        .call("Disconnect", &())
                        .await
                        .map_err(anyhow::Error::from)?;
                }
            }
        }
        Action::Connect(id, password) => {
            if !matches!(observed.availability, Availability::Enabled) {
                return Err(MutationError::Message(
                    "Enable Wi-Fi before connecting".into(),
                ));
            }
            let point = observed.point(&id).ok_or_else(|| {
                MutationError::Message("This network is no longer available. Scan again.".into())
            })?;
            if point.connected {
                return Ok(());
            }
            match id.security {
                Security::Enterprise => {
                    return Err(MutationError::Message(
                        "Enterprise Wi-Fi requires settings outside this panel".into(),
                    ));
                }
                Security::Unsupported => {
                    return Err(MutationError::Message(
                        "This network's security is not supported".into(),
                    ));
                }
                _ => (),
            }
            let device =
                OwnedObjectPath::try_from(id.device.clone()).map_err(anyhow::Error::from)?;
            let active: OwnedObjectPath;
            if let Some(saved) = observed.profile(point) {
                let profile = proxy(bus, saved.path.as_str(), PROFILE).await?;
                if let Some(mode) = saved.mode.or(point.personal) {
                    if let Some(password) = password.as_deref() {
                        let version: u64 = profile.get_property("VersionId").await.unwrap_or(0);
                        let mut settings: Settings = profile
                            .call("GetSettings", &())
                            .await
                            .map_err(anyhow::Error::from)?;
                        credentials(&mut settings, mode, password).map_err(
                            |error| match error {
                                MutationError::Message(message) => {
                                    MutationError::Password(id.clone(), Some(message))
                                }
                                other => other,
                            },
                        )?;
                        let args =
                            HashMap::from([("version-id".to_string(), OwnedValue::from(version))]);
                        let _: HashMap<String, OwnedValue> = profile
                            .call("Update2", &(settings, 1u32, args))
                            .await
                            .map_err(anyhow::Error::from)?;
                    } else {
                        let secrets: Result<Settings, _> =
                            profile.call("GetSecrets", &(SECURITY,)).await;
                        if !secrets.ok().is_some_and(|s| {
                            text(&s, SECURITY, "psk").is_some_and(|p| !p.is_empty())
                        }) {
                            return Err(MutationError::Password(id, None));
                        }
                    }
                }
                active = manager
                    .call("ActivateConnection", &(&saved.path, &device, &point.path))
                    .await
                    .map_err(anyhow::Error::from)?;
            } else {
                let settings =
                    new_settings(point, password.as_deref()).map_err(|error| match error {
                        MutationError::Message(message) if password.is_some() => {
                            MutationError::Password(id.clone(), Some(message))
                        }
                        other => other,
                    })?;
                let (_, activated): (OwnedObjectPath, OwnedObjectPath) = manager
                    .call(
                        "AddAndActivateConnection",
                        &(settings, &device, &point.path),
                    )
                    .await
                    .map_err(anyhow::Error::from)?;
                active = activated;
            }
            wait_connected(bus, &active, &device, id).await?;
        }
        Action::Scan | Action::CancelPassword => (),
    }
    Ok(())
}
async fn wait_connected(
    bus: &Connection,
    active: &OwnedObjectPath,
    device: &OwnedObjectPath,
    id: NetworkId,
) -> Result<(), MutationError> {
    loop {
        let connection = proxy(
            bus,
            active.as_str(),
            "org.freedesktop.NetworkManager.Connection.Active",
        )
        .await?;
        let state: Result<u32, _> = connection.get_property("State").await;
        if matches!(state, Ok(2)) {
            return Ok(());
        }
        let device = proxy(bus, device.as_str(), DEVICE).await?;
        let (state_device, reason): (u32, u32) = device
            .get_property("StateReason")
            .await
            .map_err(anyhow::Error::from)?;
        if state.is_err() || matches!(state, Ok(4)) || state_device == 120 {
            if reason == 7 || (8..=11).contains(&reason) {
                return Err(MutationError::Password(
                    id,
                    Some("Could not authenticate. Check the Wi-Fi password and try again.".into()),
                ));
            }
            return Err(MutationError::Message(
                "Wi-Fi connection failed. Scan and try again.".into(),
            ));
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn connection(kind: ConnectionKind, label: &str) -> ActiveConnection {
        ActiveConnection {
            kind,
            label: label.into(),
            interface: "if0".into(),
        }
    }
    fn settled(active: Option<ActiveConnection>) -> Snapshot {
        Snapshot {
            availability: Availability::Enabled,
            active_connection: active,
            ..Snapshot::default()
        }
    }
    #[test]
    fn connection_notices_name_the_connection_and_stay_low() {
        use dojo::notify::Urgency;
        let home = connection(ConnectionKind::Wifi, "Home");
        let wired = connection(ConnectionKind::Ethernet { usb: false }, "Wired 1");
        let up = connection_notice(None, Some(&home)).unwrap();
        assert_eq!(
            (up.kind.as_str(), up.summary.as_str()),
            ("wifi", "Connected to Home")
        );
        assert_eq!(up.urgency, Urgency::Low);
        let down = connection_notice(Some(&wired), None).unwrap();
        assert_eq!(down.summary, "Disconnected from Wired 1");
        assert_eq!(down.urgency, Urgency::Low);
        assert_eq!(
            connection_notice(None, Some(&wired)).unwrap().summary,
            "Connected to Wired 1"
        );
        assert!(connection_notice(Some(&home), Some(&home)).is_none());
        assert!(connection_notice(None, None).is_none());
    }
    #[test]
    fn moving_between_networks_is_one_connected_notice() {
        let home = connection(ConnectionKind::Wifi, "Home");
        let cafe = connection(ConnectionKind::Wifi, "Cafe");
        let notice = connection_notice(Some(&home), Some(&cafe)).unwrap();
        assert_eq!(notice.summary, "Connected to Cafe");
    }
    #[test]
    fn watch_is_silent_until_it_has_a_baseline() {
        let home = connection(ConnectionKind::Wifi, "Home");
        let mut watch = ConnectionWatch::default();
        // Before NetworkManager is read, and after a failed read, nothing is known.
        assert!(watch.observe(&Snapshot::default()).is_none());
        let failed = Snapshot {
            observation_error: Some("down".into()),
            active_connection: Some(home.clone()),
            ..settled(None)
        };
        assert!(watch.observe(&failed).is_none());
        // The first settled snapshot is the baseline, even with a connection already up.
        assert!(watch.observe(&settled(Some(home.clone()))).is_none());
        assert!(watch.observe(&settled(Some(home.clone()))).is_none());
        let down = watch.observe(&settled(None)).unwrap();
        assert_eq!(down.summary, "Disconnected from Home");
        assert!(watch.observe(&settled(None)).is_none());
        let up = watch.observe(&settled(Some(home))).unwrap();
        assert_eq!(up.summary, "Connected to Home");
    }
    #[test]
    fn watch_reports_a_move_between_networks_once() {
        let mut watch = ConnectionWatch::default();
        assert!(watch.observe(&settled(None)).is_none());
        let home = connection(ConnectionKind::Wifi, "Home");
        let cafe = connection(ConnectionKind::Wifi, "Cafe");
        assert!(watch.observe(&settled(Some(home))).is_some());
        let moved = watch.observe(&settled(Some(cafe))).unwrap();
        assert_eq!(moved.summary, "Connected to Cafe");
    }
    fn point(ssid: &[u8], strength: u8, connected: bool, security: Security) -> AccessPoint {
        AccessPoint {
            id: NetworkId {
                device: "/device/1".into(),
                ssid: ssid.to_vec(),
                security,
            },
            path: OwnedObjectPath::try_from(format!("/ap/{strength}")).unwrap(),
            strength,
            connected,
            personal: (security == Security::Personal).then_some(PersonalMode::Psk),
            supports_psk: security == Security::Personal,
        }
    }
    fn observed(points: Vec<AccessPoint>) -> Observed {
        Observed {
            availability: Availability::Enabled,
            active_connection: None,
            devices: vec![WifiDevice {
                path: OwnedObjectPath::try_from("/device/1").unwrap(),
                interface: "wlan0".into(),
                active_connection: OwnedObjectPath::try_from("/").unwrap(),
            }],
            points,
            saved: vec![],
        }
    }
    #[test]
    fn duplicate_access_points_keep_active_connection_and_real_ssid_identity() {
        let state = observed(vec![
            point(b"a:b\\c", 10, true, Security::Personal),
            point(b"a:b\\c", 90, false, Security::Personal),
            point(b"a:b\\c", 80, false, Security::Open),
        ]);
        let networks = state.networks();
        assert_eq!(networks.len(), 2);
        assert_eq!(networks[0].label, "a:b\\c");
        assert_eq!(networks[0].signal, 10);
        assert!(networks[0].connected);
        assert_eq!(
            state.point(&networks[0].id).unwrap().path.as_str(),
            "/ap/10"
        );
        assert_ne!(networks[0].id, networks[1].id);
    }
    #[test]
    fn saved_wpa2_profile_matches_transition_access_point_without_downgrading_sae_only() {
        let mut transition = point(b"Saved", 90, false, Security::Personal);
        transition.personal = Some(PersonalMode::Sae);
        let mut host = observed(vec![transition]);
        host.saved.push(Saved {
            path: OwnedObjectPath::try_from("/profile/1").unwrap(),
            ssid: b"Saved".to_vec(),
            security: Security::Personal,
            mode: Some(PersonalMode::Psk),
            interface: Some("wlan0".into()),
        });
        assert!(host.networks()[0].saved);
        host.points[0].supports_psk = false;
        assert!(!host.networks()[0].saved);
        host.points[0].supports_psk = true;
        host.saved[0].interface = Some("other-device".into());
        assert!(!host.networks()[0].saved);
    }

    #[test]
    fn disconnected_duplicates_choose_strongest_access_point() {
        let state = observed(vec![
            point(b"Office", 20, false, Security::Personal),
            point(b"Office", 90, false, Security::Personal),
        ]);
        assert_eq!(state.networks()[0].signal, 90);
        assert_eq!(
            state.point(&state.networks()[0].id).unwrap().path.as_str(),
            "/ap/90"
        );
    }
    #[test]
    fn classification_distinguishes_personal_enterprise_and_legacy_security() {
        assert_eq!(
            classify(1, 0, 0x100),
            (Security::Personal, Some(PersonalMode::Psk))
        );
        assert_eq!(
            classify(1, 0, 0x400),
            (Security::Personal, Some(PersonalMode::Sae))
        );
        assert_eq!(
            classify(1, 0x100, 0x500),
            (Security::Personal, Some(PersonalMode::Sae))
        );
        assert_eq!(classify(1, 0, 0x2000), (Security::Enterprise, None));
        assert_eq!(classify(1, 0, 0), (Security::Unsupported, None));
        assert_eq!(classify(0, 0, 0), (Security::Open, None));
    }
    #[test]
    fn refresh_preserves_action_errors_password_prompt_and_pending_state() {
        let host = observed(vec![point(b"Office", 80, false, Security::Personal)]);
        let mut snapshot = Snapshot::default();
        snapshot.error = Some("Authentication failed".into());
        snapshot.password_required = Some(host.points[0].id.clone());
        update_facts(&mut snapshot, &host);
        assert_eq!(snapshot.error.as_deref(), Some("Authentication failed"));
        assert_eq!(snapshot.password_required, Some(host.points[0].id.clone()));
        assert_eq!(snapshot.pending, None);
        assert!(!snapshot.scanning);
    }
    #[test]
    fn new_profile_preserves_raw_ssid_and_wpa3_key_management() {
        let mut ap = point(&[0xff, b':', b'\\'], 80, false, Security::Personal);
        ap.personal = Some(PersonalMode::Sae);
        let settings =
            new_settings(&ap, Some("x")).unwrap_or_else(|_| panic!("valid SAE credential"));
        assert_eq!(
            bytes(&settings, "802-11-wireless", "ssid"),
            Some(vec![0xff, b':', b'\\'])
        );
        assert_eq!(
            text(&settings, SECURITY, "key-mgmt").as_deref(),
            Some("sae")
        );
        assert_eq!(
            i32::try_from(settings[SECURITY]["pmf"].try_clone().unwrap()).unwrap(),
            3
        );
        assert!(validate_password(PersonalMode::Psk, "short").is_err());
        assert!(validate_password(PersonalMode::Psk, "password").is_ok());
        assert!(validate_password(PersonalMode::Psk, &"a".repeat(64)).is_ok());
        assert!(validate_password(PersonalMode::Psk, &"z".repeat(64)).is_err());
        assert!(validate_password(PersonalMode::Sae, "x").is_ok());
        assert!(validate_password(PersonalMode::Sae, "").is_err());
    }
    #[test]
    fn personal_network_needs_password_but_open_network_does_not() {
        let ap = point(b"New", 50, false, Security::Personal);
        assert!(matches!(
            new_settings(&ap, None),
            Err(MutationError::Password(_, None))
        ));
        let open = point(b"Guest", 50, false, Security::Open);
        assert!(
            !new_settings(&open, None)
                .unwrap_or_else(|_| panic!("open network"))
                .contains_key(SECURITY)
        );
    }

    struct PrivateBus {
        child: std::process::Child,
        address: String,
        _config: tempfile::NamedTempFile,
    }
    impl PrivateBus {
        fn start() -> Self {
            use std::io::{BufRead, BufReader, Write};
            use std::process::{Command, Stdio};
            let mut config = tempfile::NamedTempFile::new().unwrap();
            config.write_all(b"<busconfig><type>session</type><listen>unix:tmpdir=/tmp</listen><auth>EXTERNAL</auth><policy context='default'><allow send_destination='*'/><allow receive_sender='*'/><allow own='*'/></policy></busconfig>").unwrap();
            let mut child = Command::new("dbus-daemon")
                .arg(format!("--config-file={}", config.path().display()))
                .args(["--nofork", "--print-address=1"])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let mut address = String::new();
            BufReader::new(child.stdout.take().unwrap())
                .read_line(&mut address)
                .unwrap();
            Self {
                child,
                address: address.trim().into(),
                _config: config,
            }
        }
    }
    impl Drop for PrivateBus {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
    fn path(value: &str) -> OwnedObjectPath {
        OwnedObjectPath::try_from(value.to_owned()).unwrap()
    }
    #[derive(Default)]
    struct FakeState {
        enabled: AtomicBool,
        connected: AtomicBool,
        tethered: AtomicBool,
        no_wifi: AtomicBool,
        saved_name: AtomicBool,
        scans: std::sync::atomic::AtomicUsize,
        settings: std::sync::Mutex<Option<Settings>>,
    }
    struct FakeManager(Arc<FakeState>);
    #[zbus::interface(name = "org.freedesktop.NetworkManager")]
    impl FakeManager {
        fn get_devices(&self) -> Vec<OwnedObjectPath> {
            let mut devices = vec![];
            if !self.0.no_wifi.load(Ordering::Relaxed) {
                devices.push(path("/device/1"));
            }
            if self.0.tethered.load(Ordering::Relaxed) {
                devices.push(path("/device/usb"));
            }
            devices
        }
        #[zbus(property)]
        fn primary_connection(&self) -> OwnedObjectPath {
            path(if self.0.tethered.load(Ordering::Relaxed) {
                "/active/usb"
            } else if self.0.connected.load(Ordering::Relaxed) {
                "/active/1"
            } else {
                "/"
            })
        }
        #[zbus(property)]
        fn wireless_enabled(&self) -> bool {
            self.0.enabled.load(Ordering::Relaxed)
        }
        #[zbus(property)]
        fn set_wireless_enabled(&self, value: bool) {
            self.0.enabled.store(value, Ordering::Relaxed);
        }
        #[zbus(property)]
        fn wireless_hardware_enabled(&self) -> bool {
            true
        }
        fn activate_connection(
            &self,
            _profile: OwnedObjectPath,
            _device: OwnedObjectPath,
            _ap: OwnedObjectPath,
        ) -> OwnedObjectPath {
            self.0.connected.store(true, Ordering::Relaxed);
            path("/active/1")
        }
        fn add_and_activate_connection(
            &self,
            settings: Settings,
            _device: OwnedObjectPath,
            _ap: OwnedObjectPath,
        ) -> (OwnedObjectPath, OwnedObjectPath) {
            *self.0.settings.lock().unwrap() = Some(settings);
            self.0.connected.store(true, Ordering::Relaxed);
            (path("/profile/1"), path("/active/1"))
        }
    }
    struct FakeDevice(Arc<FakeState>);
    #[zbus::interface(name = "org.freedesktop.NetworkManager.Device")]
    impl FakeDevice {
        #[zbus(property)]
        fn device_type(&self) -> u32 {
            2
        }
        #[zbus(property)]
        fn managed(&self) -> bool {
            true
        }
        #[zbus(property)]
        fn interface(&self) -> &str {
            "wlan0"
        }
        #[zbus(property)]
        fn state(&self) -> u32 {
            if self.0.connected.load(Ordering::Relaxed) {
                100
            } else {
                30
            }
        }
        #[zbus(property)]
        fn state_reason(&self) -> (u32, u32) {
            (self.state(), 0)
        }
        #[zbus(property)]
        fn active_connection(&self) -> OwnedObjectPath {
            path(if self.0.connected.load(Ordering::Relaxed) {
                "/active/1"
            } else {
                "/"
            })
        }
        fn disconnect(&self) {
            self.0.connected.store(false, Ordering::Relaxed);
        }
    }
    struct FakeWireless(Arc<FakeState>);
    #[zbus::interface(name = "org.freedesktop.NetworkManager.Device.Wireless")]
    impl FakeWireless {
        #[zbus(property)]
        fn active_access_point(&self) -> OwnedObjectPath {
            path(if self.0.connected.load(Ordering::Relaxed) {
                "/ap/1"
            } else {
                "/"
            })
        }
        fn get_all_access_points(&self) -> Vec<OwnedObjectPath> {
            vec![path("/ap/1")]
        }
        fn request_scan(&self, _options: HashMap<String, OwnedValue>) {
            self.0.scans.fetch_add(1, Ordering::Relaxed);
        }
    }
    struct FakeAp(Arc<FakeState>);
    #[zbus::interface(name = "org.freedesktop.NetworkManager.AccessPoint")]
    impl FakeAp {
        #[zbus(property)]
        fn ssid(&self) -> Vec<u8> {
            if self.0.saved_name.load(Ordering::Relaxed) {
                b"Saved".to_vec()
            } else {
                vec![0xff, b':', b'\\']
            }
        }
        #[zbus(property)]
        fn flags(&self) -> u32 {
            1
        }
        #[zbus(property)]
        fn wpa_flags(&self) -> u32 {
            0
        }
        #[zbus(property)]
        fn rsn_flags(&self) -> u32 {
            0x400
        }
        #[zbus(property)]
        fn strength(&self) -> u8 {
            78
        }
    }
    struct FakeSettings;
    #[zbus::interface(name = "org.freedesktop.NetworkManager.Settings")]
    impl FakeSettings {
        fn list_connections(&self) -> Vec<OwnedObjectPath> {
            vec![path("/profile/1")]
        }
    }
    struct FakeProfile(Arc<FakeState>);
    #[zbus::interface(name = "org.freedesktop.NetworkManager.Settings.Connection")]
    impl FakeProfile {
        fn get_settings(&self) -> Settings {
            let mut ap = point(b"Saved", 78, false, Security::Personal);
            ap.personal = Some(PersonalMode::Sae);
            let mut settings =
                new_settings(&ap, Some("placeholder")).unwrap_or_else(|_| panic!("fake config"));
            settings.get_mut(SECURITY).unwrap().remove("psk");
            settings
        }
        fn get_secrets(&self, _name: &str) -> Settings {
            Settings::new()
        }
        #[zbus(property)]
        fn version_id(&self) -> u64 {
            1
        }
        fn update2(
            &self,
            settings: Settings,
            flags: u32,
            args: HashMap<String, OwnedValue>,
        ) -> HashMap<String, OwnedValue> {
            assert_eq!(flags, 1);
            assert_eq!(
                u64::try_from(args["version-id"].try_clone().unwrap()).unwrap(),
                1
            );
            *self.0.settings.lock().unwrap() = Some(settings);
            HashMap::new()
        }
    }
    struct FakeActive {
        wired: bool,
    }
    #[zbus::interface(name = "org.freedesktop.NetworkManager.Connection.Active")]
    impl FakeActive {
        #[zbus(property)]
        fn state(&self) -> u32 {
            2
        }
        #[zbus(property)]
        fn id(&self) -> &str {
            if self.wired { "Phone tether" } else { "Saved" }
        }
        #[zbus(property)]
        fn devices(&self) -> Vec<OwnedObjectPath> {
            vec![path(if self.wired {
                "/device/usb"
            } else {
                "/device/1"
            })]
        }
    }
    struct FakeEthernet;
    #[zbus::interface(name = "org.freedesktop.NetworkManager.Device")]
    impl FakeEthernet {
        #[zbus(property)]
        fn device_type(&self) -> u32 {
            1
        }
        #[zbus(property)]
        fn interface(&self) -> &str {
            "enxphone"
        }
        #[zbus(property)]
        fn udi(&self) -> &str {
            "/sys/devices/pci0000:00/usb1/1-2/1-2:1.0/net/enxphone"
        }
    }

    async fn wait_snapshot(
        controller: &Controller,
        predicate: impl Fn(&Snapshot) -> bool,
    ) -> Arc<Snapshot> {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let snapshot = controller.snapshot();
                if predicate(&snapshot) {
                    break snapshot;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("Wi-Fi worker update")
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn isolated_networkmanager_connect_scan_saved_password_and_disconnect() {
        let private = PrivateBus::start();
        let state = Arc::new(FakeState::default());
        state.enabled.store(true, Ordering::Relaxed);
        let _service = zbus::connection::Builder::address(private.address.as_str())
            .unwrap()
            .name(NM)
            .unwrap()
            .serve_at(ROOT, FakeManager(state.clone()))
            .unwrap()
            .serve_at("/device/1", FakeDevice(state.clone()))
            .unwrap()
            .serve_at("/device/1", FakeWireless(state.clone()))
            .unwrap()
            .serve_at("/ap/1", FakeAp(state.clone()))
            .unwrap()
            .serve_at(format!("{ROOT}/Settings"), FakeSettings)
            .unwrap()
            .serve_at("/profile/1", FakeProfile(state.clone()))
            .unwrap()
            .serve_at("/active/1", FakeActive { wired: false })
            .unwrap()
            .build()
            .await
            .unwrap();
        let (controller, _changes) =
            Controller::start_with_address(Some(private.address.clone())).unwrap();
        let initial = wait_snapshot(&controller, |s| s.networks.len() == 1).await;
        assert_eq!(initial.availability, Availability::Enabled);
        assert!(!initial.networks[0].saved);
        controller.request(Action::Scan).unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            while state.scans.load(Ordering::Relaxed) == 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        controller
            .request(Action::Connect(
                initial.networks[0].id.clone(),
                Some("new-password".into()),
            ))
            .unwrap();
        wait_snapshot(&controller, |s| {
            s.networks.iter().any(|n| n.connected) && s.pending.is_none()
        })
        .await;
        {
            let guard = state.settings.lock().unwrap();
            let settings = guard.as_ref().unwrap();
            assert_eq!(text(settings, SECURITY, "key-mgmt").as_deref(), Some("sae"));
            assert_eq!(
                bytes(settings, "802-11-wireless", "ssid"),
                Some(vec![0xff, b':', b'\\'])
            );
        }
        controller.request(Action::Disconnect).unwrap();
        wait_snapshot(&controller, |s| {
            s.networks.iter().all(|n| !n.connected) && s.pending.is_none()
        })
        .await;
        state.saved_name.store(true, Ordering::Relaxed);
        controller.refresh().unwrap();
        let saved = wait_snapshot(&controller, |s| {
            s.networks
                .first()
                .is_some_and(|n| n.label == "Saved" && n.saved)
        })
        .await;
        controller
            .request(Action::Connect(saved.networks[0].id.clone(), None))
            .unwrap();
        let prompted = wait_snapshot(&controller, |s| s.password_required.is_some()).await;
        assert_eq!(prompted.pending, None);
        controller.refresh().unwrap();
        assert_eq!(controller.snapshot().pending, None);
        controller
            .request(Action::Connect(
                saved.networks[0].id.clone(),
                Some(String::new()),
            ))
            .unwrap();
        let invalid = wait_snapshot(&controller, |s| {
            s.password_required.is_some() && s.error.is_some()
        })
        .await;
        assert_eq!(invalid.pending, None);
        assert_eq!(
            invalid.error.as_deref(),
            Some("Enter a nonempty WPA3 password")
        );
        controller.refresh().unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(
            controller.snapshot().error.as_deref(),
            Some("Enter a nonempty WPA3 password")
        );
        controller
            .request(Action::Connect(
                saved.networks[0].id.clone(),
                Some("retry-password".into()),
            ))
            .unwrap();
        wait_snapshot(&controller, |s| {
            s.networks.iter().any(|n| n.connected) && s.pending.is_none()
        })
        .await;
        assert!(controller.snapshot().password_required.is_none());
        controller.request(Action::SetEnabled(false)).unwrap();
        wait_snapshot(&controller, |s| {
            matches!(s.availability, Availability::Disabled) && s.pending.is_none()
        })
        .await;
        assert!(!state.enabled.load(Ordering::Relaxed));
        _service.release_name(NM).await.unwrap();
        controller.refresh().unwrap();
        let unavailable = wait_snapshot(&controller, |s| s.observation_error.is_some()).await;
        assert_eq!(unavailable.availability, Availability::Disabled);
        assert_eq!(unavailable.pending, None);
        assert_eq!(unavailable.networks.len(), 1);
        _service.request_name(NM).await.unwrap();
        controller.refresh().unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            while controller.0.refresh.load(Ordering::Acquire) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        controller.request(Action::SetEnabled(true)).unwrap();
        wait_snapshot(&controller, |s| {
            matches!(s.availability, Availability::Enabled)
                && s.pending.is_none()
                && s.error.is_none()
                && s.observation_error.is_none()
        })
        .await;
    }

    #[tokio::test]
    async fn primary_connection_changes_from_wifi_to_usb_and_back_on_signals() {
        let private = PrivateBus::start();
        let state = Arc::new(FakeState::default());
        state.enabled.store(true, Ordering::Relaxed);
        state.connected.store(true, Ordering::Relaxed);
        let service = zbus::connection::Builder::address(private.address.as_str())
            .unwrap()
            .name(NM)
            .unwrap()
            .serve_at(ROOT, FakeManager(state.clone()))
            .unwrap()
            .serve_at("/device/1", FakeDevice(state.clone()))
            .unwrap()
            .serve_at("/device/1", FakeWireless(state.clone()))
            .unwrap()
            .serve_at("/device/usb", FakeEthernet)
            .unwrap()
            .serve_at("/ap/1", FakeAp(state.clone()))
            .unwrap()
            .serve_at(format!("{ROOT}/Settings"), FakeSettings)
            .unwrap()
            .serve_at("/profile/1", FakeProfile(state.clone()))
            .unwrap()
            .serve_at("/active/1", FakeActive { wired: false })
            .unwrap()
            .serve_at("/active/usb", FakeActive { wired: true })
            .unwrap()
            .build()
            .await
            .unwrap();
        let (controller, _changes) =
            Controller::start_with_address(Some(private.address.clone())).unwrap();
        let wifi = wait_snapshot(&controller, |s| {
            s.active_connection
                .as_ref()
                .is_some_and(|c| c.kind == ConnectionKind::Wifi)
        })
        .await;
        assert_eq!(wifi.connection_title(), "Wi-Fi");
        assert_eq!(wifi.icon(), "\u{f05a9}");
        state.tethered.store(true, Ordering::Relaxed);
        service
            .emit_signal(
                None::<&str>,
                ROOT,
                NM,
                "DeviceAdded",
                &(path("/device/usb"),),
            )
            .await
            .unwrap();
        let usb = wait_snapshot(&controller, |s| {
            s.active_connection
                .as_ref()
                .is_some_and(|c| c.kind == ConnectionKind::Ethernet { usb: true })
        })
        .await;
        assert_eq!(usb.connection_title(), "USB Ethernet");
        assert_eq!(
            usb.connection_label(),
            "Connected · Phone tether · enxphone"
        );
        assert_eq!(usb.icon(), "\u{f0200}");
        assert!(
            usb.networks[0].connected,
            "Wi-Fi may stay associated while USB carries the default route"
        );
        state.enabled.store(false, Ordering::Relaxed);
        state.no_wifi.store(true, Ordering::Relaxed);
        service
            .emit_signal(
                None::<&str>,
                ROOT,
                NM,
                "DeviceRemoved",
                &(path("/device/1"),),
            )
            .await
            .unwrap();
        let usb_only = wait_snapshot(&controller, |s| s.networks.is_empty()).await;
        assert_eq!(usb_only.connection_title(), "USB Ethernet");
        assert_eq!(usb_only.icon(), "\u{f0200}");
        state.no_wifi.store(false, Ordering::Relaxed);
        state.enabled.store(true, Ordering::Relaxed);
        state.tethered.store(false, Ordering::Relaxed);
        service
            .emit_signal(
                None::<&str>,
                ROOT,
                NM,
                "DeviceRemoved",
                &(path("/device/usb"),),
            )
            .await
            .unwrap();
        let restored = wait_snapshot(&controller, |s| {
            s.active_connection
                .as_ref()
                .is_some_and(|c| c.kind == ConnectionKind::Wifi)
        })
        .await;
        assert_eq!(restored.connection_title(), "Wi-Fi");
        assert_eq!(restored.icon(), "\u{f05a9}");
        state.connected.store(false, Ordering::Relaxed);
        service
            .emit_signal(None::<&str>, ROOT, NM, "StateChanged", &(20_u32,))
            .await
            .unwrap();
        let disconnected = wait_snapshot(&controller, |s| s.active_connection.is_none()).await;
        assert_eq!(
            disconnected.connection_label(),
            "No active internet connection"
        );
    }

    #[tokio::test]
    #[ignore = "read-only observation of the host NetworkManager"]
    async fn live_networkmanager_read_only() {
        let bus = Connection::system().await.unwrap();
        let state = tokio::time::timeout(READ_LIMIT, observe(&bus))
            .await
            .unwrap()
            .unwrap();
        assert!(!state.devices.is_empty(), "managed Wi-Fi device");
        assert!(
            state.networks().iter().any(|n| n.connected),
            "current network is visible"
        );
        let active = state.active_connection.expect("primary network connection");
        println!("Primary connection: {active:?}");
    }
}
