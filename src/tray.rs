use std::collections::BTreeMap;
use std::sync::{Arc, Condvar, Mutex, RwLock, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::Context as _;
use futures_lite::future;
use gpui::{RenderImage, Window};
use image::{Frame, RgbaImage};
use zbus::blocking::{Connection, Proxy, connection::Builder as ConnectionBuilder};

#[cfg(test)]
mod runtime_regression {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};

    #[test]
    fn status_notifier_executor_runs_without_a_tokio_context() {
        let directory = tempfile::tempdir().unwrap();
        let configuration = directory.path().join("bus.conf");
        std::fs::write(&configuration, format!("<busconfig><type>session</type><listen>unix:tmpdir={}</listen><auth>EXTERNAL</auth><policy context=\"default\"><allow own=\"*\"/><allow send_destination=\"*\"/><allow receive_sender=\"*\"/></policy></busconfig>", directory.path().display())).unwrap();
        let mut daemon = Command::new("dbus-daemon")
            .arg("--config-file")
            .arg(&configuration)
            .args(["--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .spawn()
            .expect("start private D-Bus fixture");
        struct Daemon(std::process::Child);
        impl Drop for Daemon {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let mut address = String::new();
        BufReader::new(daemon.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        let _daemon = Daemon(daemon);
        let address = address.trim().to_owned();
        std::thread::spawn(move || {
            assert!(tokio::runtime::Handle::try_current().is_err());
            let connection = ConnectionBuilder::address(address.as_str())
                .unwrap()
                .build()
                .unwrap();
            // This is the bridge-spawn boundary used by run_host on its std thread.
            let bridge = connection
                .inner()
                .executor()
                .spawn(async { true }, "dojo-tray-runtime-regression");
            assert!(future::block_on(bridge).unwrap());
        })
        .join()
        .expect("tray bridge must work without a Tokio runtime");
    }
}
use zbus::fdo;
use zbus::interface;
use zbus::message::Header;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Structure, Value};

const WATCHER_NAME: &str = "org.freedesktop.StatusNotifierWatcher";
const KDE_WATCHER_NAME: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const WATCHER_INTERFACE: &str = "org.freedesktop.StatusNotifierWatcher";
const KDE_WATCHER_INTERFACE: &str = "org.kde.StatusNotifierWatcher";
const ITEM_INTERFACE: &str = "org.freedesktop.StatusNotifierItem";
const KDE_ITEM_INTERFACE: &str = "org.kde.StatusNotifierItem";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayPixmap {
    pub width: u32,
    pub height: u32,
    /// StatusNotifierItem pixmaps are transmitted as network-order ARGB32.
    pub argb: Arc<[u8]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayItem {
    pub key: String,
    pub service: String,
    pub object_path: String,
    pub title: String,
    pub status: String,
    pub icon_name: String,
    pub icon_pixmaps: Vec<TrayPixmap>,
    pub item_is_menu: bool,
    pub menu_path: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayMenuItem {
    pub id: i32,
    pub label: String,
    pub enabled: bool,
    pub visible: bool,
    pub separator: bool,
    pub checked: bool,
    pub has_submenu: bool,
    pub children: Vec<TrayMenuItem>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayMenu {
    pub revision: u32,
    pub items: Vec<TrayMenuItem>,
}

impl TrayItem {
    pub fn render_image(&self) -> Option<Arc<RenderImage>> {
        let pixmap = self
            .icon_pixmaps
            .iter()
            .min_by_key(|pixmap| pixmap.width.abs_diff(32) + pixmap.height.abs_diff(32))?;
        let expected_len = (pixmap.width as usize)
            .checked_mul(pixmap.height as usize)?
            .checked_mul(4)?;
        if pixmap.width == 0 || pixmap.height == 0 || pixmap.argb.len() != expected_len {
            return None;
        }

        let mut rgba = Vec::with_capacity(expected_len);
        for pixel in pixmap.argb.chunks_exact(4) {
            rgba.extend_from_slice(&[pixel[1], pixel[2], pixel[3], pixel[0]]);
        }
        let image = RgbaImage::from_raw(pixmap.width, pixmap.height, rgba)?;
        Some(Arc::new(RenderImage::new(vec![Frame::new(image)])))
    }
}

#[derive(Default)]
pub struct TrayImageCache {
    images: BTreeMap<String, CachedTrayImage>,
}

struct CachedTrayImage {
    pixmaps: Vec<TrayPixmap>,
    image: Arc<RenderImage>,
}

impl TrayImageCache {
    pub fn update(&mut self, items: &[TrayItem], window: &mut Window) {
        self.images.retain(|key, cached| {
            let unchanged = items.iter().any(|item| {
                item.key == *key && item.status != "Passive" && item.icon_pixmaps == cached.pixmaps
            });
            if !unchanged {
                if let Err(error) = window.drop_image(cached.image.clone()) {
                    eprintln!("Dojo could not release a tray image: {error}");
                }
            }
            unchanged
        });
        for item in items.iter().filter(|item| item.status != "Passive") {
            if !self.images.contains_key(&item.key) {
                if let Some(image) = item.render_image() {
                    self.images.insert(
                        item.key.clone(),
                        CachedTrayImage {
                            pixmaps: item.icon_pixmaps.clone(),
                            image,
                        },
                    );
                }
            }
        }
    }

    pub fn image(&self, key: &str) -> Option<Arc<RenderImage>> {
        self.images.get(key).map(|cached| cached.image.clone())
    }
}

#[derive(Clone, Copy)]
pub enum TrayAction {
    Activate,
    ContextMenu,
    SecondaryActivate,
}

#[derive(Clone)]
pub struct TrayHandle {
    items: Arc<RwLock<Arc<Vec<TrayItem>>>>,
    menus: Arc<RwLock<BTreeMap<String, TrayMenu>>>,
    commands: Option<mpsc::SyncSender<TrayCommand>>,
    wake: Arc<HostWake>,
    subscribers: Arc<Mutex<Vec<async_channel::Sender<()>>>>,
}

impl Default for TrayHandle {
    fn default() -> Self {
        Self {
            items: Arc::new(RwLock::new(Arc::new(Vec::new()))),
            menus: Arc::new(RwLock::new(BTreeMap::new())),
            commands: None,
            wake: Arc::default(),
            subscribers: Arc::default(),
        }
    }
}

impl TrayHandle {
    pub fn items(&self) -> Arc<Vec<TrayItem>> {
        self.items
            .read()
            .map(|items| items.clone())
            .unwrap_or_default()
    }

    pub fn subscribe_changes(&self) -> async_channel::Receiver<()> {
        let (sender, receiver) = async_channel::bounded(1);
        let state = self.wake.state.lock().unwrap();
        if state.shutdown {
            sender.close();
        } else {
            let mut subscribers = self.subscribers.lock().unwrap();
            subscribers.retain(|sender| !sender.is_closed());
            subscribers.push(sender);
        }
        receiver
    }

    pub fn activate(&self, item: &TrayItem, action: TrayAction) {
        self.send_command(TrayCommand::Activate {
            service: item.service.clone(),
            object_path: item.object_path.clone(),
            action,
        });
    }

    pub fn open_menu(&self, item: &TrayItem) {
        let Some(menu_path) = item.menu_path.as_ref() else {
            return;
        };
        self.send_command(TrayCommand::OpenMenu {
            key: item.key.clone(),
            service: item.service.clone(),
            menu_path: menu_path.clone(),
        });
    }

    pub fn menu(&self, key: &str) -> Option<TrayMenu> {
        self.menus.read().ok()?.get(key).cloned()
    }

    pub fn menu_event(&self, item: &TrayItem, id: i32) {
        self.send_command(TrayCommand::MenuEvent {
            service: item.service.clone(),
            menu_path: item.menu_path.clone().unwrap_or_default(),
            id,
        });
    }

    fn send_command(&self, command: TrayCommand) {
        if let Some(commands) = &self.commands {
            if let Err(error) = commands.try_send(command) {
                eprintln!("Dojo could not queue systray command: {error}");
            } else {
                self.wake.notify(false);
            }
        }
    }
}

enum TrayCommand {
    Activate {
        service: String,
        object_path: String,
        action: TrayAction,
    },
    OpenMenu {
        key: String,
        service: String,
        menu_path: String,
    },
    MenuEvent {
        service: String,
        menu_path: String,
        id: i32,
    },
}

pub struct TrayService {
    handle: TrayHandle,
    wake: Arc<HostWake>,
    worker: Option<JoinHandle<()>>,
}

impl TrayService {
    pub fn start() -> anyhow::Result<Self> {
        let items = Arc::new(RwLock::new(Arc::new(Vec::new())));
        let menus = Arc::new(RwLock::new(BTreeMap::new()));
        let registrations = Arc::new(Mutex::new(BTreeMap::new()));
        let (command_sender, command_receiver) = mpsc::sync_channel(32);
        let wake = Arc::new(HostWake::default());
        let subscribers: Arc<Mutex<Vec<async_channel::Sender<()>>>> = Arc::default();
        let worker_wake = wake.clone();
        let worker_subscribers = Arc::clone(&subscribers);
        let cleanup_wake = wake.clone();
        let worker_items = Arc::clone(&items);
        let worker_menus = Arc::clone(&menus);
        let worker_registrations = Arc::clone(&registrations);
        let worker = thread::Builder::new()
            .name("dojo-status-notifier".to_string())
            .spawn(move || {
                let cleanup_subscribers = worker_subscribers.clone();
                let cleanup_items = worker_items.clone();
                let cleanup_menus = worker_menus.clone();
                if let Err(error) = run_host(
                    worker_items,
                    worker_menus,
                    worker_registrations,
                    command_receiver,
                    worker_wake,
                    worker_subscribers,
                ) {
                    eprintln!("Dojo systray stopped: {error:#}");
                }
                cleanup_wake.shutdown();
                let mut changed = false;
                {
                    let mut items = cleanup_items.write().unwrap();
                    if !items.is_empty() {
                        *items = Arc::new(Vec::new());
                        changed = true;
                    }
                }
                {
                    let mut menus = cleanup_menus.write().unwrap();
                    if !menus.is_empty() {
                        menus.clear();
                        changed = true;
                    }
                }
                if changed {
                    publish_changes(&cleanup_subscribers);
                }
                cleanup_subscribers.lock().unwrap().clear();
            })
            .context("starting the StatusNotifierWatcher thread")?;

        Ok(Self {
            handle: TrayHandle {
                items,
                menus,
                commands: Some(command_sender),
                wake: wake.clone(),
                subscribers,
            },
            wake,
            worker: Some(worker),
        })
    }

    pub fn handle(&self) -> TrayHandle {
        self.handle.clone()
    }

    pub fn shutdown(mut self) {
        self.wake.shutdown();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for TrayService {
    fn drop(&mut self) {
        self.wake.shutdown();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[derive(Clone, Debug)]
struct ItemAddress {
    key: String,
    service: String,
    object_path: String,
}

#[derive(Clone)]
struct WatcherState {
    registrations: Arc<Mutex<BTreeMap<String, ItemAddress>>>,
    hosts: Arc<Mutex<BTreeMap<String, ()>>>,
    wake: Arc<HostWake>,
}

struct FreedesktopStatusNotifierWatcher(WatcherState);
struct KdeStatusNotifierWatcher(WatcherState);

impl WatcherState {
    fn registered_items(&self) -> Vec<String> {
        self.registrations
            .lock()
            .map(|items| items.keys().cloned().collect())
            .unwrap_or_default()
    }

    fn has_registered_host(&self) -> bool {
        self.hosts.lock().is_ok_and(|hosts| !hosts.is_empty())
    }

    fn register_item(&self, service_or_path: String, sender: Option<&str>) -> fdo::Result<String> {
        let address = parse_item_address(&service_or_path, sender)
            .ok_or_else(|| fdo::Error::InvalidArgs("item service/path is invalid".into()))?;
        let key = address.key.clone();
        self.registrations
            .lock()
            .map_err(|_| fdo::Error::Failed("item registry lock poisoned".into()))?
            .insert(key.clone(), address);
        self.wake.notify(true);
        Ok(key)
    }

    fn register_host(&self, service: String) -> fdo::Result<()> {
        self.hosts
            .lock()
            .map_err(|_| fdo::Error::Failed("host registry lock poisoned".into()))?
            .insert(service, ());
        Ok(())
    }
}

#[interface(name = "org.freedesktop.StatusNotifierWatcher")]
impl FreedesktopStatusNotifierWatcher {
    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        self.0.registered_items()
    }

    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        self.0.has_registered_host()
    }

    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }

    async fn register_status_notifier_item(
        &self,
        service_or_path: String,
        #[zbus(header)] header: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        let key = self
            .0
            .register_item(service_or_path, header.sender().map(|name| name.as_str()))?;
        Self::status_notifier_item_registered(emitter, key)
            .await
            .map_err(|error| fdo::Error::Failed(error.to_string()))?;
        Ok(())
    }

    async fn register_status_notifier_host(
        &self,
        service: String,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        self.0.register_host(service)?;
        Self::status_notifier_host_registered(emitter)
            .await
            .map_err(|error| fdo::Error::Failed(error.to_string()))?;
        Ok(())
    }

    #[zbus(signal)]
    async fn status_notifier_item_registered(
        emitter: SignalEmitter<'_>,
        service: String,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_item_unregistered(
        emitter: SignalEmitter<'_>,
        service: String,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_host_registered(emitter: SignalEmitter<'_>) -> zbus::Result<()>;
}

#[interface(name = "org.kde.StatusNotifierWatcher")]
impl KdeStatusNotifierWatcher {
    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        self.0.registered_items()
    }

    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        self.0.has_registered_host()
    }

    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }

    async fn register_status_notifier_item(
        &self,
        service_or_path: String,
        #[zbus(header)] header: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        let key = self
            .0
            .register_item(service_or_path, header.sender().map(|name| name.as_str()))?;
        Self::status_notifier_item_registered(emitter, key)
            .await
            .map_err(|error| fdo::Error::Failed(error.to_string()))?;
        Ok(())
    }

    async fn register_status_notifier_host(
        &self,
        service: String,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        self.0.register_host(service)?;
        Self::status_notifier_host_registered(emitter)
            .await
            .map_err(|error| fdo::Error::Failed(error.to_string()))?;
        Ok(())
    }

    #[zbus(signal)]
    async fn status_notifier_item_registered(
        emitter: SignalEmitter<'_>,
        service: String,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_item_unregistered(
        emitter: SignalEmitter<'_>,
        service: String,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_host_registered(emitter: SignalEmitter<'_>) -> zbus::Result<()>;
}

fn parse_item_address(service_or_path: &str, sender: Option<&str>) -> Option<ItemAddress> {
    let (service, object_path) = if service_or_path.starts_with('/') {
        (sender?, service_or_path)
    } else if let Some((service, _)) = service_or_path.split_once('/') {
        (service, &service_or_path[service.len()..])
    } else {
        (service_or_path, "/StatusNotifierItem")
    };
    let key = if object_path == "/StatusNotifierItem" && !service_or_path.starts_with('/') {
        service_or_path.to_string()
    } else {
        format!("{service}{object_path}")
    };
    zbus::names::BusName::try_from(service).ok()?;
    zbus::zvariant::ObjectPath::try_from(object_path).ok()?;
    Some(ItemAddress {
        key,
        service: service.to_string(),
        object_path: object_path.to_string(),
    })
}

#[derive(Default)]
struct HostWake {
    state: Mutex<WakeState>,
    changed: Condvar,
}

#[derive(Default)]
struct WakeState {
    pending: bool,
    refresh: bool,
    shutdown: bool,
}

impl HostWake {
    fn notify(&self, refresh: bool) {
        let mut state = self.state.lock().unwrap();
        state.pending = true;
        state.refresh |= refresh;
        self.changed.notify_one();
    }

    fn shutdown(&self) {
        let mut state = self.state.lock().unwrap();
        state.shutdown = true;
        self.changed.notify_one();
    }

    fn wait(&self, deadline: Option<Instant>) -> Option<bool> {
        let mut state = self.state.lock().unwrap();
        while !state.pending && !state.shutdown {
            state = if let Some(deadline) = deadline {
                let (state, timeout) = self
                    .changed
                    .wait_timeout(state, deadline.saturating_duration_since(Instant::now()))
                    .unwrap();
                if timeout.timed_out() {
                    return Some(true);
                }
                state
            } else {
                self.changed.wait(state).unwrap()
            };
        }
        if state.shutdown {
            return None;
        }
        state.pending = false;
        Some(std::mem::take(&mut state.refresh))
    }
}

fn publish_changes(subscribers: &Mutex<Vec<async_channel::Sender<()>>>) {
    subscribers
        .lock()
        .unwrap()
        .retain(|sender| match sender.try_send(()) {
            Ok(()) | Err(async_channel::TrySendError::Full(())) => true,
            Err(async_channel::TrySendError::Closed(())) => false,
        });
}

struct ConnectedItem {
    owner: String,
    proxy: Proxy<'static>,
}

struct ConnectedMenu {
    owner: String,
    path: String,
    proxy: Proxy<'static>,
}

fn owned_proxy(
    connection: &Connection,
    service: String,
    path: String,
    interface: &'static str,
) -> zbus::Result<Proxy<'static>> {
    zbus::blocking::proxy::Builder::new(connection)
        .destination(service)?
        .path(path)?
        .interface(interface)?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
}

fn connect_item(
    connection: &Connection,
    address: &ItemAddress,
) -> anyhow::Result<(Proxy<'static>, String)> {
    let proxy = owned_proxy(
        connection,
        address.service.clone(),
        address.object_path.clone(),
        ITEM_INTERFACE,
    )?;
    if let Ok(id) = proxy.get_property::<String>("Id") {
        return Ok((proxy, id));
    }
    let proxy = owned_proxy(
        connection,
        address.service.clone(),
        address.object_path.clone(),
        KDE_ITEM_INTERFACE,
    )?;
    let id = proxy.get_property::<String>("Id")?;
    Ok((proxy, id))
}

fn run_host(
    items: Arc<RwLock<Arc<Vec<TrayItem>>>>,
    menus: Arc<RwLock<BTreeMap<String, TrayMenu>>>,
    registrations: Arc<Mutex<BTreeMap<String, ItemAddress>>>,
    commands: mpsc::Receiver<TrayCommand>,
    wake: Arc<HostWake>,
    subscribers: Arc<Mutex<Vec<async_channel::Sender<()>>>>,
) -> anyhow::Result<()> {
    let host_name = format!(
        "org.freedesktop.StatusNotifierHost-{}-Dojo",
        std::process::id()
    );
    let watcher_state = WatcherState {
        registrations: registrations.clone(),
        hosts: Arc::default(),
        wake: wake.clone(),
    };
    let connection = ConnectionBuilder::session()?
        .method_timeout(Duration::from_secs(2))
        .name(WATCHER_NAME)?
        .name(KDE_WATCHER_NAME)?
        .name(host_name.as_str())?
        .serve_at(
            WATCHER_PATH,
            FreedesktopStatusNotifierWatcher(watcher_state.clone()),
        )?
        .serve_at(WATCHER_PATH, KdeStatusNotifierWatcher(watcher_state))?
        .build()?;
    let watcher_proxy = Proxy::new(&connection, WATCHER_NAME, WATCHER_PATH, WATCHER_INTERFACE)?;
    let _: () = watcher_proxy.call("RegisterStatusNotifierHost", &(host_name,))?;
    let owners = Arc::new(Mutex::new(BTreeMap::<String, String>::new()));
    let bridge_connection = connection.inner().clone();
    let bridge_wake = wake.clone();
    let bridge_owners = owners.clone();
    let bridge_registrations = registrations.clone();
    let bridge_items = items.clone();
    let (cancel, cancelled) = async_channel::bounded::<()>(1);
    let mut streams = Vec::new();
    for interface in [
        ITEM_INTERFACE,
        KDE_ITEM_INTERFACE,
        "org.freedesktop.DBus.Properties",
        "com.canonical.dbusmenu",
        "org.freedesktop.DBus",
    ] {
        let mut rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface(interface)?;
        if interface == "org.freedesktop.DBus" {
            rule = rule.member("NameOwnerChanged")?;
        }
        streams.push(future::block_on(zbus::MessageStream::for_match_rule(
            rule.build(),
            &bridge_connection,
            Some(8),
        ))?);
    }
    let bridge = connection.inner().executor().spawn(
        async move {
            loop {
                let message = future::race(
                    async {
                        future::poll_fn(|cx| {
                            for stream in &mut streams {
                                if let std::task::Poll::Ready(message) =
                                    futures_lite::Stream::poll_next(std::pin::Pin::new(stream), cx)
                                {
                                    return std::task::Poll::Ready(message);
                                }
                            }
                            std::task::Poll::Pending
                        })
                        .await
                    },
                    async {
                        let _ = cancelled.recv().await;
                        None
                    },
                )
                .await;
                let Some(Ok(message)) = message else {
                    break;
                };
                let header = message.header();
                let interface = header.interface().map(|s| s.as_str()).unwrap_or_default();
                let mut relevant = false;
                if interface == "org.freedesktop.DBus"
                    && header
                        .member()
                        .is_some_and(|m| m.as_str() == "NameOwnerChanged")
                {
                    if let Ok((name, old, _new)) =
                        message.body().deserialize::<(String, String, String)>()
                    {
                        relevant = bridge_registrations
                            .lock()
                            .unwrap()
                            .values()
                            .any(|address| address.service == name)
                            || bridge_owners
                                .lock()
                                .unwrap()
                                .values()
                                .any(|owner| owner == &old);
                    }
                } else if [
                    ITEM_INTERFACE,
                    KDE_ITEM_INTERFACE,
                    "org.freedesktop.DBus.Properties",
                    "com.canonical.dbusmenu",
                ]
                .contains(&interface)
                {
                    if let Some(sender) = header.sender() {
                        let sender_is_item = bridge_owners
                            .lock()
                            .unwrap()
                            .values()
                            .any(|owner| owner == sender.as_str());
                        let path = header.path().map(|path| path.as_str()).unwrap_or_default();
                        let item_path = bridge_registrations
                            .lock()
                            .unwrap()
                            .values()
                            .any(|address| address.object_path == path);
                        let menu_path = bridge_items
                            .read()
                            .unwrap()
                            .iter()
                            .any(|item| item.menu_path.as_deref() == Some(path));
                        relevant = sender_is_item && (item_path || menu_path);
                    }
                }
                if relevant {
                    bridge_wake.notify(true);
                }
            }
            for stream in streams {
                zbus::AsyncDrop::async_drop(stream).await;
            }
            bridge_wake.shutdown();
        },
        "dojo-tray-signals",
    );
    let bus = zbus::blocking::fdo::DBusProxy::new(&connection)?;
    let mut cached = BTreeMap::<String, ConnectedItem>::new();
    let mut menu_proxies = BTreeMap::<String, ConnectedMenu>::new();
    let mut deadline = None;
    wake.notify(true);
    while let Some(refresh) = wake.wait(deadline) {
        if refresh {
            let addresses: Vec<_> = registrations.lock().unwrap().values().cloned().collect();
            let mut next = Vec::new();
            let mut changed = false;
            let mut removed = Vec::new();
            for address in addresses {
                if wake.state.lock().unwrap().shutdown {
                    break;
                }
                let result = (|| -> anyhow::Result<Option<TrayItem>> {
                    let owner = bus
                        .get_name_owner(address.service.as_str().try_into()?)?
                        .to_string();
                    if cached
                        .get(&address.key)
                        .is_none_or(|entry| entry.owner != owner)
                    {
                        let owned_address = ItemAddress {
                            service: owner.clone(),
                            ..address.clone()
                        };
                        let (proxy, _) = connect_item(&connection, &owned_address)?;
                        cached.insert(
                            address.key.clone(),
                            ConnectedItem {
                                owner: owner.clone(),
                                proxy,
                            },
                        );
                        changed |= menus.write().unwrap().remove(&address.key).is_some();
                        menu_proxies.remove(&address.key);
                    }
                    owners
                        .lock()
                        .unwrap()
                        .insert(address.service.clone(), owner);
                    read_item(&cached[&address.key].proxy, &address)
                })();
                match result {
                    Ok(Some(item)) => next.push(item),
                    Ok(None) => {}
                    Err(error) => {
                        eprintln!("Dojo lost systray item {}: {error}", address.key);
                        removed.push(address);
                    }
                }
            }
            for address in removed {
                registrations.lock().unwrap().remove(&address.key);
                cached.remove(&address.key);
                menu_proxies.remove(&address.key);
                if !registrations
                    .lock()
                    .unwrap()
                    .values()
                    .any(|registered| registered.service == address.service)
                {
                    owners.lock().unwrap().remove(&address.service);
                }
                for interface in [WATCHER_INTERFACE, KDE_WATCHER_INTERFACE] {
                    let _ = connection.emit_signal(
                        None::<&str>,
                        WATCHER_PATH,
                        interface,
                        "StatusNotifierItemUnregistered",
                        &address.key,
                    );
                }
            }
            {
                let mut current = items.write().unwrap();
                if **current != next {
                    let mut menu_cache = menus.write().unwrap();
                    for old in current.iter() {
                        if next
                            .iter()
                            .find(|item| item.key == old.key)
                            .is_none_or(|item| item.menu_path != old.menu_path)
                        {
                            menu_cache.remove(&old.key);
                        }
                    }
                    *current = Arc::new(next);
                    changed = true;
                }
            }
            {
                let current = items.read().unwrap();
                let mut menus = menus.write().unwrap();
                let before = menus.len();
                menus.retain(|key, _| current.iter().any(|item| item.key == *key));
                changed |= menus.len() != before;
                menu_proxies.retain(|key, menu| {
                    current.iter().any(|item| {
                        item.key == *key && item.menu_path.as_deref() == Some(menu.path.as_str())
                    })
                });
                for (key, menu) in &menu_proxies {
                    if menus.contains_key(key) {
                        match read_menu_layout(&menu.proxy, 0) {
                            Ok((revision, items)) => {
                                let next = TrayMenu { revision, items };
                                if menus.get(key) != Some(&next) {
                                    menus.insert(key.clone(), next);
                                    changed = true;
                                }
                            }
                            Err(error) => {
                                changed |= menus.remove(key).is_some();
                                eprintln!("Dojo lost tray menu: {error:#}");
                            }
                        }
                    }
                }
            }
            if changed {
                publish_changes(&subscribers);
            }
            deadline = if registrations.lock().unwrap().is_empty() {
                None
            } else {
                Some(Instant::now() + Duration::from_secs(30))
            };
        }
        for _ in 0..32 {
            if wake.state.lock().unwrap().shutdown {
                break;
            }
            let Ok(command) = commands.try_recv() else {
                break;
            };
            match command {
                TrayCommand::Activate {
                    service,
                    object_path,
                    action,
                } => {
                    let current = items.read().unwrap();
                    let entry = current
                        .iter()
                        .find(|item| item.service == service && item.object_path == object_path)
                        .and_then(|item| cached.get(&item.key));
                    if let Some(entry) = entry {
                        if let Err(error) = activate_item(&entry.proxy, action) {
                            eprintln!("Dojo could not activate systray item: {error:#}");
                        }
                    }
                }
                TrayCommand::OpenMenu {
                    key,
                    service,
                    menu_path,
                } => {
                    let current = items.read().unwrap();
                    if !current.iter().any(|item| {
                        item.key == key
                            && item.service == service
                            && item.menu_path.as_deref() == Some(menu_path.as_str())
                    }) {
                        continue;
                    }
                    drop(current);
                    let Some(item) = cached.get(&key) else {
                        continue;
                    };
                    let result = (|| -> anyhow::Result<TrayMenu> {
                        if menu_proxies
                            .get(&key)
                            .is_none_or(|menu| menu.owner != item.owner || menu.path != menu_path)
                        {
                            let proxy = owned_proxy(
                                &connection,
                                item.owner.clone(),
                                menu_path.clone(),
                                "com.canonical.dbusmenu",
                            )?;
                            menu_proxies.insert(
                                key.clone(),
                                ConnectedMenu {
                                    owner: item.owner.clone(),
                                    path: menu_path,
                                    proxy,
                                },
                            );
                        }
                        let proxy = &menu_proxies[&key].proxy;
                        let _: bool = proxy.call("AboutToShow", &(0i32,))?;
                        let (revision, items) =
                            read_menu_layout_at_depth(proxy, 0, 0, true, &mut 512)?;
                        Ok(TrayMenu { revision, items })
                    })();
                    match result {
                        Ok(menu) => {
                            let mut current = menus.write().unwrap();
                            if current.get(&key) != Some(&menu) {
                                current.insert(key, menu);
                                publish_changes(&subscribers);
                            }
                        }
                        Err(error) => eprintln!("Dojo could not read tray menu: {error:#}"),
                    }
                }
                TrayCommand::MenuEvent {
                    service,
                    menu_path,
                    id,
                } => {
                    let current = items.read().unwrap();
                    let entry = current
                        .iter()
                        .find(|item| {
                            item.service == service
                                && item.menu_path.as_deref() == Some(menu_path.as_str())
                        })
                        .and_then(|item| menu_proxies.get(&item.key));
                    if let Some(entry) = entry {
                        if let Err(error) = send_menu_event(&entry.proxy, id) {
                            eprintln!("Dojo could not activate tray menu item: {error:#}");
                        }
                    }
                }
            }
        }
    }
    let _ = cancel.try_send(());
    if let Err(error) = future::block_on(bridge) {
        eprintln!("Dojo tray signal bridge stopped unexpectedly: {error}");
    }
    Ok(())
}

fn validated_pixmap(width: i32, height: i32, argb: Vec<u8>) -> Option<TrayPixmap> {
    let width = u32::try_from(width).ok()?;
    let height = u32::try_from(height).ok()?;
    if width == 0 || height == 0 || width > 512 || height > 512 {
        return None;
    }
    let expected = (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(4)?;
    (argb.len() == expected).then_some(TrayPixmap {
        width,
        height,
        argb: argb.into(),
    })
}

fn read_item(proxy: &Proxy<'_>, address: &ItemAddress) -> anyhow::Result<Option<TrayItem>> {
    let id = proxy.get_property::<String>("Id")?;
    let status = proxy
        .get_property::<String>("Status")
        .unwrap_or_else(|_| "Active".to_string());
    if status == "Passive" {
        return Ok(None);
    }

    let title = proxy.get_property::<String>("Title").unwrap_or(id);
    let icon_name = proxy.get_property::<String>("IconName").unwrap_or_default();
    let icon_pixmaps = proxy
        .get_property::<Vec<(i32, i32, Vec<u8>)>>("IconPixmap")
        .map(|pixmaps| {
            if pixmaps.len() > 16 {
                eprintln!(
                    "Dojo limited tray pixmaps to 16 variants for {}",
                    address.key
                );
            }
            pixmaps
        })
        .unwrap_or_default()
        .into_iter()
        .take(16)
        .filter_map(|(width, height, argb)| {
            let pixmap = validated_pixmap(width, height, argb);
            if pixmap.is_none() {
                eprintln!(
                    "Dojo rejected invalid or oversized tray pixmap from {}",
                    address.key
                );
            }
            pixmap
        })
        .collect();
    let item_is_menu = proxy.get_property::<bool>("ItemIsMenu").unwrap_or(false);
    let menu_path = proxy
        .get_property::<OwnedObjectPath>("Menu")
        .ok()
        .map(|path| path.to_string());

    Ok(Some(TrayItem {
        key: address.key.clone(),
        service: address.service.clone(),
        object_path: address.object_path.clone(),
        title,
        status,
        icon_name,
        icon_pixmaps,
        item_is_menu,
        menu_path,
    }))
}

fn read_menu_layout(proxy: &Proxy<'_>, parent_id: i32) -> anyhow::Result<(u32, Vec<TrayMenuItem>)> {
    read_menu_layout_at_depth(proxy, parent_id, 0, false, &mut 512)
}

fn read_menu_layout_at_depth(
    proxy: &Proxy<'_>,
    parent_id: i32,
    depth: usize,
    prepare: bool,
    remaining: &mut usize,
) -> anyhow::Result<(u32, Vec<TrayMenuItem>)> {
    anyhow::ensure!(depth < 16, "tray menu nesting exceeds 16 levels");
    let (revision, root): (
        u32,
        (
            i32,
            std::collections::HashMap<String, OwnedValue>,
            Vec<OwnedValue>,
        ),
    ) = proxy.call("GetLayout", &(parent_id, 1i32, Vec::<String>::new()))?;
    let (_, properties, children) = root;
    let mut items = Vec::with_capacity(children.len());
    anyhow::ensure!(
        children.len() <= *remaining,
        "tray menu exceeds 512 total entries"
    );
    *remaining -= children.len();
    for child in children {
        let structure = Structure::try_from(child)?;
        let (id, properties, _): (
            i32,
            std::collections::HashMap<String, OwnedValue>,
            Vec<OwnedValue>,
        ) = structure.try_into()?;
        let label = property_string(&properties, "label").unwrap_or_default();
        let visible = property_bool(&properties, "visible").unwrap_or(true);
        let enabled = property_bool(&properties, "enabled").unwrap_or(true);
        let separator = property_string(&properties, "type").as_deref() == Some("separator");
        let checked = property_i32(&properties, "toggle-state").unwrap_or(0) == 1;
        let has_submenu =
            property_string(&properties, "children-display").as_deref() == Some("submenu");
        let children = if has_submenu {
            if prepare {
                let _: bool = proxy.call("AboutToShow", &(id,))?;
            }
            read_menu_layout_at_depth(proxy, id, depth + 1, prepare, remaining)?.1
        } else {
            Vec::new()
        };
        items.push(TrayMenuItem {
            id,
            label,
            enabled,
            visible,
            separator,
            checked,
            has_submenu,
            children,
        });
    }
    let _ = properties;
    Ok((revision, items))
}

fn property_string(
    properties: &std::collections::HashMap<String, OwnedValue>,
    key: &str,
) -> Option<String> {
    properties.get(key)?.try_clone().ok()?.try_into().ok()
}

fn property_bool(
    properties: &std::collections::HashMap<String, OwnedValue>,
    key: &str,
) -> Option<bool> {
    properties.get(key)?.try_into().ok()
}

fn property_i32(
    properties: &std::collections::HashMap<String, OwnedValue>,
    key: &str,
) -> Option<i32> {
    properties.get(key)?.try_into().ok()
}

fn send_menu_event(proxy: &Proxy<'_>, id: i32) -> anyhow::Result<()> {
    proxy.call_noreply("Event", &(id, "clicked", Value::from(0i32), 0u32))?;
    Ok(())
}

fn activate_item(proxy: &Proxy<'_>, action: TrayAction) -> anyhow::Result<()> {
    let method = match action {
        TrayAction::Activate => "Activate",
        TrayAction::ContextMenu => "ContextMenu",
        TrayAction::SecondaryActivate => "SecondaryActivate",
    };
    let _: () = proxy.call(method, &(0i32, 0i32))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_coalesce_and_remove_closed_subscribers() {
        let handle = TrayHandle::default();
        let first = handle.subscribe_changes();
        let second = handle.subscribe_changes();
        publish_changes(&handle.subscribers);
        publish_changes(&handle.subscribers);
        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1);
        drop(first);
        publish_changes(&handle.subscribers);
        assert_eq!(handle.subscribers.lock().unwrap().len(), 1);
    }

    #[test]
    fn shutdown_interrupts_an_empty_wait_and_bypasses_pending_work() {
        let wake = Arc::new(HostWake::default());
        let waiting = wake.clone();
        let worker = thread::spawn(move || waiting.wait(None));
        wake.notify(true);
        wake.shutdown();
        // The first wake may race with shutdown. The next wait always stops.
        let _ = worker.join().unwrap();
        assert_eq!(wake.wait(None), None);
    }

    #[test]
    fn pixmaps_reject_invalid_dimensions_and_lengths() {
        assert!(validated_pixmap(0, 1, vec![]).is_none());
        assert!(validated_pixmap(-1, 1, vec![]).is_none());
        assert!(validated_pixmap(513, 1, vec![]).is_none());
        assert!(validated_pixmap(1, 1, vec![255, 1, 2]).is_none());
        assert_eq!(
            validated_pixmap(1, 1, vec![255, 1, 2, 3])
                .unwrap()
                .argb
                .as_ref(),
            &[255, 1, 2, 3]
        );
    }

    #[test]
    fn cloned_pixmaps_share_immutable_pixels() {
        let pixmap = validated_pixmap(1, 1, vec![255, 1, 2, 3]).unwrap();
        let cloned = pixmap.clone();
        assert!(Arc::ptr_eq(&pixmap.argb, &cloned.argb));
        assert_eq!(cloned.argb.as_ref(), &[255, 1, 2, 3]);
    }

    #[test]
    fn shared_network_argb_pixels_render_as_rgba() {
        let item = TrayItem {
            key: "pixels".into(),
            service: String::new(),
            object_path: String::new(),
            title: String::new(),
            status: "Active".into(),
            icon_name: String::new(),
            icon_pixmaps: vec![validated_pixmap(1, 1, vec![128, 10, 20, 30]).unwrap()],
            item_is_menu: false,
            menu_path: None,
        };
        let cloned = item.clone();
        assert!(Arc::ptr_eq(
            &item.icon_pixmaps[0].argb,
            &cloned.icon_pixmaps[0].argb
        ));
        assert_eq!(
            cloned.render_image().unwrap().as_bytes(0),
            Some([10, 20, 30, 128].as_slice())
        );
    }

    #[test]
    fn parses_service_names_and_object_paths() {
        let named_service = parse_item_address("org.example.TrayItem", None).unwrap();
        assert_eq!(named_service.key, "org.example.TrayItem");
        assert_eq!(named_service.service, "org.example.TrayItem");
        assert_eq!(named_service.object_path, "/StatusNotifierItem");

        let explicit_path = parse_item_address("org.example.TrayItem/Item", None).unwrap();
        assert_eq!(explicit_path.service, "org.example.TrayItem");
        assert_eq!(explicit_path.object_path, "/Item");

        let sender_path = parse_item_address("/Item", Some(":1.42")).unwrap();
        assert_eq!(sender_path.key, ":1.42/Item");
        assert_eq!(sender_path.service, ":1.42");
        assert_eq!(sender_path.object_path, "/Item");
        assert!(parse_item_address("/Item", None).is_none());
    }

    #[test]
    fn watcher_interfaces_share_registration_state() -> anyhow::Result<()> {
        let state = WatcherState {
            registrations: Arc::default(),
            hosts: Arc::default(),
            wake: Arc::default(),
        };
        let freedesktop = FreedesktopStatusNotifierWatcher(state.clone());
        let kde = KdeStatusNotifierWatcher(state.clone());

        assert_eq!(WATCHER_NAME, "org.freedesktop.StatusNotifierWatcher");
        assert_eq!(KDE_WATCHER_NAME, "org.kde.StatusNotifierWatcher");
        assert_eq!(
            <FreedesktopStatusNotifierWatcher as zbus::object_server::Interface>::name().as_str(),
            WATCHER_INTERFACE
        );
        assert_eq!(
            <KdeStatusNotifierWatcher as zbus::object_server::Interface>::name().as_str(),
            KDE_WATCHER_INTERFACE
        );

        let key = state.register_item("org.example.TrayItem".into(), None)?;
        state.register_host("org.freedesktop.StatusNotifierHost-test".into())?;

        assert_eq!(freedesktop.0.registered_items(), vec![key.clone()]);
        assert_eq!(kde.0.registered_items(), vec![key]);
        assert!(freedesktop.0.has_registered_host());
        assert!(kde.0.has_registered_host());
        Ok(())
    }

    #[test]
    fn opening_a_tray_menu_queues_its_dbus_request() -> anyhow::Result<()> {
        let (commands, queued) = mpsc::sync_channel(32);
        let handle = TrayHandle {
            items: Arc::default(),
            menus: Arc::default(),
            commands: Some(commands),
            wake: Arc::default(),
            subscribers: Arc::default(),
        };
        let item = TrayItem {
            key: ":1.702".into(),
            service: ":1.702".into(),
            object_path: "/StatusNotifierItem".into(),
            title: String::new(),
            status: "Active".into(),
            icon_name: String::new(),
            icon_pixmaps: Vec::new(),
            item_is_menu: false,
            menu_path: Some("/com/canonical/dbusmenu".into()),
        };

        handle.open_menu(&item);

        match queued.try_recv()? {
            TrayCommand::OpenMenu {
                key,
                service,
                menu_path,
            } => {
                assert_eq!(key, item.key);
                assert_eq!(service, item.service);
                assert_eq!(menu_path, "/com/canonical/dbusmenu");
            }
            _ => return Err(anyhow::Error::msg("menu request was not queued")),
        }
        Ok(())
    }
}
