use super::*;
use std::os::fd::{AsFd, FromRawFd, OwnedFd};

fn login(text: &str) -> Login {
    Login::parse(text).unwrap()
}

fn session(text: &str) -> SessionId {
    SessionId::parse(text).unwrap()
}

fn secret(text: &str) -> Secret {
    Secret::from_bytes(text.as_bytes()).unwrap()
}

fn requests() -> Vec<Request> {
    vec![
        Request::Hello { version: VERSION },
        Request::SelectUser {
            login: login("nathan"),
        },
        Request::Begin {
            generation: 7,
            method: Method::Password,
            login: login("alyssa.p-hacker_1"),
            session: session("niri-uwsm"),
        },
        Request::Begin {
            generation: u32::MAX,
            method: Method::Fingerprint,
            login: login("a"),
            session: session("x.y+z"),
        },
        Request::Answer {
            generation: 9,
            secret: secret("pä55 wörd😀"),
        },
        Request::Answer {
            generation: 0,
            secret: secret(""),
        },
        Request::Cancel { generation: 3 },
        Request::StartSession { generation: 3 },
        Request::Power {
            occasion: PowerOccasion::Suspend,
        },
        Request::Power {
            occasion: PowerOccasion::Hibernate,
        },
        Request::Power {
            occasion: PowerOccasion::Restart,
        },
        Request::Power {
            occasion: PowerOccasion::Shutdown,
        },
    ]
}

fn events() -> Vec<Event> {
    let mut reports = vec![
        Report::Prompt { echo: false },
        Report::Prompt { echo: true },
        Report::Notice(Notice::Hint {
            text: "Place your finger — ü".into(),
            error: true,
        }),
        Report::Notice(Notice::Hint {
            text: String::new(),
            error: false,
        }),
        Report::Notice(Notice::PasswordLocked),
        Report::Notice(Notice::PasswordPaused {
            remaining: Duration::from_secs(600),
        }),
        Report::Crashed,
    ];
    for outcome in [
        PamOutcome::Success,
        PamOutcome::Rejected,
        PamOutcome::Unavailable,
        PamOutcome::MaxTries,
        PamOutcome::Other(19),
        PamOutcome::Other(-1),
    ] {
        reports.push(Report::Finished(outcome));
    }
    let mut events: Vec<Event> = reports
        .into_iter()
        .map(|report| Event::Report {
            generation: 4,
            method: Method::Fingerprint,
            report,
        })
        .collect();
    events.extend([
        Event::Init {
            last_login: None,
            last_session: None,
            sessions: vec![],
        },
        Event::Init {
            last_login: Some(login("nathan")),
            last_session: Some(session("niri-uwsm")),
            sessions: vec![
                SessionEntry {
                    id: session("niri-uwsm"),
                    name: "Niri (UWSM)".into(),
                },
                SessionEntry {
                    id: session("plasma"),
                    name: "Plasma ünder Wayland".into(),
                },
            ],
        },
        Event::User {
            login: login("nathan"),
            display_name: Some("Nathan Duarte".into()),
            fingerprint: true,
        },
        Event::User {
            login: login("guest"),
            display_name: None,
            fingerprint: false,
        },
        Event::SessionStarted,
    ]);
    for reason in [
        SessionFailure::Refused,
        SessionFailure::SpawnFailed,
        SessionFailure::UnknownSession,
        SessionFailure::HelperCrashed,
        SessionFailure::Exited,
    ] {
        events.push(Event::SessionFailed { reason });
    }
    events
}

#[test]
fn every_request_round_trips() {
    for request in requests() {
        let datagram = request.encode();
        assert_eq!(Request::decode(&datagram), Ok(request));
    }
}

#[test]
fn every_event_round_trips() {
    for event in events() {
        let datagram = event.encode();
        assert_eq!(Event::decode(&datagram), Ok(event));
    }
}

#[test]
fn datagrams_have_fixed_bytes() {
    let bytes = |request: Request| request.encode().to_vec();
    assert_eq!(bytes(Request::Hello { version: 1 }), [b'H', 1, 0]);
    assert_eq!(bytes(Request::Cancel { generation: 7 }), [b'C', 7, 0, 0, 0]);
    assert_eq!(
        bytes(Request::StartSession { generation: 258 }),
        [b'G', 2, 1, 0, 0]
    );
    assert_eq!(
        bytes(Request::Power {
            occasion: PowerOccasion::Restart
        }),
        [b'W', 2]
    );
    assert_eq!(
        bytes(Request::SelectUser { login: login("ab") }),
        [b'U', 2, 0, b'a', b'b']
    );
    assert_eq!(
        bytes(Request::Answer {
            generation: 1,
            secret: secret("pw")
        }),
        [b'A', 1, 0, 0, 0, b'p', b'w']
    );
    assert_eq!(
        bytes(Request::Begin {
            generation: 5,
            method: Method::Password,
            login: login("a"),
            session: session("s"),
        }),
        [b'B', 5, 0, 0, 0, 1, 1, 0, b'a', 1, 0, b's']
    );
    assert_eq!(Event::SessionStarted.encode().to_vec(), [b's']);
    assert_eq!(
        Event::SessionFailed {
            reason: SessionFailure::SpawnFailed
        }
        .encode()
        .to_vec(),
        [b'f', 1]
    );
    assert_eq!(
        Event::Report {
            generation: 2,
            method: Method::Fingerprint,
            report: Report::Finished(PamOutcome::Other(19)),
        }
        .encode()
        .to_vec(),
        [b'r', 2, 0, 0, 0, 0, 4, 4, 19, 0, 0, 0]
    );
}

