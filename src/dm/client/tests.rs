use std::os::fd::AsFd;
use std::thread::JoinHandle;

use super::*;
use crate::dm::protocol::{self, Event, Message, Request};
use crate::lock::pam::{Method, PamOutcome, Report};
use crate::lock::secret::Password;
use crate::lock::worker::wire;

fn pair() -> (OwnedFd, OwnedFd) {
    let mut fds = [0; 2];
    let rc = unsafe {
        libc::socketpair(
            libc::AF_UNIX,
            libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC,
            0,
            fds.as_mut_ptr(),
        )
    };
    assert_eq!(rc, 0);
    unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) }
}

fn login(text: &str) -> Login {
    Login::parse(text).unwrap()
}

fn init() -> Event {
    Event::Init {
        last_login: Some(login("nathan")),
        last_session: Some(SessionId::parse("niri-uwsm").unwrap()),
        sessions: vec![SessionEntry {
            id: SessionId::parse("niri-uwsm").unwrap(),
            name: "Niri (UWSM)".into(),
        }],
    }
}

/// A display manager that speaks the real protocol on its end of the socket.
struct FakeDm(OwnedFd);

impl FakeDm {
    fn request(&self) -> Request {
        protocol::receive::<Request>(self.0.as_fd())
            .expect("receive")
            .expect("a request, not end of stream")
            .expect("a request that decodes")
    }

    fn say(&self, event: &Event) {
        protocol::send(self.0.as_fd(), event).expect("send");
    }

    fn garble(&self, bytes: &[u8]) {
        wire::send(self.0.as_fd(), bytes).expect("send");
    }
}

fn connected() -> (Client, FakeDm) {
    let (greeter, dm) = pair();
    (Client::over(greeter), FakeDm(dm))
}

fn on_thread(work: impl FnOnce() + Send + 'static) -> JoinHandle<()> {
    std::thread::spawn(work)
}

#[test]
fn the_handshake_says_hello_and_returns_the_display_managers_init() {
    let (client, dm) = connected();
    let server = on_thread(move || {
        assert_eq!(dm.request(), Request::Hello { version: VERSION });
        dm.say(&init());
    });
    let init = client.handshake(Duration::from_secs(5)).expect("init");
    server.join().unwrap();
    assert_eq!(init.last_login, Some(login("nathan")));
    assert_eq!(init.last_session.unwrap().as_str(), "niri-uwsm");
    assert_eq!(init.sessions.len(), 1);
    assert_eq!(init.sessions[0].name, "Niri (UWSM)");
}

#[test]
fn a_display_manager_that_does_not_answer_times_out() {
    let (client, dm) = connected();
    let started = std::time::Instant::now();
    let error = client.handshake(Duration::from_millis(100)).unwrap_err();
    assert!(matches!(error, HandshakeError::TimedOut), "{error}");
    assert!(started.elapsed() < Duration::from_secs(4));
    // The hello went out, and the socket works again without the deadline.
    assert_eq!(dm.request(), Request::Hello { version: VERSION });
}

#[test]
fn a_closed_socket_is_not_an_init() {
    let (client, dm) = connected();
    drop(dm);
    // The hello has nobody to read it; the closed socket is what the greeter learns.
    let error = client.handshake(Duration::from_secs(5)).unwrap_err();
    assert!(
        matches!(error, HandshakeError::Closed | HandshakeError::Io(_)),
        "{error}"
    );
}

#[test]
fn anything_but_init_first_is_refused() {
    let (client, dm) = connected();
    let server = on_thread(move || {
        dm.request();
        dm.say(&Event::SessionStarted);
    });
    let error = client.handshake(Duration::from_secs(5)).unwrap_err();
    server.join().unwrap();
    assert!(
        matches!(error, HandshakeError::Unexpected(Event::SessionStarted)),
        "{error}"
    );
}

#[test]
fn a_datagram_that_does_not_decode_fails_the_handshake() {
    let (client, dm) = connected();
    let server = on_thread(move || {
        dm.request();
        dm.garble(b"?");
    });
    let error = client.handshake(Duration::from_secs(5)).unwrap_err();
    server.join().unwrap();
    assert!(
        matches!(error, HandshakeError::Malformed(DecodeError::UnknownTag(b'?'))),
        "{error}"
    );
}

fn secret(text: &str) -> Password {
    let mut password = Password::new();
    for c in text.chars() {
        assert!(password.push(c));
    }
    password
}

