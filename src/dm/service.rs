//! The display manager's event loop: one thread, `poll(2)` over the greeter socket, the
//! login helpers, the greeter's own process and a signal descriptor. Every decision is in a
//! pure core (`cycle`, `registry`, `auth`, `sessions`, `memory`); this file only reads
//! descriptors, asks the cores what to do, and does it.
//!
//! Blocking steps are bounded and happen where nobody is waiting on the loop: starting
//! the greeter, stopping it, and the snapshot child. D-Bus is never called on the loop:
//! the fingerprint probe runs on its own thread and power actions on a detached one.

use std::collections::{BTreeMap, VecDeque};
use std::ffi::c_int;
use std::io;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use super::account::{self, Account};
use super::auth::Rejected;
use super::config::{Parts, Settings};
use super::cycle::{Cycle, Input, Order, Phase};
use super::greeter::{GreeterProcess, Greeters, SESSION_GONE_TIMEOUT, wait_session_gone};
use super::host::{Fingerprints, Power, Seat};
use super::launch::{HelperEvent, SessionEvent};
use super::look::Snapshots;
use super::memory::{self, Remembered};
use super::process::{HelperProcess, Received};
use super::protocol::{self, Event, Login, PowerOccasion, Request, SessionFailure, SessionId};
use super::registry::{Action, Registry};
use super::session::Service as PamService;
use super::sessions::Catalog;
use super::vt::Console;
use crate::lock::pam::Report;
use crate::session::snapshot;

/// A greeter that has not said hello by now is not coming.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const SEND_TIMEOUT: libc::timeval = libc::timeval {
    tv_sec: 2,
    tv_usec: 0,
};