#[test]
fn logins_follow_the_snapshot_rule() {
    for good in [
        "a",
        "nathan",
        "a.b",
        "a-b",
        "a_b",
        "x0",
        "0x",
        "_x",
        &"a".repeat(32),
    ] {
        assert!(Login::parse(good).is_ok(), "{good}");
    }
    for bad in [
        "",
        ".hidden",
        "-flag",
        "Nathan",
        "a b",
        "a/b",
        "..",
        "a\0b",
        "ünï",
        "a\n",
        "a:b",
        &"a".repeat(33),
    ] {
        assert!(Login::parse(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn session_ids_are_file_stems_never_paths() {
    for good in ["niri-uwsm", "Plasma", "a.b+c_d", &"a".repeat(64)] {
        assert!(SessionId::parse(good).is_some(), "{good}");
    }
    for bad in [
        "",
        ".x",
        "-x",
        "a/b",
        "../x",
        "a b",
        "a\0",
        "é",
        &"a".repeat(65),
    ] {
        assert!(SessionId::parse(bad).is_none(), "{bad:?}");
    }
}

#[test]
fn invalid_fields_are_refused_with_the_field_named() {
    let with_login = |login: &str| {
        let mut datagram = vec![b'U'];
        datagram.extend_from_slice(&(login.len() as u16).to_le_bytes());
        datagram.extend_from_slice(login.as_bytes());
        datagram
    };
    assert_eq!(
        Request::decode(&with_login("Root")),
        Err(DecodeError::Invalid("login"))
    );
    assert_eq!(
        Request::decode(&with_login("../x")),
        Err(DecodeError::Invalid("login"))
    );
    assert_eq!(
        Request::decode(&with_login(&"a".repeat(33))),
        Err(DecodeError::Invalid("login"))
    );
    assert_eq!(
        Request::decode(&[b'B', 1, 0, 0, 0, 9, 1, 0, b'a', 1, 0, b's']),
        Err(DecodeError::Invalid("method"))
    );
    assert_eq!(
        Request::decode(&[b'W', 4]),
        Err(DecodeError::Invalid("occasion"))
    );
    assert_eq!(
        Request::decode(&[b'A', 0, 0, 0, 0, b'a', 0, b'b']),
        Err(DecodeError::Invalid("secret"))
    );
    assert_eq!(
        Request::decode(&[b'A', 0, 0, 0, 0, 0xff, 0xfe]),
        Err(DecodeError::Invalid("secret"))
    );
    let mut long = vec![b'A', 0, 0, 0, 0];
    long.extend(std::iter::repeat_n(b'k', Password::CAPACITY));
    assert!(Request::decode(&long).is_ok());
    long.push(b'k');
    assert_eq!(Request::decode(&long), Err(DecodeError::Invalid("secret")));
}

#[test]
fn trailing_bytes_and_unknown_tags_are_refused() {
    assert_eq!(Request::decode(&[]), Err(DecodeError::Truncated));
    assert_eq!(Request::decode(b"Z"), Err(DecodeError::UnknownTag(b'Z')));
    assert_eq!(
        Request::decode(&[b'C', 1, 0, 0, 0, 0]),
        Err(DecodeError::Trailing)
    );
    assert_eq!(Request::decode(&[b'H', 1]), Err(DecodeError::Truncated));
    assert_eq!(Event::decode(&[b's', 0]), Err(DecodeError::Trailing));
    assert_eq!(
        Event::decode(&[b'f', 9]),
        Err(DecodeError::Invalid("reason"))
    );
    assert_eq!(
        Event::decode(&[b'r', 0, 0, 0, 0, 1, 99]),
        Err(DecodeError::Invalid("report"))
    );
    assert_eq!(
        Event::decode(&[b'r', 0, 0, 0, 0, 1, 4, 0, 5, 0, 0, 0]),
        Err(DecodeError::Invalid("outcome"))
    );
    // A request tag is not an event and the reverse.
    assert!(Event::decode(&Request::Cancel { generation: 1 }.encode()).is_err());
    assert!(Request::decode(&Event::SessionStarted.encode()).is_err());
}

#[test]
fn oversized_lists_and_texts_are_cut_by_the_encoder_and_refused_by_the_decoder() {
    let entry = |index: usize| SessionEntry {
        id: session(&format!("s{index}")),
        name: "é".repeat(200),
    };
    let init = Event::Init {
        last_login: None,
        last_session: None,
        sessions: (0..40).map(entry).collect(),
    };
    let Ok(Event::Init { sessions, .. }) = Event::decode(&init.encode()) else {
        panic!("an encoded Init always decodes");
    };
    assert_eq!(sessions.len(), MAX_SESSIONS);
    assert!(sessions.iter().all(|entry| entry.name.len() <= MAX_NAME));
    assert!(init.encode().len() <= MAX_DATAGRAM);

    let hint = Event::Report {
        generation: 1,
        method: Method::Password,
        report: Report::Notice(Notice::Hint {
            text: "ü".repeat(600),
            error: false,
        }),
    };
    assert!(Event::decode(&hint.encode()).is_ok());

    let mut count = Event::Init {
        last_login: None,
        last_session: None,
        sessions: vec![],
    }
    .encode()
    .to_vec();
    *count.last_mut().unwrap() = 33;
    assert_eq!(Event::decode(&count), Err(DecodeError::Invalid("sessions")));
}

struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }
}

#[test]
fn decoding_is_total_over_random_truncated_and_mutated_input() {
    let mut random = Random(0x9e37_79b9_7f4a_7c15);
    let valid_requests: Vec<Vec<u8>> = requests().iter().map(|r| r.encode().to_vec()).collect();
    let valid_events: Vec<Vec<u8>> = events().iter().map(|e| e.encode().to_vec()).collect();

    for _ in 0..20_000 {
        let length = random.below(80);
        let noise: Vec<u8> = (0..length).map(|_| random.next() as u8).collect();
        let _ = Request::decode(&noise);
        let _ = Event::decode(&noise);
        // A tag a message really uses, then noise, reaches deeper branches.
        let mut tagged = noise.clone();
        if let Some(first) = tagged.first_mut() {
            *first = b"HUBACGWiurSsf"[random.below(13)];
        }
        let _ = Request::decode(&tagged);
        let _ = Event::decode(&tagged);
    }

    for (valid, is_request) in valid_requests
        .iter()
        .map(|d| (d, true))
        .chain(valid_events.iter().map(|d| (d, false)))
    {
        for end in 0..valid.len() {
            let prefix = &valid[..end];
            let refused = if is_request {
                Request::decode(prefix).is_err()
            } else {
                Event::decode(prefix).is_err()
            };
            let is_answer = is_request && valid[0] == b'A';
            // An Answer's secret has no length field, so a prefix of it is another answer.
            assert!(refused || (is_answer && end >= 5), "{prefix:?}");
        }
        for _ in 0..200 {
            let mut mutated = valid.clone();
            for _ in 0..1 + random.below(3) {
                let index = random.below(mutated.len());
                mutated[index] = random.next() as u8;
            }
            if random.below(4) == 0 {
                mutated.truncate(random.below(mutated.len() + 1));
            }
            if random.below(4) == 0 {
                mutated.extend((0..random.below(8)).map(|_| random.next() as u8));
            }
            let _ = Request::decode(&mutated);
            let _ = Event::decode(&mutated);
        }
    }
}

#[test]
fn the_secret_never_prints() {
    let request = Request::Answer {
        generation: 1,
        secret: secret("hunter2"),
    };
    let shown = format!("{request:?} {request:#?}");
    assert!(!shown.contains("hunter2"));
    assert!(shown.contains("Secret(<redacted>)"));
}

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

#[test]
fn messages_cross_a_seqpacket_socket_and_eof_ends_the_stream() {
    let (greeter, dm) = pair();
    for request in requests() {
        send(greeter.as_fd(), &request).unwrap();
        assert_eq!(receive::<Request>(dm.as_fd()).unwrap(), Some(Ok(request)));
    }
    for event in events() {
        send(dm.as_fd(), &event).unwrap();
        assert_eq!(receive::<Event>(greeter.as_fd()).unwrap(), Some(Ok(event)));
    }
    wire::send(greeter.as_fd(), b"Zjunk").unwrap();
    assert_eq!(
        receive::<Request>(dm.as_fd()).unwrap(),
        Some(Err(DecodeError::UnknownTag(b'Z')))
    );
    send(greeter.as_fd(), &Request::Cancel { generation: 1 }).unwrap();
    assert!(receive::<Request>(dm.as_fd()).unwrap().is_some());
    wire::send(greeter.as_fd(), &vec![b'A'; MAX_DATAGRAM + 1]).unwrap();
    assert_eq!(
        receive::<Request>(dm.as_fd()).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    drop(greeter);
    assert_eq!(receive::<Request>(dm.as_fd()).unwrap(), None);
}

#[test]
fn a_maximum_password_answer_fits_one_datagram() {
    let (greeter, dm) = pair();
    let mut password = Password::new();
    while password.push('k') {}
    let request = Request::Answer {
        generation: 2,
        secret: Secret::new(password),
    };
    send(greeter.as_fd(), &request).unwrap();
    assert_eq!(receive::<Request>(dm.as_fd()).unwrap(), Some(Ok(request)));
}

#[test]
fn the_peer_uid_is_the_process_on_the_other_end() {
    let (a, b) = pair();
    let own = unsafe { libc::geteuid() };
    assert_eq!(peer_uid(a.as_fd()).unwrap(), own);
    assert_eq!(peer_uid(b.as_fd()).unwrap(), own);
}
