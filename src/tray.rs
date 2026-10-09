use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, RwLock, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::Context as _;
use gpui::RenderImage;
use image::{Frame, RgbaImage};
use zbus::blocking::{Connection, Proxy, connection::Builder as ConnectionBuilder};
use zbus::fdo;
use zbus::interface;
use zbus::message::Header;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Structure, Value};

const WATCHER_NAME: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const WATCHER_INTERFACE: &str = "org.kde.StatusNotifierWatcher";
const ITEM_INTERFACE: &str = "org.kde.StatusNotifierItem";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayPixmap {
    pub width: u32,
    pub height: u32,
    /// StatusNotifierItem pixmaps are transmitted as network-order ARGB32.
    pub argb: Vec<u8>,
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

#[derive(Clone, Copy)]
pub enum TrayAction {
    Activate,
    ContextMenu,
    SecondaryActivate,
}

#[derive(Clone)]
pub struct TrayHandle {
    items: Arc<RwLock<Vec<TrayItem>>>,
    menus: Arc<RwLock<BTreeMap<String, TrayMenu>>>,
    commands: Option<mpsc::Sender<TrayCommand>>,
}

impl Default for TrayHandle {
    fn default() -> Self {
        Self {
            items: Arc::new(RwLock::new(Vec::new())),
            menus: Arc::new(RwLock::new(BTreeMap::new())),
            commands: None,
        }
    }
}

impl TrayHandle {
    pub fn items(&self) -> Vec<TrayItem> {
        self.items
            .read()
            .map(|items| items.clone())
            .unwrap_or_default()
    }

    pub fn activate(&self, item: &TrayItem, action: TrayAction) {
        if let Some(commands) = &self.commands {
            let _ = commands.send(TrayCommand::Activate {
                service: item.service.clone(),
                object_path: item.object_path.clone(),
                action,
            });
        }
    }

    pub fn open_menu(&self, item: &TrayItem) {
        let Some(menu_path) = item.menu_path.as_ref() else {
            return;
        };
        if let Some(commands) = &self.commands {
            let _ = commands.send(TrayCommand::OpenMenu {
                key: item.key.clone(),
                service: item.service.clone(),
                menu_path: menu_path.clone(),
            });
        }
    }

    pub fn menu(&self, key: &str) -> Option<TrayMenu> {
        self.menus.read().ok()?.get(key).cloned()
    }

    pub fn menu_event(&self, item: &TrayItem, id: i32) {
        if let Some(commands) = &self.commands {
            let _ = commands.send(TrayCommand::MenuEvent {
                service: item.service.clone(),
                menu_path: item.menu_path.clone().unwrap_or_default(),
                id,
            });
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
    shutdown: mpsc::Sender<()>,
    worker: Option<JoinHandle<()>>,
}

impl TrayService {
    pub fn start() -> anyhow::Result<Self> {
        let items = Arc::new(RwLock::new(Vec::new()));
        let menus = Arc::new(RwLock::new(BTreeMap::new()));
        let registrations = Arc::new(Mutex::new(BTreeMap::new()));
        let (command_sender, command_receiver) = mpsc::channel();
        let (shutdown_sender, shutdown_receiver) = mpsc::channel();
        let worker_items = Arc::clone(&items);
        let worker_menus = Arc::clone(&menus);
        let worker_registrations = Arc::clone(&registrations);
        let worker = thread::Builder::new()
            .name("dojo-status-notifier".to_string())
            .spawn(move || {
                if let Err(error) = run_host(
                    worker_items,
                    worker_menus,
                    worker_registrations,
                    command_receiver,
                    shutdown_receiver,
                ) {
                    eprintln!("Dojo systray stopped: {error:#}");
                }
            })
            .context("starting the StatusNotifierWatcher thread")?;

        Ok(Self {
            handle: TrayHandle {
                items,
                menus,
                commands: Some(command_sender),
            },
            shutdown: shutdown_sender,
            worker: Some(worker),
        })
    }

    pub fn handle(&self) -> TrayHandle {
        self.handle.clone()
    }

    pub fn shutdown(mut self) {
        let _ = self.shutdown.send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for TrayService {
    fn drop(&mut self) {
        let _ = self.shutdown.send(());
    }
}

#[derive(Clone, Debug)]
struct ItemAddress {
    key: String,
    service: String,
    object_path: String,
}

struct StatusNotifierWatcher {
    registrations: Arc<Mutex<BTreeMap<String, ItemAddress>>>,
    hosts: Arc<Mutex<BTreeMap<String, ()>>>,
}

#[interface(name = "org.kde.StatusNotifierWatcher")]
impl StatusNotifierWatcher {
    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        self.registrations
            .lock()
            .map(|items| items.keys().cloned().collect())
            .unwrap_or_default()
    }

    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        self.hosts.lock().is_ok_and(|hosts| !hosts.is_empty())
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
        let address =
            parse_item_address(&service_or_path, header.sender().map(|name| name.as_str()))
                .ok_or_else(|| fdo::Error::InvalidArgs("item service/path is invalid".into()))?;
        let key = address.key.clone();
        self.registrations
            .lock()
            .map_err(|_| fdo::Error::Failed("item registry lock poisoned".into()))?
            .insert(key.clone(), address);
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
        self.hosts
            .lock()
            .map_err(|_| fdo::Error::Failed("host registry lock poisoned".into()))?
            .insert(service, ());
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
    Some(ItemAddress {
        key,
        service: service.to_string(),
        object_path: object_path.to_string(),
    })
}

fn run_host(
    items: Arc<RwLock<Vec<TrayItem>>>,
    menus: Arc<RwLock<BTreeMap<String, TrayMenu>>>,
    registrations: Arc<Mutex<BTreeMap<String, ItemAddress>>>,
    commands: mpsc::Receiver<TrayCommand>,
    shutdown: mpsc::Receiver<()>,
) -> anyhow::Result<()> {
    let host_name = format!(
        "org.freedesktop.StatusNotifierHost-{}-Dojo",
        std::process::id()
    );
    let hosts = Arc::new(Mutex::new(BTreeMap::new()));
    let connection = ConnectionBuilder::session()?
        .method_timeout(Duration::from_secs(2))
        .name(WATCHER_NAME)?
        .name(host_name.as_str())?
        .serve_at(
            WATCHER_PATH,
            StatusNotifierWatcher {
                registrations: Arc::clone(&registrations),
                hosts: Arc::clone(&hosts),
            },
        )?
        .build()
        .context("connecting the systray to the session bus")?;

    let watcher_proxy = Proxy::new(&connection, WATCHER_NAME, WATCHER_PATH, WATCHER_INTERFACE)?;
    let _: () = watcher_proxy.call("RegisterStatusNotifierHost", &(host_name,))?;

    let mut last_refresh = Instant::now() - Duration::from_millis(750);
    loop {
        if shutdown.try_recv().is_ok() {
            break;
        }

        while let Ok(command) = commands.try_recv() {
            match command {
                TrayCommand::Activate {
                    service,
                    object_path,
                    action,
                } => {
                    if let Err(error) = activate_item(&connection, &service, &object_path, action) {
                        eprintln!("Dojo could not activate systray item {service}: {error:#}");
                    }
                }
                TrayCommand::OpenMenu {
                    key,
                    service,
                    menu_path,
                } => match read_menu(&connection, &service, &menu_path) {
                    Ok(menu) => {
                        if let Ok(mut menus) = menus.write() {
                            menus.insert(key, menu);
                        }
                    }
                    Err(error) => {
                        eprintln!("Dojo could not read tray menu {service}: {error:#}")
                    }
                },
                TrayCommand::MenuEvent {
                    service,
                    menu_path,
                    id,
                } => {
                    if let Err(error) = send_menu_event(&connection, &service, &menu_path, id) {
                        eprintln!("Dojo could not activate tray menu item: {error:#}");
                    }
                }
            }
        }

        if last_refresh.elapsed() >= Duration::from_millis(750) {
            refresh_items(&connection, &items, &registrations);
            last_refresh = Instant::now();
        }
        thread::sleep(Duration::from_millis(25));
    }

    Ok(())
}

fn refresh_items(
    connection: &Connection,
    items: &RwLock<Vec<TrayItem>>,
    registrations: &Mutex<BTreeMap<String, ItemAddress>>,
) {
    let addresses = match registrations.lock() {
        Ok(registrations) => registrations.values().cloned().collect::<Vec<_>>(),
        Err(_) => return,
    };

    let mut next_items = Vec::with_capacity(addresses.len());
    let mut disappeared = Vec::new();
    for address in addresses {
        match read_item(connection, &address) {
            Ok(Some(item)) => next_items.push(item),
            Ok(None) => {}
            Err(error) => {
                eprintln!("Dojo lost systray item {}: {error}", address.key);
                disappeared.push(address.key);
            }
        }
    }

    if let Ok(mut registrations) = registrations.lock() {
        for key in disappeared {
            registrations.remove(&key);
            if let Err(error) = connection.emit_signal(
                None::<&str>,
                WATCHER_PATH,
                WATCHER_INTERFACE,
                "StatusNotifierItemUnregistered",
                &key,
            ) {
                eprintln!("Dojo could not announce removed systray item: {error}");
            }
        }
    }
    if let Ok(mut current) = items.write() {
        *current = next_items;
    }
}

fn read_item(connection: &Connection, address: &ItemAddress) -> anyhow::Result<Option<TrayItem>> {
    let proxy = Proxy::new(
        connection,
        address.service.as_str(),
        address.object_path.as_str(),
        ITEM_INTERFACE,
    )
    .with_context(|| format!("connecting to {}", address.key))?;

    let id = proxy
        .get_property::<String>("Id")
        .with_context(|| format!("reading the required Id property from {}", address.key))?;
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
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(width, height, argb)| {
            Some(TrayPixmap {
                width: u32::try_from(width).ok()?,
                height: u32::try_from(height).ok()?,
                argb,
            })
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

fn read_menu(connection: &Connection, service: &str, menu_path: &str) -> anyhow::Result<TrayMenu> {
    let proxy = Proxy::new(connection, service, menu_path, "com.canonical.dbusmenu")?;
    let (revision, root) = read_menu_layout(&proxy, 0)?;
    Ok(TrayMenu {
        revision,
        items: root,
    })
}

fn read_menu_layout(proxy: &Proxy<'_>, parent_id: i32) -> anyhow::Result<(u32, Vec<TrayMenuItem>)> {
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
            read_menu_layout(proxy, id)?.1
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

fn send_menu_event(
    connection: &Connection,
    service: &str,
    menu_path: &str,
    id: i32,
) -> anyhow::Result<()> {
    let proxy = Proxy::new(connection, service, menu_path, "com.canonical.dbusmenu")?;
    let _: () = proxy.call("Event", &(id, "clicked", Value::from(0i32), 0u32))?;
    Ok(())
}

fn activate_item(
    connection: &Connection,
    service: &str,
    object_path: &str,
    action: TrayAction,
) -> anyhow::Result<()> {
    let proxy = Proxy::new(connection, service, object_path, ITEM_INTERFACE)
        .with_context(|| format!("connecting to systray item {service}"))?;
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
}
