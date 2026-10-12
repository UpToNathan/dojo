//! Run under dbus-run-session to verify tray signals without touching the desktop bus.
#[path = "../src/tray.rs"]
mod tray;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};
use tray::{TrayAction, TrayService};
use zbus::{
    blocking::{Connection, connection::Builder},
    interface,
    zvariant::{OwnedObjectPath, OwnedValue},
};

#[derive(Default)]
struct State {
    title: String,
    status: String,
    icon: Vec<(i32, i32, Vec<u8>)>,
    revision: u32,
}
struct Item {
    state: Arc<Mutex<State>>,
    reads: Arc<AtomicUsize>,
}
impl Item {
    fn read(&self) {
        self.reads.fetch_add(1, Ordering::Relaxed);
    }
}
#[interface(name = "org.freedesktop.StatusNotifierItem")]
impl Item {
    #[zbus(property)]
    fn id(&self) -> &str {
        self.read();
        "probe"
    }
    #[zbus(property)]
    fn title(&self) -> String {
        self.read();
        self.state.lock().unwrap().title.clone()
    }
    #[zbus(property)]
    fn status(&self) -> String {
        self.read();
        self.state.lock().unwrap().status.clone()
    }
    #[zbus(property)]
    fn icon_name(&self) -> &str {
        self.read();
        ""
    }
    #[zbus(property)]
    fn icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        self.read();
        self.state.lock().unwrap().icon.clone()
    }
    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
        self.read();
        false
    }
    #[zbus(property)]
    fn menu(&self) -> OwnedObjectPath {
        self.read();
        "/Menu".try_into().unwrap()
    }
    fn activate(&self, _x: i32, _y: i32) {}
}
struct Menu(Arc<Mutex<State>>);
#[interface(name = "com.canonical.dbusmenu")]
impl Menu {
    fn about_to_show(&self, _id: i32) -> bool {
        false
    }
    fn get_layout(
        &self,
        _parent: i32,
        _depth: i32,
        _properties: Vec<String>,
    ) -> (
        u32,
        (
            i32,
            std::collections::HashMap<String, OwnedValue>,
            Vec<OwnedValue>,
        ),
    ) {
        (
            self.0.lock().unwrap().revision,
            (0, Default::default(), Vec::new()),
        )
    }
}
fn wait_until(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready() {
        assert!(Instant::now() < deadline, "tray update timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn signal(connection: &Connection, member: &str) -> zbus::Result<()> {
    connection.emit_signal(
        None::<&str>,
        "/StatusNotifierItem",
        "org.freedesktop.StatusNotifierItem",
        member,
        &(),
    )
}
fn main() -> anyhow::Result<()> {
    let tray = TrayService::start()?;
    let handle = tray.handle();
    let changes = handle.subscribe_changes();
    let state = Arc::new(Mutex::new(State {
        title: "first".into(),
        status: "Active".into(),
        icon: vec![(1, 1, vec![255, 1, 2, 3])],
        revision: 1,
    }));
    let reads = Arc::new(AtomicUsize::new(0));
    let provider = Builder::session()?
        .name("org.example.DojoTrayProbe")?
        .serve_at(
            "/StatusNotifierItem",
            Item {
                state: state.clone(),
                reads: reads.clone(),
            },
        )?
        .serve_at("/Menu", Menu(state.clone()))?
        .build()?;
    let watcher = zbus::blocking::Proxy::new(
        &provider,
        "org.freedesktop.StatusNotifierWatcher",
        "/StatusNotifierWatcher",
        "org.freedesktop.StatusNotifierWatcher",
    )?;
    // Watcher thread starts asynchronously.
    wait_until(|| {
        watcher
            .call::<_, _, ()>(
                "RegisterStatusNotifierItem",
                &("org.example.DojoTrayProbe",),
            )
            .is_ok()
    });
    wait_until(|| {
        handle
            .items()
            .first()
            .is_some_and(|item| item.title == "first")
    });
    state.lock().unwrap().title = "signal-title".into();
    signal(&provider, "NewTitle")?;
    wait_until(|| {
        handle
            .items()
            .first()
            .is_some_and(|item| item.title == "signal-title")
    });
    state.lock().unwrap().title = "properties-title".into();
    provider.emit_signal(
        None::<&str>,
        "/StatusNotifierItem",
        "org.freedesktop.DBus.Properties",
        "PropertiesChanged",
        &(
            "org.freedesktop.StatusNotifierItem",
            std::collections::HashMap::<String, OwnedValue>::new(),
            vec!["Title"],
        ),
    )?;
    wait_until(|| {
        handle
            .items()
            .first()
            .is_some_and(|item| item.title == "properties-title")
    });
    state.lock().unwrap().icon = vec![(1, 1, vec![255, 9, 8, 7])];
    signal(&provider, "NewIcon")?;
    wait_until(|| {
        handle
            .items()
            .first()
            .is_some_and(|item| item.icon_pixmaps[0].argb.as_ref() == [255, 9, 8, 7])
    });
    state.lock().unwrap().status = "Passive".into();
    signal(&provider, "NewStatus")?;
    wait_until(|| handle.items().is_empty());
    state.lock().unwrap().status = "Active".into();
    signal(&provider, "NewStatus")?;
    wait_until(|| !handle.items().is_empty());
    let item = handle.items()[0].clone();
    handle.open_menu(&item);
    wait_until(|| {
        handle
            .menu(&item.key)
            .is_some_and(|menu| menu.revision == 1)
    });
    state.lock().unwrap().revision = 2;
    provider.emit_signal(
        None::<&str>,
        "/Menu",
        "com.canonical.dbusmenu",
        "LayoutUpdated",
        &(2u32, 0i32),
    )?;
    wait_until(|| {
        handle
            .menu(&item.key)
            .is_some_and(|menu| menu.revision == 2)
    });
    std::thread::sleep(Duration::from_millis(100));
    let before = reads.load(Ordering::Relaxed);
    std::thread::sleep(Duration::from_secs(1));
    assert_eq!(
        reads.load(Ordering::Relaxed),
        before,
        "idle tray fetched properties"
    );
    let snapshot = handle.items();
    assert!(Arc::ptr_eq(&snapshot, &handle.items()));
    provider.release_name("org.example.DojoTrayProbe")?;
    wait_until(|| handle.items().is_empty() && handle.menu(&item.key).is_none());
    // Flood a bounded queue while shutting down. Shutdown bypasses queued commands.
    for _ in 0..64 {
        handle.activate(&item, TrayAction::Activate);
    }
    let start = Instant::now();
    tray.shutdown();
    assert!(
        start.elapsed() < Duration::from_secs(3),
        "shutdown drained the command backlog"
    );
    while changes.try_recv().is_ok() {}
    assert!(changes.is_closed(), "shutdown retained change subscribers");
    println!(
        "PASS registration, NewTitle/NewIcon/NewStatus, dbusmenu, idle Gets=0, shared snapshot, removal, shutdown"
    );
    Ok(())
}