pub fn log(message: impl std::fmt::Display) {
    eprintln!("dojo-dm: {message}");
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Source {
    Signals,
    Listener,
    Lookups,
    Greeter,
    Connection,
    Helper(u32),
}

struct Greeter {
    process: Box<dyn GreeterProcess>,
    /// Until it says hello.
    deadline: Option<Instant>,
}

struct Connection {
    socket: OwnedFd,
    hello: bool,
}

/// The session the greeter started, from the helper's `Start` until it ends.
struct SessionRun {
    generation: u32,
    login: Login,
    id: SessionId,
}

struct Lookups {
    requests: Sender<(Login, Option<String>)>,
    answers: Receiver<Event>,
    wake: UnixStream,
}

impl Lookups {
    fn spawn(mut probe: Box<dyn Fingerprints>) -> io::Result<Lookups> {
        let (requests, queue) = channel::<(Login, Option<String>)>();
        let (answers_in, answers) = channel();
        let (wake, mut ring) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        std::thread::Builder::new()
            .name("fingerprint-probe".into())
            .spawn(move || {
                use std::io::Write as _;
                while let Ok((login, display_name)) = queue.recv() {
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

    fn drain_wake(&self) {
        use std::io::Read as _;
        let mut sink = [0u8; 64];
        while matches!((&self.wake).read(&mut sink), Ok(n) if n > 0) {}
    }
}

struct Service {
    settings: Settings,
    console: Box<dyn Console>,
    greeters: Box<dyn Greeters>,
    power: Arc<dyn Power>,
    seat: Box<dyn Seat>,
    snapshots: Snapshots,
    lookups: Lookups,
    cycle: Cycle,
    registry: Registry,
    catalog: Catalog,
    signals: OwnedFd,
    listener: OwnedFd,
    greeter: Option<Greeter>,
    connection: Option<Connection>,
    helpers: BTreeMap<u32, HelperProcess>,
    session: Option<SessionRun>,
    /// Why the last session did not start, when the greeter that asked for it was already gone.
    /// The next greeter is told right after its `Init`, once.
    unreported: Option<SessionFailure>,
    last_login: Option<Login>,
    epoch: Instant,
    exit: Option<i32>,
}

/// Runs until the cycle says to stop and returns the exit status.
pub fn run(settings: Settings, parts: Parts) -> i32 {
    // Signals are blocked before any thread exists, so none of them ever sees one.
    let signals = match signal_descriptor() {
        Ok(fd) => fd,
        Err(error) => return fail("signals", error),
    };
    let listener = match listen(&settings.socket, parts.greeter_group) {
        Ok(fd) => fd,
        Err(error) => return fail("greeter socket", error),
    };
    let lookups = match Lookups::spawn(parts.fingerprints) {
        Ok(lookups) => lookups,
        Err(error) => return fail("fingerprint probe", error),
    };
    let snapshots = Snapshots::new(settings.state.clone(), parts.greeter_group);
    if let Err(error) = snapshots.prepare() {
        log(format!(
            "cannot prepare {}: {error}",
            settings.state.display()
        ));
    }
    let (cycle, orders) = Cycle::start();
    let mut service = Service {
        last_login: memory::load(&settings.state).login,
        settings,
        console: parts.console,
        greeters: parts.greeters,
        power: Arc::from(parts.power),
        seat: parts.seat,
        snapshots,
        lookups,
        cycle,
        registry: Registry::new(),
        catalog: Catalog::default(),
        signals,
        listener,
        greeter: None,
        connection: None,
        helpers: BTreeMap::new(),
        session: None,
        unreported: None,
        epoch: Instant::now(),
        exit: None,
    };
    service.run_orders(orders);
    while service.exit.is_none() {
        service.turn();
    }
    let _ = std::fs::remove_file(&service.settings.socket);
    service.exit.unwrap_or(0)
}

fn fail(what: &str, error: io::Error) -> i32 {
    log(format!("{what}: {error}"));
    super::cycle::EXIT_GREETER_UNAVAILABLE
}

impl Service {
    fn now(&self) -> Duration {
        self.epoch.elapsed()
    }

    fn timeout(&self) -> c_int {
        let deadline = self.greeter.as_ref().and_then(|greeter| greeter.deadline);
        deadline.map_or(-1, |deadline| {
            let left = deadline.saturating_duration_since(Instant::now());
            (left.as_millis() as c_int).saturating_add(1)
        })
    }

    /// One pass: wait for something, then deal with all of it.
    fn turn(&mut self) {
        let mut sources = vec![
            (Source::Signals, self.signals.as_raw_fd()),
            (Source::Listener, self.listener.as_raw_fd()),
            (Source::Lookups, self.lookups.wake.as_raw_fd()),
        ];
        if let Some(greeter) = &self.greeter {
            sources.push((Source::Greeter, greeter.process.exit_fd().as_raw_fd()));
        }
        if let Some(connection) = &self.connection {
            sources.push((Source::Connection, connection.socket.as_raw_fd()));
        }
        for (generation, helper) in &self.helpers {
            sources.push((Source::Helper(*generation), helper.socket().as_raw_fd()));
        }
        let mut polls: Vec<libc::pollfd> = sources
            .iter()
            .map(|(_, fd)| libc::pollfd {
                fd: *fd,
                events: libc::POLLIN,
                revents: 0,
            })
            .collect();
        let ready = unsafe { libc::poll(polls.as_mut_ptr(), polls.len() as _, self.timeout()) };
        if ready < 0 && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
            log(format!("poll: {}", io::Error::last_os_error()));
            std::thread::sleep(Duration::from_millis(100));
        }
        if ready > 0 {
            let fired: Vec<Source> = sources
                .iter()
                .zip(&polls)
                .filter(|(_, poll)| poll.revents != 0)
                .map(|((source, _), _)| *source)
                .collect();
            for source in fired {
                if self.exit.is_some() {
                    return;
                }
                self.dispatch(source);
            }
        }
        self.check_deadline();
    }

    fn dispatch(&mut self, source: Source) {
        match source {
            Source::Signals => self.read_signal(),
            Source::Listener => self.accept(),
            Source::Lookups => self.lookups_answered(),
            Source::Greeter => {
                let over = self.greeter.as_mut().is_some_and(|g| g.process.ended());
                if over {
                    log("the greeter ended");
                    self.greeter_lost();
                }
            }
            Source::Connection => self.read_request(),
            Source::Helper(generation) => self.read_helper(generation),
        }
    }

    fn check_deadline(&mut self) {
        let late = self
            .greeter
            .as_ref()
            .and_then(|greeter| greeter.deadline)
            .is_some_and(|deadline| Instant::now() >= deadline);
        if late {
            log("the greeter did not connect in time");
            self.greeter_lost();
        }
    }

    fn apply(&mut self, input: Input) {
        let orders = self.cycle.on(input);
        self.run_orders(orders);
    }

    fn run_orders(&mut self, orders: Vec<Order>) {
        let mut queue: VecDeque<Order> = orders.into();
        while let Some(order) = queue.pop_front() {
            match order {
                Order::LaunchGreeter => {
                    if let Err(error) = self.launch_greeter() {
                        log(format!("the greeter did not start: {error}"));
                        queue.extend(self.cycle.on(Input::GreeterFailed { at: self.now() }));
                    }
                }
                Order::StopGreeter => {
                    self.stop_greeter();
                }
                Order::Snapshot => self.snapshot_current(),
                Order::Exit(code) => self.finish(code),
            }
        }
    }

    fn launch_greeter(&mut self) -> io::Result<()> {
        if let Err(error) = self.console.acquire() {
            log(format!("cannot take the console: {error}"));
        }
        let process = self.greeters.launch(self.console.vt())?;
        self.greeter = Some(Greeter {
            process,
            deadline: Some(Instant::now() + CONNECT_TIMEOUT),
        });
        Ok(())
    }

    /// Stops the greeter and its connection and waits until its logind session is gone.
    /// `false` when something of the greeter is still running afterwards.
    fn stop_greeter(&mut self) -> bool {
        self.connection = None;
        let actions = self.registry.greeter_gone();
        let mut clean = true;
        if let Some(mut greeter) = self.greeter.take() {
            if !greeter.process.stop() {
                log("some of the greeter's processes survived");
                clean = false;
            }
            drop(greeter);
            if !wait_session_gone(
                self.seat.as_ref(),
                self.greeters.peer_uid(),
                SESSION_GONE_TIMEOUT,
            ) {
                log("the greeter's session is still known to logind");
            }
        }
        self.run_actions(actions);
        clean
    }

    /// The greeter went away or never came: a failed start unless a session took over.
    fn greeter_lost(&mut self) {
        self.stop_greeter();
        if self.cycle.phase() == Phase::Greeter {
            self.apply(Input::GreeterFailed { at: self.now() });
        }
    }

    fn finish(&mut self, code: i32) {
        // A running session belongs to its own logind scope; leaving it is not ending it.
        if let Some(session) = &self.session
            && let Some(helper) = self.helpers.remove(&session.generation)
        {
            std::mem::forget(helper);
        }
        self.helpers.clear();
        self.exit = Some(code);
    }

    fn read_signal(&mut self) {
        let mut info = [0u8; 128];
        let read = unsafe { libc::read(self.signals.as_raw_fd(), info.as_mut_ptr().cast(), 128) };
        if read > 0 {
            log("stopping");
            self.apply(Input::Terminate);
        }
    }

    fn accept(&mut self) {
        loop {
            let fd = unsafe {
                libc::accept4(
                    self.listener.as_raw_fd(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
                )
            };
            if fd < 0 {
                return;
            }
            let socket = unsafe { OwnedFd::from_raw_fd(fd) };
            let from_greeter =
                protocol::peer_uid(socket.as_fd()).ok() == Some(self.greeters.peer_uid());
            let free = self.greeter.is_some() && self.connection.is_none();
            if !from_greeter || !free {
                log("refused a connection");
                continue;
            }
            unsafe {
                libc::setsockopt(
                    socket.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_SNDTIMEO,
                    (&SEND_TIMEOUT as *const libc::timeval).cast(),
                    size_of::<libc::timeval>() as libc::socklen_t,
                )
            };
            self.connection = Some(Connection {
                socket,
                hello: false,
            });
        }
    }

    fn read_request(&mut self) {
        let Some(connection) = &self.connection else {
            return;
        };
        match protocol::receive::<Request>(connection.socket.as_fd()) {
            Ok(Some(Ok(request))) => self.handle(request),
            Ok(Some(Err(error))) => log(format!("undecodable request: {error}")),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Ok(None) | Err(_) => {
                log("the greeter closed its connection");
                self.greeter_lost();
            }
        }
    }

    fn handle(&mut self, request: Request) {
        let hello = self.connection.as_ref().is_some_and(|c| c.hello);
        match request {
            Request::Hello { version } if !hello => {
                if version != protocol::VERSION {
                    log(format!("the greeter speaks protocol {version}"));
                    return self.greeter_lost();
                }
                if let Some(connection) = &mut self.connection {
                    connection.hello = true;
                }
                if let Some(greeter) = &mut self.greeter {
                    greeter.deadline = None;
                }
                self.send_init();
            }
            _ if !hello => {}
            Request::Hello { .. } => {}
            Request::SelectUser { login } => {
                let display_name = snapshot::load(self.snapshots.root(), login.as_str())
                    .map(|look| look.display_name());
                let _ = self.lookups.requests.send((login, display_name));
            }
            Request::Power { occasion } => self.power_off(occasion),
            other => {
                let cancel = matches!(other, Request::Cancel { .. });
                match self.registry.request(other) {
                    Ok(actions) => self.run_actions(actions),
                    // The attempt is over already: a loser the race killed, a refusal just relayed.
                    Err(Rejected::Stale) if cancel => {}
                    Err(rejected) => log(format!("ignored a request: {rejected:?}")),
                }
            }
        }
    }

    fn send_init(&mut self) {
        let (catalog, skipped) = Catalog::load(
            &self.settings.sessions,
            self.settings.sessions_owner,
            |path| account::is_executable_file(path),
        );
        for entry in skipped {
            log(format!(
                "session entry {} left out: {}",
                entry.file, entry.reason
            ));
        }
        self.catalog = catalog;
        let Remembered { login, session } = memory::load(&self.settings.state);
        self.send(&Event::Init {
            last_login: login,
            last_session: session.filter(|id| self.catalog.resolve(id).is_some()),
            sessions: self.catalog.entries(),
        });
        if let Some(reason) = self.unreported.take() {
            self.send(&Event::SessionFailed { reason });
        }
    }

    fn send(&self, event: &Event) {
        if let Some(connection) = &self.connection
            && let Err(error) = protocol::send(connection.socket.as_fd(), event)
        {
            log(format!("cannot reach the greeter: {error}"));
        }
    }

    fn lookups_answered(&mut self) {
        self.lookups.drain_wake();
        while let Ok(event) = self.lookups.answers.try_recv() {
            self.send(&event);
        }
    }

    fn power_off(&self, occasion: PowerOccasion) {
        let power = self.power.clone();
        let spawned = std::thread::Builder::new()
            .name("power".into())
            .spawn(move || {
                if let Err(error) = power.perform(occasion) {
                    log(format!("{occasion:?}: {error}"));
                }
            });
        if let Err(error) = spawned {
            log(format!("cannot start the power action: {error}"));
        }
    }

    fn run_actions(&mut self, actions: Vec<Action>) {
        let mut queue: VecDeque<Action> = actions.into();
        while let Some(action) = queue.pop_front() {
            match action {
                Action::Spawn {
                    generation,
                    method,
                    login,
                } => {
                    let spawned = HelperProcess::spawn(
                        &self.settings.helper,
                        PamService::Login(method),
                        &login,
                    );
                    match spawned {
                        Ok(helper) => {
                            self.helpers.insert(generation, helper);
                        }
                        Err(error) => {
                            log(format!("cannot start the PAM helper: {error}"));
                            let crashed = HelperEvent::Report(Report::Crashed);
                            queue.extend(self.registry.helper(generation, crashed));
                        }
                    }
                }
                Action::Answer { generation, secret } => {
                    if let Some(helper) = self.helpers.get(&generation)
                        && let Err(error) = helper.answer(secret.into_password())
                    {
                        log(format!("cannot answer the PAM helper: {error}"));
                    }
                }
                Action::Relay(Event::SessionFailed { reason }) if self.connection.is_none() => {
                    self.unreported = Some(reason);
                }
                Action::Relay(event) => self.send(&event),
                Action::StartSession {
                    generation,
                    session,
                } => queue.extend(self.begin_session(generation, &session)),
                Action::Kill { generation } => {
                    self.helpers.remove(&generation);
                }
            }
        }
    }

    fn read_helper(&mut self, generation: u32) {
        let Some(helper) = self.helpers.get(&generation) else {
            return;
        };
        let event = match helper.receive() {
            Received::Event(event) => event,
            Received::Closed | Received::Garbled => HelperEvent::Report(Report::Crashed),
        };
        let in_session = self
            .session
            .as_ref()
            .is_some_and(|session| session.generation == generation);
        if matches!(event, HelperEvent::Session(SessionEvent::Ended))
            && let Some(helper) = self.helpers.get_mut(&generation)
        {
            // Ended is sent just before the helper's last PAM call; let it finish.
            helper.wait_for_exit(Duration::from_secs(3));
        }
        let over = in_session
            && matches!(
                event,
                HelperEvent::Session(SessionEvent::Ended | SessionEvent::Failed(_))
                    | HelperEvent::Report(Report::Crashed)
            );
        if in_session && event == HelperEvent::Session(SessionEvent::Started) {
            self.remember_session();
        }
        let actions = self.registry.helper(generation, event);
        self.run_actions(actions);
        if self.registry.phase(generation).is_none() {
            self.helpers.remove(&generation);
        }
        if over {
            self.session = None;
            self.apply(Input::SessionOver);
        }
    }

    fn remember_session(&self) {
        let Some(session) = &self.session else { return };
        let state = &self.settings.state;
        let saved = memory::save_login(state, &session.login)
            .and_then(|()| memory::save_session(state, &session.id));
        if let Err(error) = saved {
            log(format!("cannot remember the session: {error}"));
        }
    }

    /// The greeter asked to start `id` for the attempt that won. Everything that can say
    /// no does so before the greeter goes; after that the greeter is stopped, the user's
    /// look is snapshotted, and only then does the helper start the session.
    fn begin_session(&mut self, generation: u32, id: &SessionId) -> Vec<Action> {
        let Some(login) = self
            .registry
            .winner()
            .filter(|winner| winner.generation == generation)
            .map(|winner| winner.login.clone())
        else {
            return Vec::new();
        };
        let Some(session) = self.catalog.resolve(id).cloned() else {
            return self
                .registry
                .refuse_start(generation, SessionFailure::UnknownSession);
        };
        let Some((account, gecos)) = lookup_account(&login) else {
            return self
                .registry
                .refuse_start(generation, SessionFailure::Refused);
        };
        let start = session.start(&self.console.vt());
        self.session = Some(SessionRun {
            generation,
            login: login.clone(),
            id: id.clone(),
        });
        self.last_login = Some(login);
        self.apply(Input::SessionStarting);
        self.refresh_snapshot(&account, gecos.as_deref());
        if !self.stop_greeter() {
            // The greeter user's code may still be running; the user's session, and the
            // password that unlocked it, must not start beside it.
            log("not starting the session: the greeter left processes behind");
            return self.abandon_start(generation, SessionFailure::Refused);
        }
        let sent = self
            .helpers
            .get(&generation)
            .map(|helper| helper.start(&start));
        if !matches!(sent, Some(Ok(()))) {
            log("cannot send the session to its helper");
            return self.abandon_start(generation, SessionFailure::SpawnFailed);
        }
        Vec::new()
    }

    /// The session was announced and the greeter is gone, but it will not start: a fresh
    /// greeter takes over and learns why once it connects.
    fn abandon_start(&mut self, generation: u32, reason: SessionFailure) -> Vec<Action> {
        self.session = None;
        self.apply(Input::SessionOver);
        self.registry.refuse_start(generation, reason)
    }

    fn refresh_snapshot(&self, account: &Account, gecos: Option<&str>) {
        if let Err(error) = self.snapshots.refresh(account, gecos) {
            log(format!(
                "look snapshot of {}: {error}",
                account.login.as_str()
            ));
        }
    }

    /// The user of the running session, or else the last one to log in.
    fn snapshot_current(&self) {
        let login = self
            .session
            .as_ref()
            .map(|session| &session.login)
            .or(self.last_login.as_ref());
        if let Some(login) = login
            && let Some((account, gecos)) = lookup_account(login)
        {
            self.refresh_snapshot(&account, gecos.as_deref());
        }
    }
}

fn lookup_account(login: &Login) -> Option<(Account, Option<String>)> {
    let entry = account::lookup(login)?;
    let gecos = entry.gecos.clone();
    Account::validate(
        login,
        Some(entry),
        &account::read_shells(),
        account::is_executable_file,
    )
    .ok()
    .map(|account| (account, gecos))
}

/// SIGTERM and SIGINT, blocked for the process and read from a descriptor instead.
fn signal_descriptor() -> io::Result<OwnedFd> {
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGTERM);
        libc::sigaddset(&mut set, libc::SIGINT);
        if libc::sigprocmask(libc::SIG_BLOCK, &set, std::ptr::null_mut()) < 0 {
            return Err(io::Error::last_os_error());
        }
        let fd = libc::signalfd(-1, &set, libc::SFD_CLOEXEC | libc::SFD_NONBLOCK);
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(OwnedFd::from_raw_fd(fd))
    }
}

/// The greeter socket: a `SOCK_SEQPACKET` listener only root and the greeter's group can
/// connect to. The peer's uid is checked again on every accept.
fn listen(path: &Path, group: Option<u32>) -> io::Result<OwnedFd> {
    use std::os::unix::ffi::OsStrExt as _;
    let bytes = path.as_os_str().as_bytes();
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if bytes.len() >= address.sun_path.len() || bytes.contains(&0) {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (slot, byte) in address.sun_path.iter_mut().zip(bytes) {
        *slot = *byte as libc::c_char;
    }
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let fd = unsafe {
        libc::socket(
            libc::AF_UNIX,
            libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
            0,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let socket = unsafe { OwnedFd::from_raw_fd(fd) };
    let bound = unsafe {
        libc::bind(
            socket.as_raw_fd(),
            (&address as *const libc::sockaddr_un).cast(),
            size_of::<libc::sockaddr_un>() as libc::socklen_t,
        )
    };
    if bound < 0 {
        return Err(io::Error::last_os_error());
    }
    use std::os::unix::fs::PermissionsExt as _;
    if let Some(group) = group {
        std::os::unix::fs::chown(path, None, Some(group))?;
    }
    let mode = if group.is_some() { 0o660 } else { 0o600 };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    if unsafe { libc::listen(socket.as_raw_fd(), 4) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(socket)
}
