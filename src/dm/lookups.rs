//! Answers to `SelectUser`: the display name from the user's look snapshot and whether
//! fprintd knows a finger of theirs. Both are slow or can block (a file read, a D-Bus call
//! with a timeout), so they run on one thread of their own and the service loop never waits
//! for them.
//!
//! The greeter only ever cares about the user it shows now, so requests are not queued:
//! a request replaces the one still waiting. However fast a greeter asks, the thread holds
//! one pending login and the loop holds nothing.

use std::io::{self, Read as _, Write as _};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Condvar, Mutex};

use super::host::Fingerprints;
use super::protocol::{Event, Login};
use crate::session::snapshot;

/// A slot that keeps the latest value posted to it.
struct Mailbox<T> {
    state: Mutex<(Option<T>, bool)>,
    posted: Condvar,
}

impl<T> Mailbox<T> {
    fn new() -> Mailbox<T> {
        Mailbox {
            state: Mutex::new((None, false)),
            posted: Condvar::new(),
        }
    }

    fn post(&self, value: T) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.0 = Some(value);
        self.posted.notify_one();
    }

    fn close(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.1 = true;
        self.posted.notify_one();
    }

    /// Blocks until a value is posted. `None` once the mailbox is closed.
    fn take(&self) -> Option<T> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if state.1 {
                return None;
            }
            if let Some(value) = state.0.take() {
                return Some(value);
            }
            state = self.posted.wait(state).unwrap_or_else(|e| e.into_inner());
        }
    }
}

pub struct Lookups {
    requests: Arc<Mailbox<Login>>,
    answers: Receiver<Event>,
    wake: UnixStream,
}

impl Lookups {
    /// `snapshots` is the look snapshot root the display names are read from.
    pub fn spawn(mut probe: Box<dyn Fingerprints>, snapshots: PathBuf) -> io::Result<Lookups> {
        let requests: Arc<Mailbox<Login>> = Arc::new(Mailbox::new());
        let (answers_in, answers) = channel();
        let (wake, mut ring) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        let inbox = requests.clone();
        std::thread::Builder::new()
            .name("user-lookup".into())
            .spawn(move || {
                while let Some(login) = inbox.take() {
                    let display_name = snapshot::load(&snapshots, login.as_str())
                        .map(|look| look.display_name());
                    let fingerprint = probe.enrolled(&login);
                    let event = Event::User {
                        login,
                        display_name,
                        fingerprint,
                    };
                    if answers_in.send(event).is_err() || ring.write_all(&[1]).is_err() {
                        return;
                    }
                }
            })?;
        Ok(Lookups {
            requests,
            answers,
            wake,
        })
    }

    /// Ask about `login`, replacing a request that has not been started yet.
    pub fn select(&self, login: Login) {
        self.requests.post(login);
    }

    /// Readable when [`answered`](Self::answered) has something.
    pub fn wake_fd(&self) -> RawFd {
        self.wake.as_raw_fd()
    }

    pub fn answered(&self) -> Vec<Event> {
        let mut sink = [0u8; 64];
        while matches!((&self.wake).read(&mut sink), Ok(n) if n > 0) {}
        self.answers.try_iter().collect()
    }
}

impl Drop for Lookups {
    fn drop(&mut self) {
        self.requests.close();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn a_mailbox_keeps_only_the_latest_post() {
        let mailbox = Mailbox::new();
        for n in 0..1000 {
            mailbox.post(n);
        }
        assert_eq!(mailbox.take(), Some(999));
        mailbox.close();
        assert_eq!(mailbox.take(), None);
    }

    struct Slow {
        asked: Arc<AtomicUsize>,
        gate: std::sync::mpsc::Receiver<()>,
    }

    impl Fingerprints for Slow {
        fn enrolled(&mut self, login: &Login) -> bool {
            self.asked.fetch_add(1, Ordering::SeqCst);
            let _ = self.gate.recv();
            login.as_str() == "bob"
        }
    }

    #[test]
    fn a_flood_of_selections_costs_one_lookup_for_the_first_and_one_for_the_last() {
        let asked = Arc::new(AtomicUsize::new(0));
        let (open, gate) = std::sync::mpsc::channel();
        let root = tempfile::tempdir().unwrap();
        let lookups = Lookups::spawn(
            Box::new(Slow {
                asked: asked.clone(),
                gate,
            }),
            root.path().to_owned(),
        )
        .unwrap();
        let login = |name: &str| Login::parse(name).unwrap();
        lookups.select(login("first"));
        let deadline = Instant::now() + Duration::from_secs(5);
        while asked.load(Ordering::SeqCst) == 0 {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        // The thread is stuck inside the first lookup, as it is when fprintd does not answer.
        for n in 0..10_000 {
            lookups.select(login(&format!("user{n}")));
        }
        lookups.select(login("bob"));
        open.send(()).unwrap();
        open.send(()).unwrap();
        let mut events = Vec::new();
        while events.len() < 2 {
            assert!(Instant::now() < deadline, "{events:?}");
            std::thread::sleep(Duration::from_millis(2));
            events.extend(lookups.answered());
        }
        std::thread::sleep(Duration::from_millis(50));
        events.extend(lookups.answered());
        let names: Vec<_> = events
            .iter()
            .map(|event| match event {
                Event::User { login, .. } => login.as_str().to_owned(),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(names, ["first", "bob"]);
        assert_eq!(asked.load(Ordering::SeqCst), 2);
    }
}
