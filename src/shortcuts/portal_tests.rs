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
    answers: Mutex<VecDeque<Option<Vec<Choice>>>>,
    asked: Mutex<Vec<ChoiceRequest>>,
}

#[async_trait]
impl Approver for Scripted {
    async fn choose(&self, request: ChoiceRequest) -> Option<Vec<Choice>> {
        self.asked.lock().unwrap().push(request);
        self.answers.lock().unwrap().pop_front().flatten()
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
