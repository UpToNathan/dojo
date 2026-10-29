use std::collections::{BTreeMap, BTreeSet};
use std::sync::{
    Arc, Mutex, RwLock,
    atomic::{AtomicU64, Ordering},
};
use std::time::Duration;

use anyhow::{Context as _, bail};
use futures_lite::StreamExt;
use tokio::sync::{oneshot, watch};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue};
use zbus::{Connection, MessageStream, Proxy, interface};

const AGENT_PATH: &str = "/org/dojo/BluetoothAgent";
const BLUEZ: &str = "org.bluez";
// A fixed window from the first signal, never extended by subsequent signals.
const REFRESH_WINDOW: Duration = Duration::from_millis(100);
static BLUEZ_EPOCH: AtomicU64 = AtomicU64::new(1);
static PROMPT_SERIAL: AtomicU64 = AtomicU64::new(1);
type Objects = std::collections::HashMap<
    OwnedObjectPath,
    std::collections::HashMap<String, std::collections::HashMap<String, OwnedValue>>,
>;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DeviceId {
    path: String,
    epoch: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct AdapterId {
    path: String,
    epoch: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Adapter {
    pub id: AdapterId,
    pub name: String,
    pub powered: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub id: DeviceId,
    pub name: String,
    pub address: String,
    pub icon: String,
    pub paired: bool,
    pub connected: bool,
    pub battery: Option<u8>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Availability {
    Starting,
    Ready,
    NoAdapter,
    ServiceMissing,
    BusUnavailable,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromptKind {
    Confirm(u32),
    Pin,
    Passkey,
    Authorize(String),
    Display { code: String, entered: u16 },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prompt {
    pub token: u64,
    pub device: DeviceId,
    pub kind: PromptKind,
}
impl Prompt {
    pub fn validate(&self, input: &str) -> anyhow::Result<String> {
        match self.kind {
            PromptKind::Pin => {
                if input.is_empty()
                    || input.len() > 16
                    || !input.chars().all(|c| c.is_ascii_alphanumeric())
                {
                    bail!("Enter a PIN of 1–16 letters or digits");
                }
            }
            PromptKind::Passkey => {
                if input.is_empty() || input.len() > 6 || !input.bytes().all(|c| c.is_ascii_digit())
                {
                    bail!("Enter a passkey of up to six digits");
                }
            }
            PromptKind::Confirm(_) | PromptKind::Authorize(_) => return Ok(String::new()),
            PromptKind::Display { .. } => bail!("This code must be entered on the device"),
        }
        Ok(input.into())
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub availability: Availability,
    pub adapters: Vec<Adapter>,
    pub devices: Vec<Device>,
    pub pending: BTreeMap<DeviceId, String>,
    pub power_pending: BTreeSet<AdapterId>,
    pub scanning: bool,
    pub prompt: Option<Prompt>,
    pub error: Option<String>,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            availability: Availability::Starting,
            adapters: Vec::new(),
            devices: Vec::new(),
            pending: BTreeMap::new(),
            power_pending: BTreeSet::new(),
            scanning: false,
            prompt: None,
            error: None,
        }
    }
}

#[derive(Clone)]
pub enum Action {
    Power(AdapterId, bool),
    Connect(DeviceId, bool),
    Pair(DeviceId),
    Cancel(DeviceId),
    /// Removes the device and its pairing from the adapter; it has to be paired again.
    Forget(DeviceId),
    Answer(u64, Option<String>),
}
enum Event {
    Action(Action),
    Prompt(Prompt, Option<oneshot::Sender<Option<String>>>),
    Clear(u64),
    CancelPrompt,
    PanelClosed,
}
struct Shared {
    snapshot: RwLock<Arc<Snapshot>>,
    notify: async_channel::Sender<()>,
    events: async_channel::Sender<Event>,
    scan: watch::Sender<bool>,
    stop: watch::Sender<bool>,
    #[cfg(test)]
    publications: AtomicU64,
}
impl Drop for Shared {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
    }
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
        let (events, receiver) = async_channel::bounded(64);
        let (notify, changes) = async_channel::bounded(1);
        let (scan, scan_rx) = watch::channel(false);
        let (stop, stop_rx) = watch::channel(false);
        let shared = Arc::new(Shared {
            snapshot: RwLock::new(Arc::new(Snapshot::default())),
            notify,
            events,
            scan,
            stop,
            #[cfg(test)]
            publications: AtomicU64::new(0),
        });
        let weak = Arc::downgrade(&shared);
        std::thread::Builder::new()
            .name("dojo-bluetooth".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("Bluetooth runtime");
                runtime.block_on(async move {
                    loop {
                        let connection = match &address {
                            Some(address) => {
                                zbus::connection::Builder::address(address.as_str())
                                    .unwrap()
                                    .build()
                                    .await
                            }
                            None => Connection::system().await,
                        };
                        match connection {
                            Ok(connection) => {
                                if let Err(error) = run(
                                    connection,
                                    receiver.clone(),
                                    scan_rx.clone(),
                                    stop_rx.clone(),
                                    weak.clone(),
                                )
                                .await
                                {
                                    if let Some(shared) = weak.upgrade() {
                                        let mut snapshot = Snapshot::default();
                                        snapshot.availability = Availability::BusUnavailable;
                                        snapshot.error =
                                            Some(format!("Bluetooth bus disconnected: {error:#}"));
                                        publish(&shared, &snapshot);
                                    } else {
                                        break;
                                    }
                                } else {
                                    break;
                                }
                            }
                            Err(_) => {
                                if let Some(shared) = weak.upgrade() {
                                    let mut snapshot = Snapshot::default();
                                    snapshot.availability = Availability::BusUnavailable;
                                    publish(&shared, &snapshot);
                                } else {
                                    break;
                                }
                            }
                        }
                        if *stop_rx.borrow() || !stop_rx.has_changed().is_ok() {
                            break;
                        }
                        tokio::time::sleep(Duration::from_secs(3)).await;
                    }
                });
            })?;
        Ok((Self(shared), changes))
    }
    #[cfg(test)]
    pub(crate) fn unavailable_for_test() -> (Self, async_channel::Receiver<()>) {
        let directory = tempfile::tempdir().unwrap();
        Self::start_with_address(Some(format!(
            "unix:path={}/missing",
            directory.path().display()
        )))
        .unwrap()
    }
    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.0.snapshot.read().unwrap().clone()
    }
    pub fn submit(&self, action: Action) -> anyhow::Result<()> {
        self.0
            .events
            .try_send(Event::Action(action))
            .context("Bluetooth is busy; try again")
    }
    pub fn scan(&self, enabled: bool) {
        self.0.scan.send_if_modified(|current| {
            if *current == enabled {
                false
            } else {
                *current = enabled;
                true
            }
        });
    }
    pub fn close_panel(&self) {
        self.scan(false);
        let _ = self.0.events.try_send(Event::PanelClosed);
    }
}
fn publish(shared: &Shared, snapshot: &Snapshot) {
    #[cfg(test)]
    shared.publications.fetch_add(1, Ordering::Relaxed);
    let mut current = shared.snapshot.write().unwrap();
    if current.as_ref() != snapshot {
        *current = Arc::new(snapshot.clone());
        let _ = shared.notify.try_send(());
    }
}
fn string(
    properties: &std::collections::HashMap<String, OwnedValue>,
    name: &str,
) -> Option<String> {
    properties
        .get(name)
        .and_then(|v| <&str>::try_from(v).ok())
        .map(str::to_owned)
}
fn boolean(properties: &std::collections::HashMap<String, OwnedValue>, name: &str) -> bool {
    properties
        .get(name)
        .and_then(|v| bool::try_from(v).ok())
        .unwrap_or(false)
}
fn parse_objects(objects: Objects, epoch: u64, snapshot: &mut Snapshot) {
    snapshot.adapters.clear();
    snapshot.devices.clear();
    for (path, interfaces) in objects {
        if let Some(properties) = interfaces.get("org.bluez.Adapter1") {
            snapshot.adapters.push(Adapter {
                id: AdapterId {
                    path: path.to_string(),
                    epoch,
                },
                name: string(properties, "Alias").unwrap_or_else(|| "Bluetooth adapter".into()),
                powered: boolean(properties, "Powered"),
            });
        }
        if let Some(properties) = interfaces.get("org.bluez.Device1") {
            let Some(address) = string(properties, "Address") else {
                continue;
            };
            snapshot.devices.push(Device {
                id: DeviceId {
                    path: path.to_string(),
                    epoch,
                },
                name: string(properties, "Alias")
                    .or_else(|| string(properties, "Name"))
                    .unwrap_or_else(|| address.clone()),
                address,
                icon: string(properties, "Icon").unwrap_or_default(),
                paired: boolean(properties, "Paired"),
                connected: boolean(properties, "Connected"),
                battery: interfaces
                    .get("org.bluez.Battery1")
                    .and_then(|p| p.get("Percentage"))
                    .and_then(|v| u8::try_from(v).ok())
                    .filter(|v| *v <= 100),
            });
        }
    }
    snapshot.adapters.sort_by(|a, b| a.id.cmp(&b.id));
    snapshot.devices.sort_by(|a, b| {
        (!a.connected, !a.paired, a.name.to_lowercase(), &a.id).cmp(&(
            !b.connected,
            !b.paired,
            b.name.to_lowercase(),
            &b.id,
        ))
    });
    snapshot.availability = if snapshot.adapters.is_empty() {
        Availability::NoAdapter
    } else {
        Availability::Ready
    };
}
async fn refresh(
    connection: &Connection,
    epoch: u64,
    snapshot: &mut Snapshot,
) -> anyhow::Result<()> {
    let objects: Objects = tokio::time::timeout(Duration::from_secs(3), async {
        let proxy =
            Proxy::new(connection, BLUEZ, "/", "org.freedesktop.DBus.ObjectManager").await?;
        proxy.call("GetManagedObjects", &()).await
    })
    .await
    .context("Bluetooth state request timed out")??;
    parse_objects(objects, epoch, snapshot);
    Ok(())
}
async fn owner(connection: &Connection) -> Option<String> {
    let proxy = Proxy::new(
        connection,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    )
    .await
    .ok()?;
    proxy
        .call::<_, _, String>("GetNameOwner", &(BLUEZ,))
        .await
        .ok()
}

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.bluez.Error")]
enum AgentError {
    Rejected(String),
    Canceled(String),
    #[zbus(error)]
    Bus(zbus::Error),
}
struct Agent {
    events: async_channel::Sender<Event>,
    owner: Arc<RwLock<Option<String>>>,
    allowed: Arc<Mutex<Option<DeviceId>>>,
    runtime: tokio::runtime::Handle,
}
impl Agent {
    fn check(
        &self,
        header: &zbus::message::Header<'_>,
        device: Option<&OwnedObjectPath>,
    ) -> Result<Option<DeviceId>, AgentError> {
        if header.sender().map(|s| s.as_str()) != self.owner.read().unwrap().as_deref() {
            return Err(AgentError::Rejected("Unknown Bluetooth service".into()));
        }
        let allowed = self.allowed.lock().unwrap().clone();
        if let Some(device) = device {
            if allowed.as_ref().map(|id| id.path.as_str()) != Some(device.as_str()) {
                return Err(AgentError::Rejected("Pairing was not requested".into()));
            }
        }
        Ok(allowed)
    }
    async fn ask(&self, device: DeviceId, kind: PromptKind) -> Result<String, AgentError> {
        let token = PROMPT_SERIAL.fetch_add(1, Ordering::Relaxed);
        let (reply, response) = oneshot::channel();
        self.events
            .send(Event::Prompt(
                Prompt {
                    token,
                    device,
                    kind,
                },
                Some(reply),
            ))
            .await
            .map_err(|_| AgentError::Canceled("Shell closed".into()))?;
        let answer = self
            .runtime
            .spawn(async move { tokio::time::timeout(Duration::from_secs(90), response).await })
            .await;
        let _ = self.events.send(Event::Clear(token)).await;
        match answer {
            Ok(Ok(Ok(Some(answer)))) => Ok(answer),
            Ok(Ok(Ok(None))) => Err(AgentError::Rejected("Pairing rejected".into())),
            _ => Err(AgentError::Canceled("Pairing canceled or timed out".into())),
        }
    }
    async fn display(
        &self,
        device: DeviceId,
        code: String,
        entered: u16,
    ) -> Result<(), AgentError> {
        let token = PROMPT_SERIAL.fetch_add(1, Ordering::Relaxed);
        self.events
            .send(Event::Prompt(
                Prompt {
                    token,
                    device,
                    kind: PromptKind::Display { code, entered },
                },
                None,
            ))
            .await
            .map_err(|_| AgentError::Canceled("Shell closed".into()))
    }
}
#[interface(name = "org.bluez.Agent1")]
impl Agent {
    async fn request_confirmation(
        &self,
        device: OwnedObjectPath,
        passkey: u32,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> Result<(), AgentError> {
        let id = self.check(&header, Some(&device))?.unwrap();
        self.ask(id, PromptKind::Confirm(passkey)).await.map(|_| ())
    }
    async fn request_pin_code(
        &self,
        device: OwnedObjectPath,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> Result<String, AgentError> {
        let id = self.check(&header, Some(&device))?.unwrap();
        self.ask(id, PromptKind::Pin).await
    }
    async fn request_passkey(
        &self,
        device: OwnedObjectPath,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> Result<u32, AgentError> {
        let id = self.check(&header, Some(&device))?.unwrap();
        self.ask(id, PromptKind::Passkey)
            .await?
            .parse()
            .map_err(|_| AgentError::Rejected("Invalid passkey".into()))
    }
    async fn request_authorization(
        &self,
        device: OwnedObjectPath,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> Result<(), AgentError> {
        let id = self.check(&header, Some(&device))?.unwrap();
        self.ask(
            id,
            PromptKind::Authorize("Allow pairing with this device?".into()),
        )
        .await
        .map(|_| ())
    }
    async fn authorize_service(
        &self,
        device: OwnedObjectPath,
        uuid: String,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> Result<(), AgentError> {
        let id = self.check(&header, Some(&device))?.unwrap();
        self.ask(
            id,
            PromptKind::Authorize(format!("Allow the device service {uuid}?")),
        )
        .await
        .map(|_| ())
    }
    async fn display_pin_code(
        &self,
        device: OwnedObjectPath,
        pincode: String,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> Result<(), AgentError> {
        let id = self.check(&header, Some(&device))?.unwrap();
        self.display(id, pincode, 0).await
    }
    async fn display_passkey(
        &self,
        device: OwnedObjectPath,
        passkey: u32,
        entered: u16,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> Result<(), AgentError> {
        let id = self.check(&header, Some(&device))?.unwrap();
        self.display(id, format!("{passkey:06}"), entered).await
    }
    async fn cancel(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> Result<(), AgentError> {
        self.check(&header, None)?;
        let _ = self.events.send(Event::CancelPrompt).await;
        Ok(())
    }
    async fn release(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> Result<(), AgentError> {
        self.check(&header, None)?;
        let _ = self.events.send(Event::CancelPrompt).await;
        Ok(())
    }
}

async fn execute(connection: Connection, action: Action) -> anyhow::Result<()> {
    match action {
        Action::Power(id, powered) => {
            Proxy::new(&connection, BLUEZ, id.path.as_str(), "org.bluez.Adapter1")
                .await?
                .set_property("Powered", powered)
                .await?;
        }
        Action::Connect(id, connected) => {
            let proxy =
                Proxy::new(&connection, BLUEZ, id.path.as_str(), "org.bluez.Device1").await?;
            proxy
                .call::<_, _, ()>(if connected { "Connect" } else { "Disconnect" }, &())
                .await?;
        }
        Action::Pair(id) => {
            let proxy =
                Proxy::new(&connection, BLUEZ, id.path.as_str(), "org.bluez.Device1").await?;
            proxy.call::<_, _, ()>("Pair", &()).await?;
            proxy.call::<_, _, ()>("Connect", &()).await?;
        }
        Action::Cancel(id) => {
            let proxy =
                Proxy::new(&connection, BLUEZ, id.path.as_str(), "org.bluez.Device1").await?;
            proxy.call::<_, _, ()>("CancelPairing", &()).await?;
        }
        Action::Forget(id) => {
            // BlueZ removes a device through its adapter, which is the parent object path.
            let adapter = id
                .path
                .rsplit_once('/')
                .map(|(adapter, _)| adapter)
                .context("Bluetooth device path has no adapter")?;
            Proxy::new(&connection, BLUEZ, adapter, "org.bluez.Adapter1")
                .await?
                .call::<_, _, ()>("RemoveDevice", &(ObjectPath::try_from(id.path.as_str())?,))
                .await?;
        }
        Action::Answer(..) => unreachable!(),
    }
    Ok(())
}
async fn stop_scan(connection: &Connection, owned: &mut BTreeSet<String>) {
    for path in std::mem::take(owned) {
        if let Ok(proxy) = Proxy::new(connection, BLUEZ, path.as_str(), "org.bluez.Adapter1").await
        {
            let _ = tokio::time::timeout(
                Duration::from_secs(2),
                proxy.call::<_, _, ()>("StopDiscovery", &()),
            )
            .await;
        }
    }
}
async fn run(
    connection: Connection,
    events: async_channel::Receiver<Event>,
    mut scan: watch::Receiver<bool>,
    mut stop: watch::Receiver<bool>,
    shared: std::sync::Weak<Shared>,
) -> anyhow::Result<()> {
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(BLUEZ)?
        .build();
    let mut signals = MessageStream::for_match_rule(rule, &connection, Some(64)).await?;
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.freedesktop.DBus")?
        .interface("org.freedesktop.DBus")?
        .member("NameOwnerChanged")?
        .arg(0, BLUEZ)?
        .build();
    let mut owners = MessageStream::for_match_rule(rule, &connection, Some(16)).await?;
    let current_owner = Arc::new(RwLock::new(owner(&connection).await));
    let allowed = Arc::new(Mutex::new(None));
    let Some(initial_shared) = shared.upgrade() else {
        return Ok(());
    };
    connection
        .object_server()
        .at(
            AGENT_PATH,
            Agent {
                events: initial_shared.events.clone(),
                owner: current_owner.clone(),
                allowed: allowed.clone(),
                runtime: tokio::runtime::Handle::current(),
            },
        )
        .await?;
    drop(initial_shared);
    let mut epoch = BLUEZ_EPOCH.fetch_add(1, Ordering::Relaxed);
    let mut snapshot = Snapshot::default();
    let mut registered = false;
    let mut owned_scans = BTreeSet::new();
    let mut deadline = None;
    let mut tasks = tokio::task::JoinSet::new();
    let mut pairing_task: Option<tokio::task::AbortHandle> = None;
    let mut waiter: Option<(u64, oneshot::Sender<Option<String>>)> = None;
    let mut refresh_now = true;
    let mut refresh_at = None;
    let mut publish_local = true;
    loop {
        // Publish local feedback before any potentially slow D-Bus reconciliation.
        snapshot.scanning = !owned_scans.is_empty();
        if let Some(shared) = shared.upgrade() {
            if publish_local {
                publish(&shared, &snapshot);
            }
        } else {
            break;
        }
        if refresh_now {
            refresh_now = false;
            refresh_at = None;
            if current_owner.read().unwrap().is_none() {
                snapshot = Snapshot::default();
                snapshot.availability = Availability::ServiceMissing;
            } else {
                if let Err(error) = refresh(&connection, epoch, &mut snapshot).await {
                    snapshot.error = Some(format!("Could not read Bluetooth devices: {error:#}"));
                }
                if !registered {
                    let manager =
                        Proxy::new(&connection, BLUEZ, "/org/bluez", "org.bluez.AgentManager1")
                            .await?;
                    let agent = OwnedObjectPath::try_from(AGENT_PATH)?;
                    match manager
                        .call::<_, _, ()>("RegisterAgent", &(agent, "KeyboardDisplay"))
                        .await
                    {
                        Ok(()) => registered = true,
                        Err(error) => {
                            snapshot.error = Some(format!("Pairing agent unavailable: {error}"))
                        }
                    }
                }
            }
            if let Some(shared) = shared.upgrade() {
                publish(&shared, &snapshot);
            } else {
                break;
            }
        }
        let refresh_timeout = async {
            match refresh_at {
                Some(at) => tokio::time::sleep_until(at).await,
                None => std::future::pending().await,
            }
        };
        let timeout = async {
            match deadline {
                Some(at) => tokio::time::sleep_until(at).await,
                None => std::future::pending().await,
            }
        };
        // Only local mutations need publication before reconciliation. A signal
        // schedules a refresh, but has not changed the snapshot yet.
        publish_local = true;
        tokio::select! {
            // Owner changes and expired deadlines must win over a continuous signal burst.
            biased;
            _ = stop.changed() => break,
            message = owners.next() => {
                let Some(Ok(message)) = message else { bail!("system bus closed"); };
                if let Ok((name, _, new_owner)) = message.body().deserialize::<(String, String, String)>() {
                    if name == BLUEZ {
                        *current_owner.write().unwrap() = (!new_owner.is_empty()).then_some(new_owner);
                        epoch = BLUEZ_EPOCH.fetch_add(1, Ordering::Relaxed); tasks.abort_all(); pairing_task = None; waiter = None; *allowed.lock().unwrap() = None; registered = false; owned_scans.clear(); deadline = None; snapshot = Snapshot::default();
                        if current_owner.read().unwrap().is_none() { snapshot.availability = Availability::ServiceMissing; }
                        refresh_at = None;
                        refresh_now = true;
                    }
                }
            }
            _ = refresh_timeout => { refresh_now = true; publish_local = false; },
            _ = timeout => { stop_scan(&connection, &mut owned_scans).await; deadline = None; }
            changed = scan.changed() => {
                if changed.is_err() { break; }
                if !*scan.borrow_and_update() { stop_scan(&connection, &mut owned_scans).await; deadline = None; }
                else {
                    snapshot.error = None;
                    for adapter in snapshot.adapters.iter().filter(|a| a.powered) {
                        if owned_scans.contains(&adapter.id.path) { continue; }
                        let proxy = Proxy::new(&connection, BLUEZ, adapter.id.path.as_str(), "org.bluez.Adapter1").await?;
                        match tokio::time::timeout(Duration::from_secs(3), proxy.call::<_, _, ()>("StartDiscovery", &())).await {
                            Ok(Ok(())) => { owned_scans.insert(adapter.id.path.clone()); }
                            result => snapshot.error = Some(format!("Could not scan for devices: {result:?}")),
                        }
                    }
                    if owned_scans.is_empty() && snapshot.error.is_none() { snapshot.error = Some("Turn Bluetooth on before scanning".into()); }
                    deadline = Some(tokio::time::Instant::now() + Duration::from_secs(30));
                }
            }
            result = tasks.join_next(), if !tasks.is_empty() => {
                if let Some(Ok((action, result))) = result {
                    // Methods need not emit a signal (including failed/partial operations).
                    refresh_now = true;
                    match action {
                        Action::Power(id, _) => { snapshot.power_pending.remove(&id); }
                        Action::Connect(id, _) | Action::Pair(id) | Action::Cancel(id) | Action::Forget(id) => { snapshot.pending.remove(&id); if allowed.lock().unwrap().as_ref() == Some(&id) { *allowed.lock().unwrap() = None; snapshot.prompt = None; waiter = None; } }
                        Action::Answer(..) => {}
                    }
                    if let Err(error) = result { snapshot.error = Some(error); }
                }
            }
            event = events.recv() => {
                let Ok(event) = event else { break; };
                match event {
                    Event::Prompt(prompt, reply) => { if *allowed.lock().unwrap() == Some(prompt.device.clone()) { snapshot.prompt = Some(prompt.clone()); waiter = reply.map(|reply| (prompt.token, reply)); } }
                    Event::Clear(token) => { if snapshot.prompt.as_ref().is_some_and(|p| p.token == token) { snapshot.prompt = None; waiter = None; } }
                    Event::PanelClosed => {
                        stop_scan(&connection, &mut owned_scans).await; deadline = None;
                        snapshot.prompt = None; waiter = None;
                        let pairing = allowed.lock().unwrap().take();
                        if let Some(id) = pairing {
                            if let Some(task) = pairing_task.take() { task.abort(); }
                            snapshot.pending.insert(id.clone(), "Canceling…".into());
                            let connection = connection.clone();
                            tasks.spawn(async move { let action = Action::Cancel(id); let result = execute(connection, action.clone()).await.map_err(|e| e.to_string()); (action, result) });
                        }
                    }
                    Event::CancelPrompt => { snapshot.prompt = None; waiter = None; }
                    Event::Action(Action::Answer(token, answer)) => {
                        if snapshot.prompt.as_ref().is_some_and(|p| p.token == token) {
                            let answer = match answer { Some(input) => snapshot.prompt.as_ref().unwrap().validate(&input).map(Some), None => Ok(None) };
                            match answer { Ok(answer) => { if let Some((_, reply)) = waiter.take() { let _ = reply.send(answer); } snapshot.prompt = None; }, Err(error) => snapshot.error = Some(error.to_string()) }
                        }
                    }
                    Event::Action(action) => {
                        let valid = match &action { Action::Power(id, _) => id.epoch == epoch && snapshot.adapters.iter().any(|a| &a.id == id), Action::Connect(id, _) | Action::Pair(id) | Action::Cancel(id) | Action::Forget(id) => id.epoch == epoch && snapshot.devices.iter().any(|d| &d.id == id), Action::Answer(..) => false };
                        if !valid { snapshot.error = Some("Device changed; choose it again".into()); continue; }
                        match &action {
                            Action::Power(id, _) => { if !snapshot.power_pending.insert(id.clone()) { continue; } }
                            Action::Connect(id, connected) => { if snapshot.pending.contains_key(id) { continue; } snapshot.pending.insert(id.clone(), if *connected { "Connecting…" } else { "Disconnecting…" }.into()); }
                            Action::Pair(id) => {
                                if !registered { snapshot.error = Some("Pairing agent is unavailable".into()); continue; }
                                if allowed.lock().unwrap().is_some() || snapshot.pending.contains_key(id) { snapshot.error = Some("Finish the current pairing first".into()); continue; }
                                *allowed.lock().unwrap() = Some(id.clone()); snapshot.pending.insert(id.clone(), "Pairing…".into());
                            }
                            Action::Cancel(id) => {
                                waiter = None; snapshot.prompt = None;
                                if allowed.lock().unwrap().as_ref() == Some(id) {
                                    if let Some(task) = pairing_task.take() { task.abort(); }
                                    *allowed.lock().unwrap() = None;
                                }
                                snapshot.pending.insert(id.clone(), "Canceling…".into());
                            }
                            Action::Forget(id) => { if snapshot.pending.contains_key(id) { continue; } snapshot.pending.insert(id.clone(), "Forgetting…".into()); }
                            Action::Answer(..) => unreachable!(),
                        }
                        snapshot.error = None;
                        let connection = connection.clone();
                        let pairing = matches!(action, Action::Pair(_));
                        let task = tasks.spawn(async move { let result = tokio::time::timeout(Duration::from_secs(100), execute(connection, action.clone())).await.map_err(|_| "Bluetooth operation timed out".to_string()).and_then(|r| r.map_err(|e| format!("{e:#}"))); (action, result) });
                        if pairing { pairing_task = Some(task); }
                    }
                }
            }
            message = signals.next() => {
                publish_local = false;
                let Some(Ok(message)) = message else { bail!("Bluetooth signal stream closed"); };
                let header = message.header();
                let from_owner = header.sender().map(|s| s.as_str()) == current_owner.read().unwrap().as_deref()
                    && header.sender().is_some();
                let state_signal = matches!(
                    (header.interface().map(|i| i.as_str()), header.member().map(|m| m.as_str())),
                    (Some("org.freedesktop.DBus.Properties"), Some("PropertiesChanged"))
                        | (Some("org.freedesktop.DBus.ObjectManager"), Some("InterfacesAdded" | "InterfacesRemoved"))
                );
                if from_owner && state_signal {
                    refresh_at.get_or_insert_with(|| tokio::time::Instant::now() + REFRESH_WINDOW);
                }
            }
        }
    }
    tasks.abort_all();
    drop(waiter);
    stop_scan(&connection, &mut owned_scans).await;
    if registered {
        if let Ok(manager) =
            Proxy::new(&connection, BLUEZ, "/org/bluez", "org.bluez.AgentManager1").await
        {
            let _ = tokio::time::timeout(
                Duration::from_secs(2),
                manager.call::<_, _, ()>(
                    "UnregisterAgent",
                    &(OwnedObjectPath::try_from(AGENT_PATH)?,),
                ),
            )
            .await;
        }
    }
    Ok(())
}

/// A connected device's battery at or below this percentage is low.
const BATTERY_LOW: u8 = 15;
/// A low-battery warning re-arms once the battery is above this percentage.
const BATTERY_REARM: u8 = 20;

#[derive(Clone, Copy)]
struct Seen {
    connected: bool,
    /// Whether a fall to low battery would be reported.
    armed: bool,
}

/// Remembers each device between snapshots so the shell can send the notices for what changed:
/// a device connecting or disconnecting, and a connected device's battery falling low (once per
/// fall). What was already true when the watch first saw a ready adapter is not reported, and a
/// device that is first seen already connected is silent. A device that vanishes is forgotten
/// without a notice.
#[derive(Default)]
pub struct DeviceWatch {
    known: Option<BTreeMap<DeviceId, Seen>>,
}
impl DeviceWatch {
    pub fn observe(&mut self, snapshot: &Snapshot) -> Vec<dojo::notify::Notice> {
        use dojo::notify::{Notice, Urgency};
        // Only an adapter that was read says anything about devices: a missing service empties
        // the list, and its return is not a string of reconnections.
        if !matches!(
            snapshot.availability,
            Availability::Ready | Availability::NoAdapter
        ) {
            return Vec::new();
        }
        let before = self.known.take();
        let announce = before.is_some();
        let before = before.unwrap_or_default();
        let mut notices = Vec::new();
        let mut known = BTreeMap::new();
        for device in &snapshot.devices {
            let low = device.battery.is_some_and(|b| b <= BATTERY_LOW);
            let seen = match before.get(&device.id) {
                None => Seen {
                    connected: device.connected,
                    armed: !(device.connected && low),
                },
                Some(was) => {
                    let mut armed = was.armed;
                    if !device.connected {
                        armed = true;
                    } else if let Some(level) = device.battery {
                        if level > BATTERY_REARM {
                            armed = true;
                        } else if level <= BATTERY_LOW && armed {
                            armed = false;
                            notices.push(
                                Notice::new(
                                    format!("bluetooth-battery-{}", device.address),
                                    format!("{} battery low", device.name),
                                )
                                .body(format!("{level}% left"))
                                .urgency(Urgency::Normal),
                            );
                        }
                    }
                    if was.connected != device.connected {
                        let verb = if device.connected {
                            "connected"
                        } else {
                            "disconnected"
                        };
                        notices.push(
                            Notice::new(
                                format!("bluetooth-{}", device.address),
                                format!("{} {verb}", device.name),
                            )
                            .urgency(Urgency::Low),
                        );
                    }
                    Seen {
                        connected: device.connected,
                        armed,
                    }
                }
            };
            known.insert(device.id.clone(), seen);
        }
        self.known = Some(known);
        if announce { notices } else { Vec::new() }
    }
}

#[cfg(test)]
mod tests;
