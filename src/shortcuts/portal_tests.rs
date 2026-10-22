use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

use futures_lite::StreamExt;
use zbus::message::Type;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};
use zbus::{Connection, MatchRule, Message, MessageStream};

use super::*;
use crate::shortcuts::registry::Grab;
use crate::shortcuts::trigger::Trigger;

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
struct FakeCompositor {
    sets: Mutex<Vec<Vec<Grab>>>,
    refuse_key: Mutex<Option<String>>,
}

impl FakeCompositor {
    fn latest(&self) -> Vec<Grab> {
        self.sets.lock().unwrap().last().cloned().unwrap_or_default()
    }
}

impl Compositor for FakeCompositor {
    fn set_grabs(&self, grabs: &[Grab]) -> anyhow::Result<Vec<super::super::compositor::Refusal>> {
        self.sets.lock().unwrap().push(grabs.to_vec());
        let refuse = self.refuse_key.lock().unwrap().clone();
        Ok(grabs
            .iter()
            .filter(|grab| Some(&grab.key) == refuse.as_ref())
            .map(|grab| (grab.id.clone(), "already a configured bind".to_owned()))
            .collect())
    }
}

/// Answers from a script and records what it was asked.
#[derive(Default)]
struct Scripted {
    interactive: bool,
    answers: Mutex<VecDeque<Option<Vec<Choice>>>>,
    asked: Mutex<Vec<ChoiceRequest>>,
}

#[async_trait]
impl Approver for Scripted {
    async fn choose(&self, request: ChoiceRequest) -> Option<Vec<Choice>> {
        self.asked.lock().unwrap().push(request);
        self.answers.lock().unwrap().pop_front().flatten()
    }

    fn interactive(&self) -> bool {
        self.interactive
    }
}

/// Never answers, so only the frontend's Close can end the call.
struct Silent;

#[async_trait]
impl Approver for Silent {
    async fn choose(&self, _request: ChoiceRequest) -> Option<Vec<Choice>> {
        std::future::pending().await
    }
}

struct Harness {
    _bus: PrivateBus,
    client: Connection,
    compositor: Arc<FakeCompositor>,
    events: async_channel::Sender<KeyEvent>,
    portal: Portal,
}

async fn harness(approver: Arc<dyn Approver>, store: Option<Store>) -> Harness {
    let bus = PrivateBus::start();
    let compositor = Arc::new(FakeCompositor::default());
    let (events, receiver) = async_channel::unbounded();
    let address = bus.address.clone();
    let portal = {
        let compositor = compositor.clone();
        tokio::task::spawn_blocking(move || {
            Portal::start(Some(address), store, compositor, receiver, approver).unwrap()
        })
        .await
        .unwrap()
    };
    let client = zbus::connection::Builder::address(bus.address.as_str())
        .unwrap()
        .build()
        .await
        .unwrap();
    Harness {
        _bus: bus,
        client,
        compositor,
        events,
        portal,
    }
}

type Reply = (u32, HashMap<String, OwnedValue>);

fn opts() -> HashMap<&'static str, Value<'static>> {
    HashMap::new()
}

async fn call(client: &Connection, member: &str, body: &(impl serde::Serialize + zbus::zvariant::DynamicType)) -> Message {
    client
        .call_method(Some(BUS_NAME), OBJECT_PATH, Some(INTERFACE), member, body)
        .await
        .unwrap()
}

async fn create_session(client: &Connection, session: &str, app: &str) -> Reply {
    let message = call(
        client,
        "CreateSession",
        &(
            ObjectPath::try_from("/request/create").unwrap(),
            ObjectPath::try_from(session).unwrap(),
            app,
            opts(),
        ),
    )
    .await;
    message.body().deserialize().unwrap()
}

fn shortcut(id: &str, description: &str, preferred: Option<&str>) -> (String, HashMap<String, Value<'static>>) {
    let mut properties = HashMap::new();
    properties.insert("description".to_owned(), Value::from(description.to_owned()));
    if let Some(preferred) = preferred {
        properties.insert("preferred_trigger".to_owned(), Value::from(preferred.to_owned()));
    }
    (id.to_owned(), properties)
}

