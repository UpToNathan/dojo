//! Socket feasibility adapters, not production browser/accessibility transports.
//! Handshake/authentication and identity attestation must occur outside the actor;
//! this module proves the final single syscall and correlated reader seam.
use crate::*;
use anyhow::{Context, Result, bail, ensure};
use std::{
    io::{Read, Write},
    net::{Shutdown, TcpStream},
    num::NonZeroU32,
    os::{fd::AsRawFd, unix::net::UnixStream},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WriteReport {
    pub requested: usize,
    pub accepted: usize,
    pub attempt: CommitAttempt,
}
fn write_once(fd: i32, bytes: &[u8]) -> WriteReport {
    // One nonblocking syscall, no loop, no library/TLS buffer, no flush later.
    // bytes is valid for its full length; the descriptor stays alive in its owner.
    let count = unsafe {
        libc::send(
            fd,
            bytes.as_ptr().cast(),
            bytes.len(),
            libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL,
        )
    };
    let (accepted, attempt) = if count >= 0 {
        let count = count as usize;
        (
            count,
            if count == bytes.len() {
                CommitAttempt::Accepted
            } else if count == 0 {
                CommitAttempt::Disconnected
            } else {
                CommitAttempt::Partial
            },
        )
    } else {
        let error = std::io::Error::last_os_error();
        (
            0,
            if matches!(
                error.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
            ) {
                CommitAttempt::WouldBlock
            } else {
                CommitAttempt::Disconnected
            },
        )
    };
    WriteReport {
        requested: bytes.len(),
        accepted,
        attempt,
    }
}
fn wait_readable(fd: i32, deadline: std::time::Instant) -> std::io::Result<()> {
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "fixture reply deadline",
            ));
        }
        let mut event = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut event, 1, remaining.as_millis().clamp(1, 20) as i32) };
        if ready < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        if event.revents & (libc::POLLIN | libc::POLLHUP) != 0 {
            return Ok(());
        }
        if event.revents & (libc::POLLERR | libc::POLLNVAL) != 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionAborted,
                "fixture reply socket failed",
            ));
        }
    }
}
struct DeadlineReader<'a> {
    socket: &'a mut UnixStream,
    deadline: std::time::Instant,
}
impl Read for DeadlineReader<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        loop {
            match self.socket.read(bytes) {
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    wait_readable(self.socket.as_raw_fd(), self.deadline)?
                }
                result => return result,
            }
        }
    }
}
struct Prepared {
    handle: IntentHandle,
    target: TargetIdentity,
    mutation: Mutation,
    bytes: Vec<u8>,
}
impl Prepared {
    fn matches(&self, permit: &ExternalPermit<'_>) -> bool {
        Arc::ptr_eq(&self.handle.owner, &permit.handle.owner)
            && self.handle.id == permit.handle.id
            && self.target == permit.target
            && self.mutation == *permit.mutation
    }
}
/// Constructed only after a bounded reader authenticates correlation. Reader
/// waiting never borrows the coordination actor; apply is a separate actor turn.
pub struct CorrelatedReceipt {
    handle: IntentHandle,
    receipt: Receipt,
}
impl CorrelatedReceipt {
    pub fn apply(self, actor: &mut Actor) -> Result<Status> {
        Ok(actor.receipt(&self.handle, self.receipt)?)
    }
}
/// Owns the writer FD after a clean loopback WS handshake. No stream accessor.
/// Constructor target binding is injected attestation evidence in this prototype.
pub struct RawWebSocketWriter {
    socket: TcpStream,
    prepared: Prepared,
    last: Option<WriteReport>,
}
impl RawWebSocketWriter {
    pub fn prepare_handoff(
        socket: TcpStream,
        target: TargetIdentity,
        handle: &IntentHandle,
        mutation: Mutation,
    ) -> Result<Self> {
        ensure!(
            socket.peer_addr()?.ip().is_loopback(),
            "only loopback cleartext WebSocket handoff is supported"
        );
        let Mutation::CdpNavigate { url } = &mutation else {
            bail!("only classified Page.navigate is supported by this socket spike");
        };
        ensure!(mutation.bounded(), "oversized mutation");
        let payload = serde_json::to_vec(
            &serde_json::json!({"id":handle.id,"method":"Page.navigate","params":{"url":url}}),
        )?;
        let bytes = masked_text_frame(&payload)?;
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket,
            prepared: Prepared {
                handle: handle.clone(),
                target,
                mutation,
                bytes,
            },
            last: None,
        })
    }
    pub fn last_write(&self) -> Option<WriteReport> {
        self.last
    }
}
impl sealed::Approved for RawWebSocketWriter {}
impl CommitTransport for RawWebSocketWriter {
    fn route(&self) -> Route {
        Route::Cdp
    }
    fn intent_binding(&self) -> Option<&IntentHandle> {
        Some(&self.prepared.handle)
    }
    fn binding(&self) -> Option<TargetIdentity> {
        Some(self.prepared.target)
    }
    fn supports(&self, mutation: &Mutation) -> bool {
        mutation == &self.prepared.mutation
    }
    fn try_commit(&mut self, permit: ExternalPermit<'_>) -> CommitAttempt {
        if !self.prepared.matches(&permit) {
            return CommitAttempt::Disconnected;
        }
        let report = write_once(self.socket.as_raw_fd(), &self.prepared.bytes);
        if matches!(
            report.attempt,
            CommitAttempt::Partial | CommitAttempt::Disconnected
        ) {
            let _ = self.socket.shutdown(Shutdown::Both);
        }
        self.last = Some(report);
        report.attempt
    }
}
fn masked_text_frame(payload: &[u8]) -> Result<Vec<u8>> {
    ensure!(payload.len() <= 65535, "fixture WS frame exceeds limit");
    let mut mask = [0; 4];
    getrandom::getrandom(&mut mask)
        .map_err(|_| anyhow::anyhow!("WebSocket masking entropy unavailable"))?;
    let mut bytes = Vec::with_capacity(payload.len() + 8);
    bytes.push(0x81);
    if payload.len() < 126 {
        bytes.push(0x80 | payload.len() as u8);
    } else {
        bytes.push(0x80 | 126);
        bytes.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    }
    bytes.extend_from_slice(&mask);
    bytes.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    Ok(bytes)
}
/// Read half rejects every write. A production control-frame broker must route
/// ping/pong/close through the actor too; tungstenite reading is not writer-free.
pub struct ReadOnlySocket {
    socket: TcpStream,
    deadline: std::time::Instant,
}
impl Read for ReadOnlySocket {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        loop {
            match self.socket.read(bytes) {
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    wait_readable(self.socket.as_raw_fd(), self.deadline)?
                }
                result => return result,
            }
        }
    }
}
impl Write for ReadOnlySocket {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "reader has no writer authority",
        ))
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub struct WebSocketAckReader {
    reader: tungstenite::WebSocket<ReadOnlySocket>,
    handle: IntentHandle,
}
impl WebSocketAckReader {
    /// Fixture reader only. Handshake must have no buffered post-upgrade frames.
    pub fn from_clean_handoff(socket: TcpStream, handle: &IntentHandle) -> Result<Self> {
        socket.set_read_timeout(Some(std::time::Duration::from_secs(2)))?;
        Ok(Self {
            reader: tungstenite::WebSocket::from_raw_socket(
                ReadOnlySocket {
                    socket,
                    deadline: std::time::Instant::now() + std::time::Duration::from_secs(2),
                },
                tungstenite::protocol::Role::Client,
                Some(tungstenite::protocol::WebSocketConfig {
                    max_message_size: Some(65536),
                    max_frame_size: Some(65536),
                    ..Default::default()
                }),
            ),
            handle: handle.clone(),
        })
    }
    pub fn receive(&mut self) -> Result<CorrelatedReceipt> {
        self.reader.get_mut().deadline =
            std::time::Instant::now() + std::time::Duration::from_secs(2);
        let message = self.reader.read()?;
        let reply: serde_json::Value = serde_json::from_str(message.to_text()?)?;
        ensure!(
            reply.get("id").and_then(serde_json::Value::as_u64) == Some(self.handle.id),
            "uncorrelated CDP reply"
        );
        let receipt = if reply.get("result").is_some() && reply.get("error").is_none() {
            Receipt::Acknowledged
        } else {
            Receipt::Rejected
        };
        Ok(CorrelatedReceipt {
            handle: self.handle.clone(),
            receipt,
        })
    }
}
/// Raw Unix D-Bus writer. The caller hands over an already authenticated
/// connection with no other writer, a pinned unique destination owner, and an
/// explicitly prepared classified intent. This spike never uses zbus::Connection::send.
pub struct RawDbusWriter {
    socket: UnixStream,
    prepared: Prepared,
    last: Option<WriteReport>,
}
impl RawDbusWriter {
    pub fn prepare_handoff(
        socket: UnixStream,
        target: TargetIdentity,
        handle: &IntentHandle,
        mutation: Mutation,
        destination: &str,
    ) -> Result<Self> {
        ensure!(
            destination.starts_with(':'),
            "D-Bus destination must be a pinned unique owner"
        );
        ensure!(mutation.bounded(), "oversized mutation");
        let serial = NonZeroU32::new(handle.id.try_into()?).context("invalid D-Bus serial")?;
        let message = match &mutation {
            Mutation::AtspiInvoke { object, index } => {
                zbus::Message::method_call(object.as_str(), "DoAction")?
                    .destination(destination)?
                    .interface("org.a11y.atspi.Action")?
                    .serial(serial)
                    .build(&(*index as i32,))?
            }
            Mutation::AtspiSetValue { object, value } => {
                zbus::Message::method_call(object.as_str(), "SetTextContents")?
                    .destination(destination)?
                    .interface("org.a11y.atspi.EditableText")?
                    .serial(serial)
                    .build(&(value,))?
            }
            _ => bail!("unclassified D-Bus mutation"),
        };
        ensure!(
            message.data().fds().is_empty(),
            "ancillary descriptors unsupported"
        );
        socket.set_nonblocking(true)?;
        let bytes = message.data().bytes().to_vec();
        Ok(Self {
            socket,
            prepared: Prepared {
                handle: handle.clone(),
                target,
                mutation,
                bytes,
            },
            last: None,
        })
    }
    pub fn last_write(&self) -> Option<WriteReport> {
        self.last
    }
}
impl sealed::Approved for RawDbusWriter {}
impl CommitTransport for RawDbusWriter {
    fn route(&self) -> Route {
        Route::Atspi
    }
    fn intent_binding(&self) -> Option<&IntentHandle> {
        Some(&self.prepared.handle)
    }
    fn binding(&self) -> Option<TargetIdentity> {
        Some(self.prepared.target)
    }
    fn supports(&self, mutation: &Mutation) -> bool {
        mutation == &self.prepared.mutation
    }
    fn try_commit(&mut self, permit: ExternalPermit<'_>) -> CommitAttempt {
        if !self.prepared.matches(&permit) {
            return CommitAttempt::Disconnected;
        }
        let report = write_once(self.socket.as_raw_fd(), &self.prepared.bytes);
        if matches!(
            report.attempt,
            CommitAttempt::Partial | CommitAttempt::Disconnected
        ) {
            let _ = self.socket.shutdown(Shutdown::Both);
        }
        self.last = Some(report);
        report.attempt
    }
}
/// Reads and safely decodes one descriptor-free fixture D-Bus acknowledgement.
/// It is deliberately limited to a bool method return; general bus routing and
/// ancillary/control messages belong to a coherent production broker.
pub fn receive_dbus_ack(
    reader: &mut UnixStream,
    handle: &IntentHandle,
) -> Result<CorrelatedReceipt> {
    reader.set_read_timeout(Some(std::time::Duration::from_secs(2)))?;
    let mut reader = DeadlineReader {
        socket: reader,
        deadline: std::time::Instant::now() + std::time::Duration::from_secs(2),
    };
    let mut prefix = [0u8; 16];
    reader.read_exact(&mut prefix)?;
    let endian = match prefix[0] {
        b'l' => zbus::zvariant::Endian::Little,
        b'B' => zbus::zvariant::Endian::Big,
        _ => bail!("invalid D-Bus endianness"),
    };
    let word = |offset| {
        let bytes: [u8; 4] = prefix[offset..offset + 4].try_into().unwrap();
        match endian {
            zbus::zvariant::Endian::Little => u32::from_le_bytes(bytes),
            _ => u32::from_be_bytes(bytes),
        }
    };
    let body_size = word(4) as usize;
    let fields_size = word(12) as usize;
    let body_offset = (16usize
        .checked_add(fields_size)
        .context("header overflow")?
        + 7)
        & !7;
    let total = body_offset
        .checked_add(body_size)
        .context("D-Bus frame overflow")?;
    ensure!(
        total <= 65536 && total >= 16,
        "D-Bus reply size exceeds fixture limit"
    );
    let mut bytes = prefix.to_vec();
    bytes.resize(total, 0);
    reader.read_exact(&mut bytes[16..])?;
    let data = zbus::zvariant::serialized::Data::new(
        &bytes[..body_offset],
        zbus::zvariant::serialized::Context::new_dbus(endian, 0),
    );
    let (header, _): (zbus::message::Header<'_>, usize) = data.deserialize()?;
    ensure!(
        header.unix_fds().unwrap_or(0) == 0,
        "ancillary reply descriptors unsupported"
    );
    ensure!(
        header.reply_serial().map(|s| u64::from(s.get())) == Some(handle.id),
        "uncorrelated D-Bus reply"
    );
    let receipt = match header.message_type() {
        zbus::message::Type::MethodReturn => {
            ensure!(
                header.signature().to_string() == "b",
                "unexpected D-Bus acknowledgement signature"
            );
            let data = zbus::zvariant::serialized::Data::new(
                &bytes[body_offset..],
                zbus::zvariant::serialized::Context::new_dbus(endian, body_offset),
            );
            let (success, _): (bool, usize) = data.deserialize()?;
            if success {
                Receipt::Acknowledged
            } else {
                Receipt::Rejected
            }
        }
        zbus::message::Type::Error => Receipt::Rejected,
        _ => bail!("unexpected D-Bus reply"),
    };
    Ok(CorrelatedReceipt {
        handle: handle.clone(),
        receipt,
    })
}

fn live_fixture_target() -> Result<TargetIdentity> {
    let mut target = fixture_target();
    let pid = std::process::id();
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let (_, fields) = stat
        .rsplit_once(')')
        .context("invalid fixture process stat")?;
    target.process = ProcessIdentity {
        pid: NonZeroU32::new(pid).unwrap(),
        start_time: NonZeroU64::new(
            fields
                .split_whitespace()
                .nth(19)
                .context("fixture process start time missing")?
                .parse()?,
        )
        .context("zero fixture start time")?,
    };
    Ok(target)
}
fn prepared_actor(target: TargetIdentity, mutation: Mutation) -> Result<(Actor, IntentHandle)> {
    let mut actor = Actor::new(target.compositor_epoch);
    actor.install_target(target)?;
    let observed = actor.observe(target, NonZeroU64::new(1).unwrap())?;
    let grant = actor.grant(target, mutation, 100)?;
    let handle = actor.prepare(observed, grant, 90)?;
    Ok((actor, handle))
}
/// Actual loopback WS handshake, masked mutation frame, stalled fixture effect,
/// correlated reader acknowledgement. No CDP service or real browser is contacted.
pub fn prove_loopback_websocket() -> Result<String> {
    use std::{
        net::TcpListener,
        sync::{
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
        time::Duration,
    };
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
    let address = listener.local_addr()?;
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let effects = Arc::new(AtomicUsize::new(0));
    let server_effects = effects.clone();
    let server = std::thread::spawn(move || -> Result<()> {
        let (socket, _) = listener.accept()?;
        socket.set_read_timeout(Some(Duration::from_secs(2)))?;
        socket.set_write_timeout(Some(Duration::from_secs(2)))?;
        let mut ws = tungstenite::accept(socket)?;
        let request: serde_json::Value = serde_json::from_str(ws.read()?.to_text()?)?;
        ensure!(
            request["method"] == "Page.navigate"
                && request["params"]["url"] == "https://fixture.invalid/",
            "wrong mutation payload"
        );
        entered_tx.send(())?;
        release_rx.recv_timeout(Duration::from_secs(2))?;
        server_effects.fetch_add(1, Ordering::SeqCst);
        ws.send(tungstenite::Message::Text(
            serde_json::json!({"id":request["id"],"result":{}}).to_string(),
        ))?;
        Ok(())
    });
    let socket = TcpStream::connect_timeout(&address, Duration::from_secs(2))?;
    socket.set_read_timeout(Some(Duration::from_secs(2)))?;
    socket.set_write_timeout(Some(Duration::from_secs(2)))?;
    let (handshake, _) = tungstenite::client(format!("ws://{address}/fixture"), socket)?;
    let writer_socket = handshake.get_ref().try_clone()?;
    let reader_socket = handshake.get_ref().try_clone()?;
    drop(handshake);
    let target = live_fixture_target()?;
    let mutation = Mutation::CdpNavigate {
        url: "https://fixture.invalid/".into(),
    };
    let (mut actor, handle) = prepared_actor(target, mutation.clone())?;
    let mut reader = WebSocketAckReader::from_clean_handoff(reader_socket, &handle)?;
    let mut writer = RawWebSocketWriter::prepare_handoff(writer_socket, target, &handle, mutation)?;
    let status = actor.try_dispatch(
        &handle,
        Requirement::NoTransportCommitAfterTakeover,
        &mut writer,
    )?;
    ensure!(
        status == Status::InFlight,
        "WS fixture did not commit fully"
    );
    entered_rx.recv_timeout(Duration::from_secs(2))?;
    // The peer effect is stalled. Human processing uses no reader or writer.
    actor.human_takeover(target.client);
    ensure!(
        effects.load(Ordering::SeqCst) == 0,
        "fixture effect was not held"
    );
    release_tx.send(())?;
    let receipt = reader.receive()?;
    let status = receipt.apply(&mut actor)?;
    ensure!(
        status
            == Status::Finished(Outcome::Unknown {
                reason: UnknownReason::RevokedAfterCommit,
                late_receipt: Some(Receipt::Acknowledged)
            }),
        "late WS acknowledgement changed unknown outcome"
    );
    server
        .join()
        .map_err(|_| anyhow::anyhow!("WS fixture thread panicked"))??;
    ensure!(
        effects.load(Ordering::SeqCst) == 1,
        "WS fixture did not apply exactly one effect"
    );
    Ok(format!(
        "loopback WS\t{:?}\t{status:?}",
        writer.last_write().unwrap()
    ))
}
struct DbusFixtureAction {
    entered: std::sync::mpsc::Sender<()>,
    release: Arc<tokio::sync::Notify>,
    effects: Arc<std::sync::atomic::AtomicUsize>,
}
#[zbus::interface(name = "org.a11y.atspi.Action")]
impl DbusFixtureAction {
    async fn do_action(&self, index: i32) -> bool {
        if index != 7 {
            return false;
        }
        let _ = self.entered.send(());
        self.release.notified().await;
        self.effects
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        true
    }
}
fn authenticate_fixture(socket: &mut UnixStream) -> Result<()> {
    use std::time::Duration;
    socket.set_read_timeout(Some(Duration::from_secs(2)))?;
    socket.set_write_timeout(Some(Duration::from_secs(2)))?;
    // EXTERNAL auth is verified by zbus against Unix peer credentials.
    let uid = unsafe { libc::geteuid() }.to_string();
    let hex = uid.bytes().map(|b| format!("{b:02x}")).collect::<String>();
    socket.write_all(format!("\0AUTH EXTERNAL {hex}\r\n").as_bytes())?;
    let mut response = Vec::new();
    while !response.ends_with(b"\r\n") && response.len() < 4096 {
        let mut byte = [0];
        socket.read_exact(&mut byte)?;
        response.push(byte[0]);
    }
    ensure!(
        response.starts_with(b"OK ") && response.ends_with(b"\r\n"),
        "fixture D-Bus authentication refused"
    );
    socket.write_all(b"BEGIN\r\n")?;
    Ok(())
}
/// Authenticated Unix p2p D-Bus call against zbus's real object server. The
/// application effect waits separately; the actor never waits for its method.
pub fn prove_unix_dbus() -> Result<String> {
    use std::{
        sync::{
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
        time::Duration,
    };
    let (server_socket, mut client) = UnixStream::pair()?;
    let (entered_tx, entered_rx) = mpsc::channel();
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
    let release = Arc::new(tokio::sync::Notify::new());
    let server_release = release.clone();
    let effects = Arc::new(AtomicUsize::new(0));
    let server_effects = effects.clone();
    let server = std::thread::spawn(move || -> Result<()> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async move {
            server_socket.set_nonblocking(true)?;
            let connection = zbus::connection::Builder::async_io_unix_stream(server_socket)
                .p2p()
                .server(zbus::Guid::generate())?
                .serve_at(
                    "/fixture",
                    DbusFixtureAction {
                        entered: entered_tx,
                        release: server_release,
                        effects: server_effects,
                    },
                )?
                .build()
                .await?;
            tokio::time::timeout(Duration::from_secs(4), stop_rx).await??;
            connection.close().await?;
            Ok(())
        })
    });
    authenticate_fixture(&mut client)?;
    let mut reader = client.try_clone()?;
    let target = live_fixture_target()?;
    let mutation = Mutation::AtspiInvoke {
        object: "/fixture".into(),
        index: 7,
    };
    let (mut actor, handle) = prepared_actor(target, mutation.clone())?;
    let mut writer = RawDbusWriter::prepare_handoff(client, target, &handle, mutation, ":1.5")?;
    ensure!(
        actor.try_dispatch(
            &handle,
            Requirement::NoTransportCommitAfterTakeover,
            &mut writer
        )? == Status::InFlight,
        "D-Bus fixture did not commit"
    );
    entered_rx.recv_timeout(Duration::from_secs(2))?;
    actor.human_takeover(target.client);
    ensure!(
        effects.load(Ordering::SeqCst) == 0,
        "D-Bus fixture effect was not held"
    );
    release.notify_one();
    let receipt = receive_dbus_ack(&mut reader, &handle)?;
    let status = receipt.apply(&mut actor)?;
    ensure!(
        status
            == Status::Finished(Outcome::Unknown {
                reason: UnknownReason::RevokedAfterCommit,
                late_receipt: Some(Receipt::Acknowledged)
            }),
        "late D-Bus receipt changed unknown outcome"
    );
    let _ = stop_tx.send(());
    server
        .join()
        .map_err(|_| anyhow::anyhow!("D-Bus fixture thread panicked"))??;
    ensure!(
        effects.load(Ordering::SeqCst) == 1,
        "D-Bus fixture did not apply one effect"
    );
    Ok(format!(
        "Unix D-Bus\t{:?}\t{status:?}",
        writer.last_write().unwrap()
    ))
}
fn small_send_buffer(socket: &UnixStream) -> Result<()> {
    let size = 4096i32;
    let result = unsafe {
        libc::setsockopt(
            socket.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_SNDBUF,
            (&size as *const i32).cast(),
            std::mem::size_of_val(&size) as libc::socklen_t,
        )
    };
    ensure!(result == 0, "fixture send-buffer configuration failed");
    Ok(())
}
/// Actual kernel backpressure and partial message acceptance with no active
/// receiver. These streams are wire-framing fixtures, not authenticated buses.
pub fn prove_unix_pressure() -> Result<Vec<String>> {
    let target = live_fixture_target()?;
    let mut reports = Vec::new();
    let (mut socket, mut peer) = UnixStream::pair()?;
    small_send_buffer(&socket)?;
    socket.set_nonblocking(true)?;
    let ping = zbus::Message::method_call("/fixture", "Ping")?
        .interface("org.freedesktop.DBus.Peer")?
        .build(&())?;
    let mut filled = 0;
    for _ in 0..2048 {
        match socket.write(ping.data().bytes()) {
            Ok(size) => filled += size,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(error) => return Err(error.into()),
        }
    }
    let mutation = Mutation::AtspiInvoke {
        object: "/fixture".into(),
        index: 7,
    };
    let (mut actor, handle) = prepared_actor(target, mutation.clone())?;
    let mut writer = RawDbusWriter::prepare_handoff(socket, target, &handle, mutation, ":1.5")?;
    ensure!(
        actor.try_dispatch(
            &handle,
            Requirement::NoTransportCommitAfterTakeover,
            &mut writer
        )? == Status::Queued,
        "full socket did not refuse commit"
    );
    let report = writer.last_write().unwrap();
    ensure!(
        report.attempt == CommitAttempt::WouldBlock && report.accepted == 0,
        "would-block accepted mutation bytes"
    );
    actor.human_takeover(target.client);
    ensure!(
        actor.try_dispatch(
            &handle,
            Requirement::NoTransportCommitAfterTakeover,
            &mut writer
        ) == Err(Refusal::Consumed),
        "revoked queued write retried"
    );
    peer.set_nonblocking(true)?;
    let mut received = 0;
    let mut bytes = [0; 8192];
    loop {
        match peer.read(&mut bytes) {
            Ok(0) => break,
            Ok(size) => received += size,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(error) => return Err(error.into()),
        }
    }
    ensure!(received == filled, "mutation appended bytes to full socket");
    reports.push(format!(
        "Unix would-block\t{report:?}\tfiller bytes {filled}"
    ));
    let (socket, mut peer) = UnixStream::pair()?;
    small_send_buffer(&socket)?;
    let mutation = Mutation::AtspiSetValue {
        object: "/fixture".into(),
        value: "x".repeat(16384),
    };
    let (mut actor, handle) = prepared_actor(target, mutation.clone())?;
    let mut writer = RawDbusWriter::prepare_handoff(socket, target, &handle, mutation, ":1.5")?;
    let status = actor.try_dispatch(
        &handle,
        Requirement::NoTransportCommitAfterTakeover,
        &mut writer,
    )?;
    let report = writer.last_write().unwrap();
    ensure!(
        report.attempt == CommitAttempt::Partial
            && report.accepted > 0
            && report.accepted < report.requested,
        "kernel did not produce partial fixture write"
    );
    ensure!(
        status
            == Status::Finished(Outcome::Unknown {
                reason: UnknownReason::PartialWrite,
                late_receipt: None
            }),
        "partial write was not unknown"
    );
    ensure!(
        actor.try_dispatch(
            &handle,
            Requirement::NoTransportCommitAfterTakeover,
            &mut writer
        ) == Err(Refusal::Consumed),
        "partial frame retried"
    );
    let mut received = Vec::new();
    peer.read_to_end(&mut received)?;
    ensure!(
        received.len() == report.accepted,
        "partial close wrote a hidden remainder"
    );
    reports.push(format!("Unix partial\t{report:?}\t{status:?}"));
    Ok(reports)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_loopback_ws_commit_and_late_receipt_preserve_unknown() {
        prove_loopback_websocket().unwrap();
    }
    #[test]
    fn authenticated_zbus_fixture_accepts_raw_commit_and_late_receipt() {
        prove_unix_dbus().unwrap();
    }
    #[test]
    fn unix_kernel_wouldblock_and_partial_have_no_hidden_retry() {
        prove_unix_pressure().unwrap();
    }
    #[test]
    fn real_writer_rejects_target_and_intent_mismatches_without_bytes() {
        let target = live_fixture_target().unwrap();
        let mutation = Mutation::AtspiInvoke {
            object: "/fixture".into(),
            index: 7,
        };
        let (mut actor, handle) = prepared_actor(target, mutation.clone()).unwrap();
        let mut wrong = target;
        wrong.document_generation = NonZeroU64::new(99).unwrap();
        let (socket, mut peer) = UnixStream::pair().unwrap();
        peer.set_nonblocking(true).unwrap();
        let mut writer =
            RawDbusWriter::prepare_handoff(socket, wrong, &handle, mutation.clone(), ":1.5")
                .unwrap();
        assert_eq!(
            actor.try_dispatch(
                &handle,
                Requirement::NoTransportCommitAfterTakeover,
                &mut writer
            ),
            Err(Refusal::StaleTarget)
        );
        let mut byte = [0];
        assert_eq!(
            peer.read(&mut byte).unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        assert!(writer.last_write().is_none());
        let stamp = actor.observe(target, NonZeroU64::new(1).unwrap()).unwrap();
        let grant = actor.grant(target, mutation.clone(), 100).unwrap();
        let other = actor.prepare(stamp, grant, 90).unwrap();
        let (socket, mut peer) = UnixStream::pair().unwrap();
        peer.set_nonblocking(true).unwrap();
        let mut writer =
            RawDbusWriter::prepare_handoff(socket, target, &other, mutation, ":1.5").unwrap();
        assert_eq!(
            actor.try_dispatch(
                &handle,
                Requirement::NoTransportCommitAfterTakeover,
                &mut writer
            ),
            Err(Refusal::WrongIntent)
        );
        assert_eq!(
            peer.read(&mut byte).unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        assert!(writer.last_write().is_none());
    }
}
