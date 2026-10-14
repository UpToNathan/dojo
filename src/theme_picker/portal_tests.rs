use super::*;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Bus {
    child: Child,
    address: String,
    directory: PathBuf,
}
impl Bus {
    fn start() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let directory = std::env::temp_dir().join(format!(
            "dojo-portal-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir(&directory).unwrap();
        let config = directory.join("bus.conf");
        // Self-contained config: Nix's daemon need not have /etc/dbus-1/session.conf.
        std::fs::write(
            &config,
            br#"<busconfig>
          <type>session</type><listen>unix:tmpdir=/tmp</listen>
          <auth>EXTERNAL</auth><policy context="default">
            <allow send_destination="*"/><allow receive_sender="*"/>
            <allow own="*"/>
          </policy>
        </busconfig>"#,
        )
        .unwrap();
        let mut child = Command::new("dbus-daemon")
            .arg(format!("--config-file={}", config.display()))
            .args(["--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .spawn()
            .expect("private dbus-daemon required");
        let mut address = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        Self {
            child,
            address: address.trim().into(),
            directory,
        }
    }
}
impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

#[derive(Clone)]
struct Scenario {
    code: u32,
    uris: Option<Vec<String>>,
    method_error: bool,
    mismatch: bool,
    quiet: bool,
    delay: Duration,
    hang_close: bool,
    malformed_body: bool,
    wrong_uris_type: bool,
    late_create: bool,
}
impl Default for Scenario {
    fn default() -> Self {
        Self {
            code: 0,
            uris: Some(vec!["file:///tmp/a%20b.png".into()]),
            method_error: false,
            mismatch: false,
            quiet: false,
            delay: Duration::ZERO,
            hang_close: false,
            malformed_body: false,
            wrong_uris_type: false,
            late_create: false,
        }
    }
}

struct FakeRequest {
    closes: Arc<AtomicUsize>,
    hang: bool,
}
#[zbus::interface(name = "org.freedesktop.portal.Request")]
impl FakeRequest {
    async fn close(&self) {
        self.closes.fetch_add(1, Ordering::SeqCst);
        if self.hang {
            std::future::pending::<()>().await;
        }
    }
}

struct FakeChooser {
    runtime: tokio::runtime::Handle,
    scenario: Scenario,
    closes: Arc<AtomicUsize>,
    opened: async_channel::Sender<()>,
}
#[zbus::interface(name = "org.freedesktop.portal.FileChooser")]
impl FakeChooser {
    async fn open_file(
        &self,
        parent: &str,
        title: &str,
        options: Results,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        assert_eq!(parent, "");
        assert_eq!(title, "Choose wallpaper");
        assert!(!bool::try_from(options["multiple"].try_clone().unwrap()).unwrap());
        assert!(!bool::try_from(options["directory"].try_clone().unwrap()).unwrap());
        let filters =
            Vec::<(String, Vec<(u32, String)>)>::try_from(options["filters"].try_clone().unwrap())
                .unwrap();
        assert_eq!(
            filters,
            vec![(
                "Images".into(),
                vec![
                    (1, "image/jpeg".into()),
                    (1, "image/png".into()),
                    (1, "image/webp".into()),
                ]
            )]
        );
        let token = String::try_from(options["handle_token"].try_clone().unwrap()).unwrap();
        assert!(token.starts_with("dojo_wallpaper_"));
        let sender = header
            .sender()
            .unwrap()
            .as_str()
            .trim_start_matches(':')
            .replace('.', "_");
        let predicted = format!("{DESKTOP}/request/{sender}/{token}");
        let actual = if self.scenario.mismatch {
            format!("{predicted}_other")
        } else {
            predicted.clone()
        };
        if self.scenario.late_create {
            self.opened.send(()).await.unwrap();
            let delay = self.scenario.delay;
            self.runtime
                .spawn(async move { tokio::time::sleep(delay).await })
                .await
                .unwrap();
        }
        // Install request before delaying the method reply so Close can arrive
        // concurrently with a pending OpenFile.
        for path in [&predicted, &actual] {
            connection
                .object_server()
                .at(
                    path.as_str(),
                    FakeRequest {
                        closes: self.closes.clone(),
                        hang: self.scenario.hang_close,
                    },
                )
                .await
                .unwrap();
        }
        if !self.scenario.late_create {
            self.opened.send(()).await.unwrap();
        }
        if self.scenario.method_error {
            return Err(zbus::fdo::Error::Failed("fake failure".into()));
        }
        if !self.scenario.quiet {
            let mut results = Results::new();
            if let Some(uris) = &self.scenario.uris {
                results.insert(
                    "uris".into(),
                    OwnedValue::try_from(Value::from(uris.clone())).unwrap(),
                );
            }
            if self.scenario.wrong_uris_type {
                results.insert("uris".into(), OwnedValue::from(true));
            }
            // Deliberately emit BEFORE returning OpenFile's handle.
            if self.scenario.malformed_body {
                connection
                    .emit_signal(
                        None::<&str>,
                        actual.as_str(),
                        REQUEST,
                        "Response",
                        &("not a response",),
                    )
                    .await
                    .unwrap();
            } else {
                connection
                    .emit_signal(
                        None::<&str>,
                        actual.as_str(),
                        REQUEST,
                        "Response",
                        &(self.scenario.code, results),
                    )
                    .await
                    .unwrap();
            }
        }
        let delay = if self.scenario.late_create {
            Duration::ZERO
        } else {
            self.scenario.delay
        };
        // zbus's default async-io executor runs interface methods outside Tokio.
        self.runtime
            .spawn(async move { tokio::time::sleep(delay).await })
            .await
            .unwrap();
        Ok(OwnedObjectPath::try_from(actual).unwrap())
    }
}

struct Fixture {
    _bus: Bus,
    _server: Connection,
    closes: Arc<AtomicUsize>,
    opened: async_channel::Receiver<()>,
}
impl Fixture {
    async fn new(scenario: Scenario) -> Self {
        let bus = Bus::start();
        let closes = Arc::new(AtomicUsize::new(0));
        let (opened, received) = async_channel::bounded(1);
        let server = zbus::connection::Builder::address(bus.address.as_str())
            .unwrap()
            .name(DESTINATION)
            .unwrap()
            .serve_at(
                DESKTOP,
                FakeChooser {
                    runtime: tokio::runtime::Handle::current(),
                    scenario,
                    closes: closes.clone(),
                    opened,
                },
            )
            .unwrap()
            .build()
            .await
            .unwrap();
        Self {
            _bus: bus,
            _server: server,
            closes,
            opened: received,
        }
    }
    fn start(&self) -> (Request, async_channel::Receiver<Outcome>) {
        Request::start_with(Settings {
            address: Some(self._bus.address.clone()),
            method: Duration::from_millis(500),
            response: Duration::from_millis(150),
            close: Duration::from_millis(80),
        })
        .unwrap()
    }
    async fn opened(&self) {
        tokio::time::timeout(Duration::from_secs(2), self.opened.recv())
            .await
            .unwrap()
            .unwrap();
    }
}

async fn receive(receiver: &async_channel::Receiver<Outcome>) -> Outcome {
    let result = tokio::time::timeout(Duration::from_secs(3), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    // Worker must also exit and close the result channel (not just publish).
    assert!(
        tokio::time::timeout(Duration::from_secs(2), receiver.recv())
            .await
            .unwrap()
            .is_err()
    );
    result
}

#[test]
fn strict_local_uris() {
    for (uri, bytes) in [
        ("file:///tmp/a%20b.png", b"/tmp/a b.png".as_slice()),
        ("file://localhost/tmp/a.png", b"/tmp/a.png".as_slice()),
        ("file:///tmp/%FF%23%3F", b"/tmp/\xff#?".as_slice()),
    ] {
        let path = local_file_uri(uri).unwrap();
        assert_eq!(path.into_os_string().into_vec(), bytes);
    }
    for uri in [
        "https:///tmp/a",
        "file:/tmp/a",
        "file://remote/tmp/a",
        "file://localhost:80/a",
        "file://user@localhost/a",
        "file://LOCALHOST/a",
        "file://",
        "file://localhost",
        "file:///a?x",
        "file:///a#x",
        "file:///a%",
        "file:///a%2",
        "file:///a%GG",
        "file:///a%00",
        "file:///a\0",
        "file:///a b",
        "file:///é",
        "file:///a\\b",
        "file://%6cocalhost/a",
        "file:///a\n",
    ] {
        assert!(local_file_uri(uri).is_err(), "accepted {uri:?}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn early_response_is_received_and_completed_request_is_not_closed() {
    let fixture = Fixture::new(Scenario {
        delay: Duration::from_millis(60),
        ..Default::default()
    })
    .await;
    let (guard, receiver) = fixture.start();
    assert_eq!(
        receive(&receiver).await.unwrap(),
        Some(PathBuf::from("/tmp/a b.png"))
    );
    drop(guard);
    assert_eq!(fixture.closes.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn response_codes_and_invalid_results() {
    for (scenario, cancelled) in [
        (
            Scenario {
                code: 1,
                ..Default::default()
            },
            true,
        ),
        (
            Scenario {
                code: 2,
                ..Default::default()
            },
            false,
        ),
        (
            Scenario {
                code: 99,
                ..Default::default()
            },
            false,
        ),
        (
            Scenario {
                uris: None,
                ..Default::default()
            },
            false,
        ),
        (
            Scenario {
                malformed_body: true,
                ..Default::default()
            },
            false,
        ),
        (
            Scenario {
                wrong_uris_type: true,
                ..Default::default()
            },
            false,
        ),
        (
            Scenario {
                uris: Some(vec![]),
                ..Default::default()
            },
            false,
        ),
        (
            Scenario {
                uris: Some(vec!["file:///a".into(), "file:///b".into()]),
                ..Default::default()
            },
            false,
        ),
        (
            Scenario {
                uris: Some(vec!["file://remote/a".into()]),
                ..Default::default()
            },
            false,
        ),
    ] {
        let fixture = Fixture::new(scenario).await;
        let (_guard, receiver) = fixture.start();
        let outcome = receive(&receiver).await;
        if cancelled {
            assert_eq!(outcome.unwrap(), None);
        } else {
            assert!(outcome.is_err());
        }
        assert_eq!(fixture.closes.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn drop_closes_pending_open_and_late_reply() {
    for (mismatch, late_create, expected_closes) in
        [(false, false, 2), (true, false, 2), (false, true, 1)]
    {
        let fixture = Fixture::new(Scenario {
            quiet: true,
            delay: Duration::from_millis(200),
            mismatch,
            late_create,
            ..Default::default()
        })
        .await;
        let (guard, receiver) = fixture.start();
        fixture.opened().await;
        drop(guard);
        assert_eq!(receive(&receiver).await.unwrap(), None);
        assert_eq!(fixture.closes.load(Ordering::SeqCst), expected_closes);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn drop_while_waiting_for_response_closes_once() {
    let fixture = Fixture::new(Scenario {
        quiet: true,
        ..Default::default()
    })
    .await;
    let (guard, receiver) = fixture.start();
    fixture.opened().await;
    tokio::time::sleep(Duration::from_millis(40)).await;
    drop(guard);
    assert_eq!(receive(&receiver).await.unwrap(), None);
    assert_eq!(fixture.closes.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn errors_and_timeouts_close_requests_and_exit() {
    for (scenario, expected_closes) in [
        (
            Scenario {
                method_error: true,
                ..Default::default()
            },
            1,
        ),
        (
            Scenario {
                mismatch: true,
                ..Default::default()
            },
            2,
        ),
        (
            Scenario {
                quiet: true,
                ..Default::default()
            },
            1,
        ),
        (
            Scenario {
                quiet: true,
                delay: Duration::from_secs(5),
                ..Default::default()
            },
            1,
        ),
        (
            Scenario {
                quiet: true,
                hang_close: true,
                ..Default::default()
            },
            1,
        ),
    ] {
        let fixture = Fixture::new(scenario).await;
        let (_guard, receiver) = fixture.start();
        assert!(receive(&receiver).await.is_err());
        assert_eq!(fixture.closes.load(Ordering::SeqCst), expected_closes);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_stays_bounded_when_open_and_close_hang() {
    let fixture = Fixture::new(Scenario {
        quiet: true,
        hang_close: true,
        delay: Duration::from_secs(5),
        ..Default::default()
    })
    .await;
    let (guard, receiver) = fixture.start();
    fixture.opened().await;
    drop(guard);
    assert_eq!(receive(&receiver).await.unwrap(), None);
    assert_eq!(fixture.closes.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stalled_bus_setup_is_timed_out_or_cancelled() {
    let fixture = Fixture::new(Scenario::default()).await;
    let socket = fixture._bus.directory.join("silent-bus");
    // Accept no authentication messages: connection setup must remain bounded.
    let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    for cancel in [false, true] {
        let (guard, receiver) = Request::start_with(Settings {
            address: Some(format!("unix:path={}", socket.display())),
            method: Duration::from_millis(100),
            ..Default::default()
        })
        .unwrap();
        if cancel {
            drop(guard);
            assert_eq!(receive(&receiver).await.unwrap(), None);
        } else {
            assert!(receive(&receiver).await.is_err());
            drop(guard);
        }
    }
    assert_eq!(fixture.closes.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connection_failure_is_reported() {
    let (_guard, receiver) = Request::start_with(Settings {
        address: Some("unix:path=/nonexistent/dojo-portal-test-bus".into()),
        ..Default::default()
    })
    .unwrap();
    assert!(receive(&receiver).await.is_err());
}