async fn bind(
    client: &Connection,
    session: &str,
    request: &str,
    shortcuts: Vec<(String, HashMap<String, Value<'static>>)>,
) -> Reply {
    let message = call(
        client,
        "BindShortcuts",
        &(
            ObjectPath::try_from(request).unwrap(),
            ObjectPath::try_from(session).unwrap(),
            shortcuts,
            "",
            opts(),
        ),
    )
    .await;
    message.body().deserialize().unwrap()
}

fn listed(reply: &Reply) -> Vec<(String, String, String)> {
    let shortcuts: Vec<(String, HashMap<String, OwnedValue>)> = reply
        .1
        .get("shortcuts")
        .map(|value| value.try_clone().unwrap().try_into().unwrap())
        .unwrap_or_default();
    shortcuts
        .into_iter()
        .map(|(id, properties)| {
            let text = |key: &str| {
                String::try_from(properties[key].try_clone().unwrap()).unwrap()
            };
            (id, text("description"), text("trigger_description"))
        })
        .collect()
}

async fn signals(client: &Connection, member: &str) -> MessageStream {
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .interface(INTERFACE)
        .unwrap()
        .member(member)
        .unwrap()
        .build();
    MessageStream::for_match_rule(rule, client, Some(16)).await.unwrap()
}

fn key_event(grab_id: &str, pressed: bool, time_ms: u64) -> KeyEvent {
    KeyEvent {
        grab_id: grab_id.to_owned(),
        pressed,
        time_ms,
    }
}

#[tokio::test]
async fn a_bound_shortcut_is_grabbed_and_its_key_reaches_the_app() {
    let h = harness(Arc::new(PreferredTriggers), None).await;
    let mut activated = signals(&h.client, "Activated").await;
    let mut deactivated = signals(&h.client, "Deactivated").await;

    let proxy = zbus::Proxy::new(&h.client, BUS_NAME, OBJECT_PATH, INTERFACE)
        .await
        .unwrap();
    assert_eq!(proxy.get_property::<u32>("version").await.unwrap(), 2);

    assert_eq!(create_session(&h.client, "/session/one", "org.example.App").await.0, 0);
    let reply = bind(
        &h.client,
        "/session/one",
        "/request/bind",
        vec![shortcut("ptt", "Push to talk", Some("CTRL+ALT+k"))],
    )
    .await;
    assert_eq!(reply.0, 0);
    assert_eq!(
        listed(&reply),
        [("ptt".into(), "Push to talk".into(), "Ctrl+Alt+K".into())]
    );
    let grabs = h.compositor.latest();
    assert_eq!(grabs.len(), 1);
    assert_eq!(grabs[0].key, "Ctrl+Alt+K");

    h.events.send(key_event(&grabs[0].id, true, 1234)).await.unwrap();
    let message = activated.next().await.unwrap().unwrap();
    let body = message.body();
    let (session, id, timestamp, _): (ObjectPath<'_>, String, u64, HashMap<String, OwnedValue>) =
        body.deserialize().unwrap();
    assert_eq!((session.as_str(), id.as_str(), timestamp), ("/session/one", "ptt", 1234));

    h.events.send(key_event(&grabs[0].id, false, 1300)).await.unwrap();
    let message = deactivated.next().await.unwrap().unwrap();
    let (_, id, timestamp, _): (ObjectPath<'_>, String, u64, HashMap<String, OwnedValue>) =
        message.body().deserialize().unwrap();
    assert_eq!((id.as_str(), timestamp), ("ptt", 1300));

    // The app asking again sees the same key.
    let message = call(
        &h.client,
        "ListShortcuts",
        &(
            ObjectPath::try_from("/request/list").unwrap(),
            ObjectPath::try_from("/session/one").unwrap(),
        ),
    )
    .await;
    let reply: Reply = message.body().deserialize().unwrap();
    assert_eq!(listed(&reply)[0].2, "Ctrl+Alt+K");

    // Closing the session lets go of the key.
    h.client
        .call_method(
            Some(BUS_NAME),
            "/session/one",
            Some("org.freedesktop.impl.portal.Session"),
            "Close",
            &(),
        )
        .await
        .unwrap();
    assert!(h.compositor.latest().is_empty());
    drop(h.portal);
}

#[tokio::test]
async fn a_key_the_compositor_refuses_is_reported_unbound() {
    let h = harness(Arc::new(PreferredTriggers), None).await;
    *h.compositor.refuse_key.lock().unwrap() = Some("Ctrl+F1".to_owned());
    create_session(&h.client, "/session/one", "app").await;
    let reply = bind(
        &h.client,
        "/session/one",
        "/request/bind",
        vec![
            shortcut("taken", "Taken", Some("CTRL+F1")),
            shortcut("free", "Free", Some("CTRL+F2")),
        ],
    )
    .await;
    assert_eq!(reply.0, 0);
    let shortcuts = listed(&reply);
    assert_eq!(shortcuts[0].2, "");
    assert_eq!(shortcuts[1].2, "Ctrl+F2");
}

#[tokio::test]
async fn the_frontend_can_withdraw_a_pending_bind() {
    let h = harness(Arc::new(Silent), None).await;
    create_session(&h.client, "/session/one", "app").await;

    let client = h.client.clone();
    let pending = tokio::spawn(async move {
        bind(
            &client,
            "/session/one",
            "/request/bind",
            vec![shortcut("a", "A", Some("CTRL+F1"))],
        )
        .await
    });
    // The Request object appears once the call is waiting on the user.
    let mut closed = false;
    for _ in 0..100 {
        let result = h
            .client
            .call_method(
                Some(BUS_NAME),
                "/request/bind",
                Some("org.freedesktop.impl.portal.Request"),
                "Close",
                &(),
            )
            .await;
        if result.is_ok() {
            closed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(closed, "the pending call never exported its Request");
    assert_eq!(pending.await.unwrap().0, 1, "withdrawn calls answer 'cancelled'");
    assert!(h.compositor.latest().is_empty());
}

#[tokio::test]
async fn cancelling_the_panel_binds_nothing() {
    let approver = Arc::new(Scripted::default());
    approver.answers.lock().unwrap().push_back(None);
    let h = harness(approver, None).await;
    create_session(&h.client, "/session/one", "app").await;
    let reply = bind(
        &h.client,
        "/session/one",
        "/request/bind",
        vec![shortcut("a", "A", Some("CTRL+F1"))],
    )
    .await;
    assert_eq!(reply.0, 1);
    assert!(h.compositor.sets.lock().unwrap().is_empty());
}

#[tokio::test]
async fn choices_survive_a_restart_and_are_not_asked_again() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("global-shortcuts.toml");
    {
        let approver = Arc::new(Scripted::default());
        approver.answers.lock().unwrap().push_back(Some(vec![Choice {
            id: "ptt".into(),
            trigger: Some(Trigger::from_xdg("SUPER+F9").unwrap()),
        }]));
        let h = harness(approver, Some(Store::at(path.clone()))).await;
        create_session(&h.client, "/session/one", "org.example.App").await;
        let reply = bind(
            &h.client,
            "/session/one",
            "/request/bind",
            vec![shortcut("ptt", "Push to talk", Some("CTRL+k"))],
        )
        .await;
        assert_eq!(listed(&reply)[0].2, "Super+F9", "the user's key wins over the app's hint");
    }

    let approver = Arc::new(Scripted::default());
    let h = harness(approver.clone(), Some(Store::at(path))).await;
    create_session(&h.client, "/session/two", "org.example.App").await;
    let message = call(
        &h.client,
        "ListShortcuts",
        &(
            ObjectPath::try_from("/request/list").unwrap(),
            ObjectPath::try_from("/session/two").unwrap(),
        ),
    )
    .await;
    let reply: Reply = message.body().deserialize().unwrap();
    assert_eq!(listed(&reply)[0].2, "Super+F9", "listed before the app binds again");

    let reply = bind(
        &h.client,
        "/session/two",
        "/request/bind",
        vec![shortcut("ptt", "Push to talk", Some("CTRL+k"))],
    )
    .await;
    assert_eq!(listed(&reply)[0].2, "Super+F9");
    assert!(approver.asked.lock().unwrap().is_empty(), "a known shortcut needs no question");
    assert_eq!(h.compositor.latest()[0].key, "Super+F9");
}

#[tokio::test]
async fn configuring_rebinds_and_announces_the_change() {
    let approver = Arc::new(Scripted::default());
    approver.answers.lock().unwrap().push_back(Some(vec![Choice {
        id: "ptt".into(),
        trigger: Some(Trigger::from_xdg("CTRL+F5").unwrap()),
    }]));
    approver.answers.lock().unwrap().push_back(Some(vec![Choice {
        id: "ptt".into(),
        trigger: Some(Trigger::from_xdg("CTRL+F6").unwrap()),
    }]));
    let h = harness(approver.clone(), None).await;
    let mut changed = signals(&h.client, "ShortcutsChanged").await;
    create_session(&h.client, "/session/one", "app").await;
    bind(
        &h.client,
        "/session/one",
        "/request/bind",
        vec![shortcut("ptt", "Push to talk", None)],
    )
    .await;
    assert_eq!(h.compositor.latest()[0].key, "Ctrl+F5");

    h.client
        .call_method(
            Some(BUS_NAME),
            OBJECT_PATH,
            Some(INTERFACE),
            "ConfigureShortcuts",
            &(ObjectPath::try_from("/session/one").unwrap(), "", opts()),
        )
        .await
        .unwrap();
    let message = changed.next().await.unwrap().unwrap();
    let body = message.body();
    let (session, shortcuts): (ObjectPath<'_>, Vec<(String, HashMap<String, OwnedValue>)>) =
        body.deserialize().unwrap();
    assert_eq!(session.as_str(), "/session/one");
    assert_eq!(shortcuts[0].0, "ptt");
    assert_eq!(h.compositor.latest()[0].key, "Ctrl+F6");
    let asked = approver.asked.lock().unwrap();
    assert!(asked[1].reconfigure);
    assert_eq!(asked[1].shortcuts[0].current, Some(Trigger::from_xdg("CTRL+F5").unwrap()));
}

#[tokio::test]
async fn unknown_sessions_fail_cleanly() {
    let h = harness(Arc::new(PreferredTriggers), None).await;
    let reply = bind(
        &h.client,
        "/session/missing",
        "/request/bind",
        vec![shortcut("a", "A", None)],
    )
    .await;
    assert_eq!(reply.0, 2);
    assert_eq!(create_session(&h.client, "/session/one", "app").await.0, 0);
    assert_eq!(
        create_session(&h.client, "/session/one", "app").await.0,
        2,
        "a session handle cannot be created twice"
    );
}

#[tokio::test]
async fn keys_are_lifted_while_the_user_is_typing_and_restored_after() {
    let approver = Arc::new(Scripted {
        interactive: true,
        ..Scripted::default()
    });
    approver.answers.lock().unwrap().push_back(Some(vec![Choice {
        id: "a".into(),
        trigger: Some(Trigger::from_xdg("CTRL+F1").unwrap()),
    }]));
    approver.answers.lock().unwrap().push_back(None);
    let h = harness(approver, None).await;
    create_session(&h.client, "/session/one", "app").await;
    bind(&h.client, "/session/one", "/request/one", vec![shortcut("a", "A", None)]).await;
    let sets = h.compositor.sets.lock().unwrap().clone();
    assert_eq!(sets.len(), 2, "{sets:?}");
    assert!(sets[0].is_empty(), "grabs were not lifted before the question");
    assert_eq!(sets[1][0].key, "Ctrl+F1");

    // Cancelling the panel puts back what was held before.
    h.client
        .call_method(
            Some(BUS_NAME),
            OBJECT_PATH,
            Some(INTERFACE),
            "ConfigureShortcuts",
            &(ObjectPath::try_from("/session/one").unwrap(), "", opts()),
        )
        .await
        .unwrap();
    for _ in 0..100 {
        if h.compositor.sets.lock().unwrap().len() >= 4 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let sets = h.compositor.sets.lock().unwrap().clone();
    assert_eq!(sets.len(), 4, "{sets:?}");
    assert!(sets[2].is_empty());
    assert_eq!(sets[3][0].key, "Ctrl+F1");
}

/// The same flow through the real xdg-desktop-portal frontend, which is what apps talk to. It
/// checks the frontend accepts this backend and how it names a plain host app.
///
/// `DOJO_XDG_DESKTOP_PORTAL=/path/to/libexec/xdg-desktop-portal cargo test -- --ignored real_frontend`
#[tokio::test]
#[ignore = "needs the real xdg-desktop-portal binary in DOJO_XDG_DESKTOP_PORTAL"]
async fn real_frontend_routes_an_app_to_this_backend() {
    let frontend = std::env::var("DOJO_XDG_DESKTOP_PORTAL")
        .expect("set DOJO_XDG_DESKTOP_PORTAL to the xdg-desktop-portal binary");
    let h = harness(Arc::new(PreferredTriggers), None).await;

    let home = tempfile::tempdir().unwrap();
    let portals = home.path().join("data/xdg-desktop-portal/portals");
    std::fs::create_dir_all(&portals).unwrap();
    std::fs::write(
        portals.join("dojo.portal"),
        format!("[portal]\nDBusName={BUS_NAME}\nInterfaces=org.freedesktop.impl.portal.GlobalShortcuts\nUseIn=niri\n"),
    )
    .unwrap();
    // Registering an app id needs a desktop entry of that name.
    let applications = home.path().join("data/applications");
    std::fs::create_dir_all(&applications).unwrap();
    std::fs::write(
        applications.join("org.example.Recorder.desktop"),
        "[Desktop Entry]\nType=Application\nName=Recorder\nExec=true\n",
    )
    .unwrap();
    let config = home.path().join("config/xdg-desktop-portal");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(
        config.join("niri-portals.conf"),
        "[preferred]\ndefault=none\norg.freedesktop.impl.portal.GlobalShortcuts=dojo\n",
    )
    .unwrap();
    let mut portal = Command::new(frontend)
        .args(["-r", "-v"])
        .env("DBUS_SESSION_BUS_ADDRESS", &h._bus.address)
        .env("XDG_DATA_HOME", home.path().join("data"))
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("XDG_CURRENT_DESKTOP", "niri")
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();

    const PORTAL: &str = "org.freedesktop.portal.Desktop";
    const FRONT: &str = "org.freedesktop.portal.GlobalShortcuts";
    const REQUEST: &str = "org.freedesktop.portal.Request";
    let app = h.client.clone();
    let mut responses = MessageStream::for_match_rule(
        MatchRule::builder()
            .msg_type(Type::Signal)
            .interface(REQUEST)
            .unwrap()
            .member("Response")
            .unwrap()
            .build(),
        &app,
        Some(16),
    )
    .await
    .unwrap();
    let mut activated = MessageStream::for_match_rule(
        MatchRule::builder()
            .msg_type(Type::Signal)
            .interface(FRONT)
            .unwrap()
            .member("Activated")
            .unwrap()
            .build(),
        &app,
        Some(16),
    )
    .await
    .unwrap();

    // A plain host app has no app id until it registers one; the frontend refuses it otherwise.
    for _ in 0..100 {
        let registered = app
            .call_method(
                Some(PORTAL),
                "/org/freedesktop/portal/desktop",
                Some("org.freedesktop.host.portal.Registry"),
                "Register",
                &("org.example.Recorder", opts()),
            )
            .await;
        match registered {
            Ok(_) => break,
            // The frontend takes a moment to own its name.
            Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
    let mut created = None;
    for _ in 0..100 {
        let mut options: HashMap<&str, Value<'_>> = HashMap::new();
        options.insert("handle_token", Value::from("create1"));
        options.insert("session_handle_token", Value::from("session1"));
        match app
            .call_method(Some(PORTAL), "/org/freedesktop/portal/desktop", Some(FRONT), "CreateSession", &(options,))
            .await
        {
            Ok(message) => {
                created = Some(message);
                break;
            }
            Err(error) => {
                let text = error.to_string();
                if !text.contains("ServiceUnknown") && !text.contains("NameHasNoOwner") {
                    panic!("CreateSession failed: {text}");
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
    created.expect("the frontend never appeared on the bus");

    let message = tokio::time::timeout(Duration::from_secs(5), responses.next())
        .await
        .expect("no Response to CreateSession")
        .unwrap()
        .unwrap();
    let body = message.body();
    let (code, results): (u32, HashMap<String, OwnedValue>) = body.deserialize().unwrap();
    assert_eq!(code, 0, "the frontend refused CreateSession: {results:?}");
    let session = String::try_from(results["session_handle"].try_clone().unwrap()).unwrap();

    let mut options: HashMap<&str, Value<'_>> = HashMap::new();
    options.insert("handle_token", Value::from("bind1"));
    let shortcut = shortcut("ptt", "Push to talk", Some("CTRL+ALT+k"));
    let shortcuts = vec![(shortcut.0.as_str(), {
        let mut properties: HashMap<&str, Value<'_>> = HashMap::new();
        for (key, value) in &shortcut.1 {
            properties.insert(key.as_str(), value.clone());
        }
        properties
    })];
    app.call_method(
        Some(PORTAL),
        "/org/freedesktop/portal/desktop",
        Some(FRONT),
        "BindShortcuts",
        &(ObjectPath::try_from(session.as_str()).unwrap(), shortcuts, "", options),
    )
    .await
    .unwrap();
    let message = tokio::time::timeout(Duration::from_secs(5), responses.next())
        .await
        .expect("no Response to BindShortcuts")
        .unwrap()
        .unwrap();
    let body = message.body();
    let (code, results): (u32, HashMap<String, OwnedValue>) = body.deserialize().unwrap();
    assert_eq!(code, 0, "{results:?}");
    let bound = listed(&(code, results));
    assert_eq!(bound[0].2, "Ctrl+Alt+K");

    let grabs = h.compositor.latest();
    assert_eq!(grabs.len(), 1);
    h.events.send(key_event(&grabs[0].id, true, 99)).await.unwrap();
    let message = tokio::time::timeout(Duration::from_secs(5), activated.next())
        .await
        .expect("the frontend did not forward Activated")
        .unwrap()
        .unwrap();
    let body = message.body();
    let (forwarded, id, _, _): (ObjectPath<'_>, String, u64, HashMap<String, OwnedValue>) =
        body.deserialize().unwrap();
    assert_eq!((forwarded.as_str(), id.as_str()), (session.as_str(), "ptt"));

    let _ = portal.kill();
    let _ = portal.wait();
}
