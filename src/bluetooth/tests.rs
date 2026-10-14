use super::*;

#[test]
fn pairing_inputs_are_validated() {
    let mut prompt = Prompt {
        token: 1,
        device: DeviceId {
            path: "/org/bluez/hci0/dev_01".into(),
            epoch: 1,
        },
        kind: PromptKind::Pin,
    };
    assert_eq!(prompt.validate("1234").unwrap(), "1234");
    for invalid in ["", "contains spaces", "12345678901234567"] {
        assert!(prompt.validate(invalid).is_err());
    }
    prompt.kind = PromptKind::Passkey;
    assert_eq!(prompt.validate("000123").unwrap(), "000123");
    for invalid in ["", "-1", "1000000", "12x"] {
        assert!(prompt.validate(invalid).is_err());
    }
}

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize};

struct PrivateBus {
    child: Child,
    address: String,
    _config: tempfile::NamedTempFile,
}
impl PrivateBus {
    fn start() -> Self {
        use std::io::Write;
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
#[derive(Default)]
struct FakeState {
    reads: AtomicUsize,
    powered: AtomicBool,
    connected: AtomicBool,
    paired: AtomicBool,
    fail_connect: AtomicBool,
    scans: AtomicUsize,
    cancel_calls: AtomicUsize,
    connects: AtomicUsize,
    agent: Mutex<Option<(String, OwnedObjectPath)>>,
    pairing_mode: AtomicUsize,
    answers: Mutex<Vec<String>>,
}
struct FakeManager(Arc<FakeState>);
#[interface(name = "org.bluez.AgentManager1")]
impl FakeManager {
    fn register_agent(
        &self,
        agent: OwnedObjectPath,
        capability: String,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) {
        assert_eq!(capability, "KeyboardDisplay");
        *self.0.agent.lock().unwrap() = Some((header.sender().unwrap().to_string(), agent));
    }
    fn unregister_agent(&self, _agent: OwnedObjectPath) {
        *self.0.agent.lock().unwrap() = None;
    }
}
struct FakeAdapter(Arc<FakeState>);
#[interface(name = "org.bluez.Adapter1")]
impl FakeAdapter {
    #[zbus(property)]
    fn alias(&self) -> &str {
        "Test adapter"
    }
    #[zbus(property)]
    fn powered(&self) -> bool {
        self.0.powered.load(Ordering::SeqCst)
    }
    #[zbus(property)]
    fn set_powered(&mut self, powered: bool) {
        self.0.powered.store(powered, Ordering::SeqCst);
    }
    #[zbus(property)]
    fn discovering(&self) -> bool {
        self.0.scans.load(Ordering::SeqCst) > 0
    }
    fn start_discovery(&self) {
        self.0.scans.fetch_add(1, Ordering::SeqCst);
    }
    fn stop_discovery(&self) {
        assert!(
            self.0.scans.fetch_sub(1, Ordering::SeqCst) > 1,
            "Stopped discovery not owned by Dojo"
        );
    }
}
struct FakeDevice(Arc<FakeState>);
#[interface(name = "org.bluez.Device1")]
impl FakeDevice {
    #[zbus(property)]
    fn alias(&self) -> &str {
        "Test headphones"
    }
    #[zbus(property)]
    fn address(&self) -> &str {
        "01:02:03:04:05:06"
    }
    #[zbus(property)]
    fn icon(&self) -> &str {
        "audio-headphones"
    }
    #[zbus(property)]
    fn paired(&self) -> bool {
        self.0.paired.load(Ordering::SeqCst)
    }
    #[zbus(property)]
    fn connected(&self) -> bool {
        self.0.connected.load(Ordering::SeqCst)
    }
    fn connect(&self) -> zbus::fdo::Result<()> {
        if self.0.fail_connect.load(Ordering::SeqCst) {
            return Err(zbus::fdo::Error::Failed(
                "Connection denied by test device".into(),
            ));
        }
        self.0.connects.fetch_add(1, Ordering::SeqCst);
        self.0.connected.store(true, Ordering::SeqCst);
        Ok(())
    }
    fn disconnect(&self) {
        self.0.connected.store(false, Ordering::SeqCst);
    }
    fn cancel_pairing(&self) {
        self.0.cancel_calls.fetch_add(1, Ordering::SeqCst);
    }
    async fn pair(&self, #[zbus(connection)] connection: &Connection) -> zbus::fdo::Result<()> {
        let (name, path) = self.0.agent.lock().unwrap().clone().unwrap();
        let agent = Proxy::new(connection, name.as_str(), path.as_str(), "org.bluez.Agent1")
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        let device = OwnedObjectPath::try_from("/org/bluez/hci0/dev_01_02_03_04_05_06").unwrap();
        let result = match self.0.pairing_mode.load(Ordering::SeqCst) {
            1 => {
                agent
                    .call::<_, _, String>("RequestPinCode", &(device,))
                    .await
            }
            2 => agent
                .call::<_, _, u32>("RequestPasskey", &(device,))
                .await
                .map(|key| format!("{key:06}")),
            _ => agent
                .call::<_, _, ()>("RequestConfirmation", &(device, 482619_u32))
                .await
                .map(|_| "confirmed".into()),
        }
        .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        self.0.answers.lock().unwrap().push(result);
        self.0.paired.store(true, Ordering::SeqCst);
        Ok(())
    }
}
struct FakeBattery;
#[interface(name = "org.bluez.Battery1")]
impl FakeBattery {
    #[zbus(property)]
    fn percentage(&self) -> u8 {
        62
    }
}
struct CountedObjects(Arc<FakeState>);
#[interface(name = "org.freedesktop.DBus.ObjectManager")]
impl CountedObjects {
    fn get_managed_objects(&self) -> Objects {
        self.0.reads.fetch_add(1, Ordering::SeqCst);
        let properties = |entries: Vec<(&str, OwnedValue)>| {
            entries
                .into_iter()
                .map(|(key, value)| (key.into(), value))
                .collect()
        };
        let text = |value: &str| OwnedValue::from(zbus::zvariant::Str::from(value));
        Objects::from([
            (
                OwnedObjectPath::try_from("/org/bluez/hci0").unwrap(),
                std::collections::HashMap::from([(
                    "org.bluez.Adapter1".into(),
                    properties(vec![
                        ("Alias", text("Test adapter")),
                        ("Powered", self.0.powered.load(Ordering::SeqCst).into()),
                    ]),
                )]),
            ),
            (
                OwnedObjectPath::try_from("/org/bluez/hci0/dev_01_02_03_04_05_06").unwrap(),
                std::collections::HashMap::from([
                    (
                        "org.bluez.Device1".into(),
                        properties(vec![
                            ("Alias", text("Test headphones")),
                            ("Address", text("01:02:03:04:05:06")),
                            ("Icon", text("audio-headphones")),
                            ("Paired", self.0.paired.load(Ordering::SeqCst).into()),
                            ("Connected", self.0.connected.load(Ordering::SeqCst).into()),
                        ]),
                    ),
                    (
                        "org.bluez.Battery1".into(),
                        properties(vec![("Percentage", 62_u8.into())]),
                    ),
                ]),
            ),
        ])
    }
}
async fn state_signal(server: &Connection) {
    server
        .emit_signal(
            None::<&str>,
            "/org/bluez/hci0/dev_01_02_03_04_05_06",
            "org.freedesktop.DBus.Properties",
            "PropertiesChanged",
            &(
                "org.bluez.Device1",
                std::collections::HashMap::<String, OwnedValue>::new(),
                vec!["Paired", "Connected"],
            ),
        )
        .await
        .unwrap();
}
async fn fake_bluez(address: &str, state: Arc<FakeState>) -> Connection {
    zbus::connection::Builder::address(address)
        .unwrap()
        .name(BLUEZ)
        .unwrap()
        .serve_at("/", CountedObjects(state.clone()))
        .unwrap()
        .serve_at("/org/bluez", FakeManager(state.clone()))
        .unwrap()
        .serve_at("/org/bluez/hci0", FakeAdapter(state.clone()))
        .unwrap()
        .serve_at("/org/bluez/hci0/dev_01_02_03_04_05_06", FakeDevice(state))
        .unwrap()
        .serve_at("/org/bluez/hci0/dev_01_02_03_04_05_06", FakeBattery)
        .unwrap()
        .build()
        .await
        .unwrap()
}
async fn wait(controller: &Controller, predicate: impl Fn(&Snapshot) -> bool) -> Arc<Snapshot> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let snapshot = controller.snapshot();
        if predicate(&snapshot) {
            return snapshot;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "Timed out with {snapshot:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn signal_bursts_are_coalesced_without_starving_refresh() {
    let bus = PrivateBus::start();
    let state = Arc::new(FakeState::default());
    let server = fake_bluez(&bus.address, state.clone()).await;
    let (controller, _changes) = Controller::start_with_address(Some(bus.address.clone())).unwrap();
    wait(&controller, |s| s.availability == Availability::Ready).await;
    tokio::time::sleep(REFRESH_WINDOW * 2).await;
    let baseline = state.reads.load(Ordering::SeqCst);
    assert_eq!(baseline, 1, "initial state needs exactly one read");
    let publications = controller.0.publications.load(Ordering::SeqCst);
    state.connected.store(true, Ordering::SeqCst);
    for _ in 0..32 {
        state_signal(&server).await;
    }
    wait(&controller, |s| s.devices[0].connected).await;
    tokio::time::sleep(REFRESH_WINDOW * 2).await;
    assert_eq!(state.reads.load(Ordering::SeqCst), baseline + 1);
    assert_eq!(
        controller.0.publications.load(Ordering::SeqCst) - publications,
        1,
        "signal bursts must not clone/compare the snapshot per signal"
    );

    state.connected.store(false, Ordering::SeqCst);
    let start = tokio::time::Instant::now();
    while start.elapsed() < REFRESH_WINDOW * 5 {
        state_signal(&server).await;
        tokio::time::sleep(Duration::from_millis(5)).await;
        if start.elapsed() > REFRESH_WINDOW * 3 {
            assert!(
                !controller.snapshot().devices[0].connected,
                "a continuous burst must not postpone reconciliation"
            );
        }
    }
    tokio::time::sleep(REFRESH_WINDOW * 2).await;
    let reads = state.reads.load(Ordering::SeqCst) - baseline - 1;
    assert!(
        (2..=7).contains(&reads),
        "expected bounded fixed-window reads, got {reads}"
    );
    let baseline = state.reads.load(Ordering::SeqCst);
    let publications = controller.0.publications.load(Ordering::SeqCst);
    server
        .emit_signal(
            None::<&str>,
            "/org/bluez/hci0",
            "org.bluez.Test",
            "Unrelated",
            &(),
        )
        .await
        .unwrap();
    tokio::time::sleep(REFRESH_WINDOW * 2).await;
    assert_eq!(state.reads.load(Ordering::SeqCst), baseline);
    assert_eq!(
        controller.0.publications.load(Ordering::SeqCst),
        publications,
        "irrelevant signals must do no snapshot publication work"
    );

    // Object topology signals use the same window as property signals.
    state.connected.store(true, Ordering::SeqCst);
    let path = OwnedObjectPath::try_from("/org/bluez/hci0/dev_01_02_03_04_05_06").unwrap();
    for _ in 0..16 {
        server
            .emit_signal(
                None::<&str>,
                "/",
                "org.freedesktop.DBus.ObjectManager",
                "InterfacesAdded",
                &(
                    path.clone(),
                    std::collections::HashMap::<
                        String,
                        std::collections::HashMap<String, OwnedValue>,
                    >::new(),
                ),
            )
            .await
            .unwrap();
        server
            .emit_signal(
                None::<&str>,
                "/",
                "org.freedesktop.DBus.ObjectManager",
                "InterfacesRemoved",
                &(path.clone(), vec!["org.bluez.Battery1"]),
            )
            .await
            .unwrap();
    }
    wait(&controller, |s| s.devices[0].connected).await;
    tokio::time::sleep(REFRESH_WINDOW * 2).await;
    assert_eq!(state.reads.load(Ordering::SeqCst), baseline + 1);
}

#[tokio::test]
async fn owner_restart_discards_pending_window_and_old_device_ids() {
    let bus = PrivateBus::start();
    let old_state = Arc::new(FakeState::default());
    let old_server = fake_bluez(&bus.address, old_state).await;
    let (controller, _changes) = Controller::start_with_address(Some(bus.address.clone())).unwrap();
    let initial = wait(&controller, |s| s.availability == Availability::Ready).await;
    let old_device = initial.devices[0].id.clone();
    state_signal(&old_server).await; // A pending coalescing window belongs to this owner.
    old_server.release_name(BLUEZ).await.unwrap();
    wait(&controller, |s| {
        s.availability == Availability::ServiceMissing && s.devices.is_empty()
    })
    .await;
    let state = Arc::new(FakeState::default());
    state.connected.store(true, Ordering::SeqCst);
    let server = fake_bluez(&bus.address, state.clone()).await;
    let restarted = wait(&controller, |s| s.availability == Availability::Ready).await;
    assert_ne!(restarted.devices[0].id, old_device);
    assert!(restarted.devices[0].connected);
    for _ in 0..32 {
        state_signal(&old_server).await;
    }
    tokio::time::sleep(REFRESH_WINDOW * 3).await;
    assert_eq!(
        state.reads.load(Ordering::SeqCst),
        1,
        "old owner's signals/window must not refresh the replacement service"
    );
    controller
        .submit(Action::Connect(old_device, false))
        .unwrap();
    wait(&controller, |s| {
        s.error.as_deref() == Some("Device changed; choose it again")
    })
    .await;
    tokio::time::sleep(REFRESH_WINDOW * 2).await;
    assert_eq!(state.reads.load(Ordering::SeqCst), 1);
    assert!(controller.snapshot().devices[0].connected);
    state.connected.store(false, Ordering::SeqCst);
    state_signal(&server).await;
    wait(&controller, |s| !s.devices[0].connected).await;
    assert_eq!(state.reads.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn local_prompt_events_do_not_read_managed_objects() {
    let bus = PrivateBus::start();
    let state = Arc::new(FakeState::default());
    state.pairing_mode.store(1, Ordering::SeqCst);
    let _server = fake_bluez(&bus.address, state.clone()).await;
    let (controller, _changes) = Controller::start_with_address(Some(bus.address.clone())).unwrap();
    let initial = wait(&controller, |s| s.availability == Availability::Ready).await;
    tokio::time::sleep(REFRESH_WINDOW * 2).await;
    let baseline = state.reads.load(Ordering::SeqCst);
    let device = initial.devices[0].id.clone();
    controller.submit(Action::Pair(device)).unwrap();
    let pairing = wait(&controller, |s| s.prompt.is_some()).await;
    let prompt = pairing.prompt.clone().unwrap();
    controller
        .submit(Action::Answer(prompt.token, Some("bad pin!".into())))
        .unwrap();
    wait(&controller, |s| {
        s.error.as_deref() == Some("Enter a PIN of 1–16 letters or digits")
    })
    .await;
    assert!(controller.snapshot().prompt.is_some());
    controller
        .submit(Action::Answer(prompt.token + 1, None))
        .unwrap();
    controller
        .0
        .events
        .send(Event::Clear(prompt.token + 1))
        .await
        .unwrap();
    tokio::time::sleep(REFRESH_WINDOW * 2).await;
    assert_eq!(
        state.reads.load(Ordering::SeqCst),
        baseline,
        "pair initiation, prompt delivery, validation and stale tokens are local"
    );
    controller
        .0
        .events
        .send(Event::Clear(prompt.token))
        .await
        .unwrap();
    wait(&controller, |s| s.prompt.is_none()).await;
    wait(&controller, |s| s.pending.is_empty()).await; // Dropped reply finishes Pair.
    tokio::time::sleep(REFRESH_WINDOW * 2).await;
    assert_eq!(
        state.reads.load(Ordering::SeqCst),
        baseline + 1,
        "only action completion should reconcile, not prompt/answer/clear events"
    );
    let baseline = state.reads.load(Ordering::SeqCst);
    controller.0.events.send(Event::CancelPrompt).await.unwrap();
    controller.close_panel(); // No owned scan or pairing: entirely local.
    tokio::time::sleep(REFRESH_WINDOW * 2).await;
    assert_eq!(state.reads.load(Ordering::SeqCst), baseline);
}

#[tokio::test]
async fn real_dbus_discovery_connections_pairing_and_restart() {
    let bus = PrivateBus::start();
    let (controller, _changes) = Controller::start_with_address(Some(bus.address.clone())).unwrap();
    wait(&controller, |s| {
        s.availability == Availability::ServiceMissing
    })
    .await;
    let state = Arc::new(FakeState::default());
    state.scans.store(1, Ordering::SeqCst); // Another client owns the first discovery session.
    let server = fake_bluez(&bus.address, state.clone()).await;
    let initial = wait(&controller, |s| s.availability == Availability::Ready).await;
    assert_eq!(initial.devices[0].name, "Test headphones");
    assert_eq!(initial.devices[0].battery, Some(62));
    assert!(!initial.devices[0].connected);
    assert!(!initial.scanning);
    let adapter = initial.adapters[0].id.clone();
    let device = initial.devices[0].id.clone();
    controller.submit(Action::Power(adapter, true)).unwrap();
    wait(&controller, |s| {
        s.adapters[0].powered && s.power_pending.is_empty()
    })
    .await;
    controller.scan(true);
    wait(&controller, |s| s.scanning).await;
    assert_eq!(state.scans.load(Ordering::SeqCst), 2);
    controller.close_panel();
    wait(&controller, |s| !s.scanning).await;
    assert_eq!(state.scans.load(Ordering::SeqCst), 1);
    controller
        .submit(Action::Connect(device.clone(), true))
        .unwrap();
    wait(&controller, |s| {
        s.devices[0].connected && s.pending.is_empty()
    })
    .await;
    assert_eq!(state.connects.load(Ordering::SeqCst), 1);
    controller
        .submit(Action::Connect(device.clone(), false))
        .unwrap();
    wait(&controller, |s| {
        !s.devices[0].connected && s.pending.is_empty()
    })
    .await;
    state.fail_connect.store(true, Ordering::SeqCst);
    controller
        .submit(Action::Connect(device.clone(), true))
        .unwrap();
    let failed = wait(&controller, |s| s.error.is_some()).await;
    assert!(failed.error.as_ref().unwrap().contains("Connection denied"));
    assert!(!failed.devices[0].connected);
    state.fail_connect.store(false, Ordering::SeqCst);
    for (mode, answer, expected) in [
        (0, "", "confirmed"),
        (1, "1234", "1234"),
        (2, "000123", "000123"),
    ] {
        state.pairing_mode.store(mode, Ordering::SeqCst);
        state.paired.store(false, Ordering::SeqCst);
        state_signal(&server).await;
        wait(&controller, |s| !s.devices[0].paired).await;
        controller.submit(Action::Pair(device.clone())).unwrap();
        let pairing = wait(&controller, |s| s.prompt.is_some()).await;
        assert!(!pairing.devices[0].paired);
        assert!(!pairing.devices[0].connected);
        let prompt = pairing.prompt.as_ref().unwrap();
        if mode == 0 {
            assert_eq!(prompt.kind, PromptKind::Confirm(482619));
        }
        controller
            .submit(Action::Answer(prompt.token, Some(answer.into())))
            .unwrap();
        wait(&controller, |s| {
            s.devices[0].paired && s.devices[0].connected && s.pending.is_empty()
        })
        .await;
        assert_eq!(state.answers.lock().unwrap().last().unwrap(), expected);
        controller
            .submit(Action::Connect(device.clone(), false))
            .unwrap();
        wait(&controller, |s| {
            !s.devices[0].connected && s.pending.is_empty()
        })
        .await;
    }
    state.paired.store(false, Ordering::SeqCst);
    controller.submit(Action::Pair(device.clone())).unwrap();
    let pairing = wait(&controller, |s| s.prompt.is_some()).await;
    controller
        .submit(Action::Answer(pairing.prompt.as_ref().unwrap().token, None))
        .unwrap();
    wait(&controller, |s| s.pending.is_empty() && s.error.is_some()).await;
    assert!(!state.paired.load(Ordering::SeqCst));
    let connects = state.connects.load(Ordering::SeqCst);
    controller.submit(Action::Pair(device.clone())).unwrap();
    wait(&controller, |s| s.prompt.is_some()).await;
    controller.close_panel();
    wait(&controller, |s| s.prompt.is_none() && s.pending.is_empty()).await;
    assert_eq!(state.cancel_calls.load(Ordering::SeqCst), 1);
    assert_eq!(state.connects.load(Ordering::SeqCst), connects);
    controller.submit(Action::Pair(device.clone())).unwrap();
    wait(&controller, |s| s.prompt.is_some()).await;
    server.release_name(BLUEZ).await.unwrap();
    wait(&controller, |s| {
        s.availability == Availability::ServiceMissing && s.prompt.is_none()
    })
    .await;
    server.request_name(BLUEZ).await.unwrap();
    let restarted = wait(&controller, |s| s.availability == Availability::Ready).await;
    assert_ne!(restarted.devices[0].id, device);
    controller.submit(Action::Connect(device, true)).unwrap();
    let stale = wait(&controller, |s| {
        s.error.as_deref() == Some("Device changed; choose it again")
    })
    .await;
    assert!(!stale.devices[0].connected);
    controller.scan(true);
    wait(&controller, |s| s.scanning).await;
    drop(controller);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while state.scans.load(Ordering::SeqCst) != 1 || state.agent.lock().unwrap().is_some() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "Bluetooth worker did not shut down"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