#[test]
fn a_login_over_the_socket_runs_request_by_request_and_the_reader_ends_with_the_socket() {
    let (client, dm) = connected();
    let (events, received) = async_channel::unbounded::<Incoming>();
    client.read_in_background(events, |incoming| incoming).unwrap();

    let server = on_thread(move || {
        let Request::Begin { generation, .. } = dm.request() else {
            panic!("a conversation begins first");
        };
        dm.say(&Event::Report {
            generation,
            method: Method::Password,
            report: Report::Prompt { echo: false },
        });
        let Request::Answer { secret, .. } = dm.request() else {
            panic!("the password is answered next");
        };
        assert_eq!(secret.into_password().expose(), b"hunter2");
        dm.say(&Event::Report {
            generation,
            method: Method::Password,
            report: Report::Finished(PamOutcome::Success),
        });
        assert_eq!(dm.request(), Request::StartSession { generation });
        dm.say(&Event::SessionStarted);
        // The socket closes as the display manager drops it.
    });

    client
        .send(&Request::Begin {
            generation: 3,
            method: Method::Password,
            login: login("nathan"),
            session: SessionId::parse("niri-uwsm").unwrap(),
        })
        .unwrap();
    let next = || futures_lite::future::block_on(received.recv()).unwrap();
    assert!(matches!(
        next(),
        Incoming::Event(Event::Report {
            report: Report::Prompt { echo: false },
            ..
        })
    ));
    client
        .send(&Request::Answer {
            generation: 3,
            secret: protocol::Secret::new(secret("hunter2")),
        })
        .unwrap();
    assert!(matches!(
        next(),
        Incoming::Event(Event::Report {
            report: Report::Finished(PamOutcome::Success),
            ..
        })
    ));
    client
        .send(&Request::StartSession { generation: 3 })
        .unwrap();
    assert!(matches!(next(), Incoming::Event(Event::SessionStarted)));
    server.join().unwrap();
    assert!(matches!(next(), Incoming::Closed(None)));
    // Nothing follows the end.
    assert!(futures_lite::future::block_on(received.recv()).is_err());
}

#[test]
fn a_malformed_datagram_is_reported_and_the_stream_goes_on() {
    let (client, dm) = connected();
    let (events, received) = async_channel::unbounded::<Incoming>();
    client.read_in_background(events, |incoming| incoming).unwrap();
    dm.garble(b"\xff");
    dm.say(&Event::SessionStarted);
    let next = || futures_lite::future::block_on(received.recv()).unwrap();
    assert!(matches!(
        next(),
        Incoming::Malformed(DecodeError::UnknownTag(0xff))
    ));
    assert!(matches!(next(), Incoming::Event(Event::SessionStarted)));
}

#[test]
fn sending_to_a_display_manager_that_has_gone_fails() {
    let (client, dm) = connected();
    drop(dm);
    let sent = client.send(&Request::Cancel { generation: 1 });
    assert!(sent.is_err());
}

#[test]
fn connecting_to_a_path_nobody_serves_fails_and_an_overlong_path_is_refused() {
    let nobody = std::env::temp_dir().join("dojo-greeter-no-such-socket");
    assert!(Client::connect(&nobody).is_err());
    let long = Path::new("/").join("x".repeat(200));
    let error = Client::connect(&long).err().expect("an error");
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
}

#[test]
fn a_listening_socket_is_reached_by_path() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("greeter.sock");
    let listener = {
        let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC, 0) };
        assert!(fd >= 0);
        let socket = unsafe { OwnedFd::from_raw_fd(fd) };
        let address = unix_address(&path).unwrap();
        let rc = unsafe {
            libc::bind(
                socket.as_raw_fd(),
                (&address as *const libc::sockaddr_un).cast(),
                size_of::<libc::sockaddr_un>() as libc::socklen_t,
            )
        };
        assert_eq!(rc, 0);
        assert_eq!(unsafe { libc::listen(socket.as_raw_fd(), 1) }, 0);
        socket
    };
    let me = unsafe { libc::geteuid() };
    let refused = Client::connect_to(&path, me + 1).err().expect("another uid is refused");
    assert_eq!(refused.kind(), io::ErrorKind::PermissionDenied);
    let accept = || {
        let accepted = unsafe {
            libc::accept4(
                listener.as_raw_fd(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                libc::SOCK_CLOEXEC,
            )
        };
        assert!(accepted >= 0);
        FakeDm(unsafe { OwnedFd::from_raw_fd(accepted) })
    };
    // The refused attempt left a connection in the queue.
    drop(accept());
    let client = Client::connect_to(&path, me).expect("connect");
    let dm = accept();
    let server = on_thread(move || {
        assert_eq!(dm.request(), Request::Hello { version: VERSION });
        dm.say(&init());
    });
    client.handshake(Duration::from_secs(5)).expect("init");
    server.join().unwrap();
}

#[test]
fn the_encoded_hello_is_what_the_protocol_says() {
    // The wire bytes are the protocol's, not the client's.
    assert_eq!(
        &**Request::Hello { version: VERSION }.encode(),
        [b'H', 1, 0]
    );
}
