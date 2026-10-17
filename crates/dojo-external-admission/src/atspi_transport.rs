//! One-use AT-SPI transport preparation. zbus authenticates and decodes; Niri
//! owns the mutation send. No mutation is submitted through a zbus connection.
use anyhow::{Context, Result, bail, ensure};
use async_io::Async;
use futures_lite::StreamExt;
use std::{
    io,
    num::NonZeroU32,
    os::{
        fd::{BorrowedFd, OwnedFd},
        unix::net::UnixStream,
    },
    sync::Arc,
    time::Instant,
};
use tokio::sync::Mutex;
use zbus::{
    Connection, Message, MessageStream,
    connection::socket::{Socket, Split, WriteHalf},
};

#[derive(Clone, Debug)]
pub enum Mutation {
    Invoke { index: u32 },
    SelectChild { index: u32 },
    SetText { text: String },
    SetNumeric { value: f64 },
}

pub struct PrepareRequest<'a> {
    pub bus_address: &'a str,
    pub unique_owner: &'a str,
    pub owner_pid: u32,
    pub owner_uid: u32,
    pub object_path: &'a str,
    pub mutation: Mutation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalReceipt {
    Acknowledged,
    Rejected,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnknownReason {
    Deadline,
    Disconnected,
    InvalidReply,
    PartialCommit,
    CanceledAfterCommit,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CallReceipt {
    /// Stays true after uncertainty, even when a later matching reply arrives.
    pub uncertain: bool,
    pub reason: Option<UnknownReason>,
    pub terminal: Option<TerminalReceipt>,
}
#[derive(Clone, Copy)]
enum ReplyKind {
    Bool,
    Empty,
}

/// Owns the read-only decoder and a single transferable commit descriptor.
/// Dropping the transferred descriptor before sending cancels the operation.
/// The caller must never retry bytes after a partial or successful commit.
pub struct PreparedCall {
    fd: Option<OwnedFd>,
    payload: Vec<u8>,
    owner: String,
    serial: NonZeroU32,
    expected: ReplyKind,
    _connection: Connection,
    replies: MessageStream,
    receipt: CallReceipt,
}
impl PreparedCall {
    pub fn take_commit_fd(&mut self) -> Result<OwnedFd> {
        self.fd
            .take()
            .context("commit descriptor already transferred")
    }
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
    pub fn request_serial(&self) -> NonZeroU32 {
        self.serial
    }
    pub fn owner(&self) -> &str {
        &self.owner
    }
    pub fn bus_guid(&self) -> &str {
        self._connection.server_guid().as_str()
    }
    pub fn sender(&self) -> &str {
        self._connection.unique_name().unwrap().as_str()
    }
    pub fn receipt(&self) -> CallReceipt {
        self.receipt
    }
    pub fn mark_unknown(&mut self, reason: UnknownReason) {
        self.receipt.uncertain = true;
        self.receipt.reason.get_or_insert(reason);
    }
    /// Only the expected unique sender and request serial may terminate this call.
    /// Deadlines do not destroy the decoder, so late receipts remain observable.
    pub async fn await_receipt(&mut self, deadline: Instant) -> CallReceipt {
        if self.receipt.terminal.is_some() {
            return self.receipt;
        }
        loop {
            if Instant::now() >= deadline {
                self.mark_unknown(UnknownReason::Deadline);
                break;
            }
            let next = tokio::time::timeout_at(deadline.into(), self.replies.next()).await;
            let message = match next {
                Err(_) => {
                    self.mark_unknown(UnknownReason::Deadline);
                    break;
                }
                Ok(None | Some(Err(_))) => {
                    self.mark_unknown(UnknownReason::Disconnected);
                    break;
                }
                Ok(Some(Ok(message))) => message,
            };
            let header = message.header();
            if header.sender().map(|s| s.as_str()) != Some(self.owner.as_str())
                || header.reply_serial() != Some(self.serial)
                || header.destination().map(|name| name.as_str()) != Some(self.sender())
            {
                continue;
            }
            if !message.data().fds().is_empty() {
                self.mark_unknown(UnknownReason::InvalidReply);
                break;
            }
            let terminal = match header.message_type() {
                zbus::message::Type::Error if header.error_name().is_some() => {
                    Some(TerminalReceipt::Rejected)
                }
                zbus::message::Type::Error => None,
                zbus::message::Type::MethodReturn => match self.expected {
                    ReplyKind::Bool if header.signature().to_string() == "b" => {
                        message.body().deserialize::<bool>().ok().map(|accepted| {
                            if accepted {
                                TerminalReceipt::Acknowledged
                            } else {
                                TerminalReceipt::Rejected
                            }
                        })
                    }
                    ReplyKind::Empty
                        if header.signature().to_string().is_empty()
                            && message.body().deserialize::<()>().is_ok() =>
                    {
                        Some(TerminalReceipt::Acknowledged)
                    }
                    _ => None,
                },
                _ => continue,
            };
            match terminal {
                Some(terminal) => self.receipt.terminal = Some(terminal),
                None => self.mark_unknown(UnknownReason::InvalidReply),
            }
            break;
        }
        self.receipt
    }
}

type UnixHalf = Arc<Async<UnixStream>>;
type WriterState = Arc<Mutex<Option<UnixHalf>>>;
#[derive(Debug)]
struct PreparationWriter(WriterState);
#[async_trait::async_trait]
impl WriteHalf for PreparationWriter {
    async fn sendmsg(&mut self, bytes: &[u8], fds: &[BorrowedFd<'_>]) -> io::Result<usize> {
        let mut state = self.0.lock().await;
        let writer = state.as_mut().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "D-Bus writer sealed for native admission",
            )
        })?;
        WriteHalf::sendmsg(writer, bytes, fds).await
    }
    async fn close(&mut self) -> io::Result<()> {
        // A decoder close must never shut down an already transferred actor FD.
        self.0.lock().await.take();
        Ok(())
    }
    async fn peer_credentials(&mut self) -> io::Result<zbus::fdo::ConnectionCredentials> {
        let mut state = self.0.lock().await;
        let writer = state
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::PermissionDenied, "sealed"))?;
        WriteHalf::peer_credentials(writer).await
    }
}
struct PreparationSocket {
    read: UnixHalf,
    writer: WriterState,
}
impl Socket for PreparationSocket {
    type ReadHalf = UnixHalf;
    type WriteHalf = PreparationWriter;
    fn split(self) -> Split<Self::ReadHalf, Self::WriteHalf> {
        Split::new(self.read, PreparationWriter(self.writer))
    }
}

/// Connect to one local bus and prove the selected owner on that connection.
/// The caller supplies a compositor-bound owner and validates native authority
/// again on the Niri event thread immediately before committing the payload.
pub async fn prepare(request: PrepareRequest<'_>, deadline: Instant) -> Result<PreparedCall> {
    tokio::time::timeout_at(deadline.into(), prepare_inner(request))
        .await
        .context("AT-SPI preparation deadline exceeded")?
}
async fn prepare_inner(request: PrepareRequest<'_>) -> Result<PreparedCall> {
    ensure!(
        request.bus_address.len() <= 4096 && !request.bus_address.contains(';'),
        "one bounded bus address required"
    );
    zbus::names::UniqueName::try_from(request.unique_owner)
        .context("unique AT-SPI owner required")?;
    zbus::zvariant::ObjectPath::try_from(request.object_path)?;
    ensure!(
        request.object_path != "/org/a11y/atspi/null",
        "null AT-SPI object"
    );
    ensure!(
        request.owner_pid > 0 && request.owner_uid == unsafe { libc::geteuid() },
        "invalid AT-SPI owner identity"
    );
    let address: zbus::Address = request.bus_address.try_into()?;
    let zbus::address::Transport::Unix(unix) = address.transport() else {
        bail!("local Unix AT-SPI bus required");
    };
    let stream = match unix.path() {
        zbus::address::transport::UnixSocket::File(path) => {
            Async::<UnixStream>::connect(path).await?
        }
        zbus::address::transport::UnixSocket::Abstract(name) => connect_abstract(name).await?,
        _ => bail!("connectable Unix AT-SPI bus address required"),
    };
    let stream = Arc::new(stream);
    let writer = Arc::new(Mutex::new(Some(stream.clone())));
    let connection = zbus::connection::Builder::socket(PreparationSocket {
        read: stream.clone(),
        writer: writer.clone(),
    })
    .auth_mechanism(zbus::AuthMechanism::External)
    .build()
    .await?;
    if let Some(expected) = address.guid() {
        ensure!(
            connection.server_guid().as_str() == expected.as_str(),
            "AT-SPI bus generation changed"
        );
    }
    let dbus = zbus::fdo::DBusProxy::new(&connection).await?;
    let owner = zbus::names::BusName::try_from(request.unique_owner)?;
    ensure!(
        dbus.get_name_owner(owner.clone()).await?.as_str() == request.unique_owner,
        "AT-SPI owner changed"
    );
    ensure!(
        dbus.get_connection_unix_process_id(owner.clone()).await? == request.owner_pid
            && dbus.get_connection_unix_user(owner).await? == request.owner_uid,
        "AT-SPI owner credentials mismatch"
    );
    drop(dbus);
    let (message, expected) = encode(
        &request,
        connection
            .unique_name()
            .context("bus Hello did not assign a name")?
            .as_str(),
    )?;
    ensure!(
        message.data().fds().is_empty() && message.data().bytes().len() <= 65536,
        "unsupported AT-SPI payload"
    );
    let mut replies = MessageStream::from(&connection);
    replies.set_max_queued(64);
    // Taking the write half waits for any preparation write to finish, then
    // makes every future zbus write fail before it can touch the transport.
    writer.lock().await.take();
    let fd: OwnedFd = stream.get_ref().try_clone()?.into();
    Ok(PreparedCall {
        fd: Some(fd),
        payload: message.data().bytes().to_vec(),
        owner: request.unique_owner.into(),
        serial: message.primary_header().serial_num(),
        expected,
        _connection: connection,
        replies,
        receipt: CallReceipt {
            uncertain: false,
            reason: None,
            terminal: None,
        },
    })
}
async fn connect_abstract(name: &std::ffi::OsStr) -> Result<Async<UnixStream>> {
    use std::os::{fd::FromRawFd, unix::ffi::OsStrExt};
    let name = name.as_bytes();
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    ensure!(
        !name.is_empty() && name.len() < address.sun_path.len(),
        "invalid abstract bus name"
    );
    address.sun_family = libc::AF_UNIX as _;
    for (target, byte) in address.sun_path[1..].iter_mut().zip(name) {
        *target = *byte as _;
    }
    let raw = unsafe {
        libc::socket(
            libc::AF_UNIX,
            libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
            0,
        )
    };
    ensure!(
        raw >= 0,
        "create Unix bus socket: {}",
        io::Error::last_os_error()
    );
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    let size = std::mem::offset_of!(libc::sockaddr_un, sun_path) + 1 + name.len();
    if unsafe {
        libc::connect(
            raw,
            (&address as *const libc::sockaddr_un).cast(),
            size as _,
        )
    } < 0
    {
        let error = io::Error::last_os_error();
        ensure!(
            error.raw_os_error() == Some(libc::EINPROGRESS),
            "connect abstract bus: {error}"
        );
    }
    let stream = Async::new(UnixStream::from(fd))?;
    stream.writable().await?;
    if let Some(error) = stream.get_ref().take_error()? {
        return Err(error.into());
    }
    stream.get_ref().peer_addr()?;
    Ok(stream)
}
fn encode(request: &PrepareRequest<'_>, sender: &str) -> Result<(Message, ReplyKind)> {
    let builder = |method| -> Result<_> {
        Ok(Message::method_call(request.object_path, method)?
            .destination(request.unique_owner)?
            .sender(sender)?)
    };
    Ok(match &request.mutation {
        Mutation::Invoke { index } => {
            ensure!(*index <= 128, "AT-SPI action index exceeds limit");
            (
                builder("DoAction")?
                    .interface("org.a11y.atspi.Action")?
                    .build(&(*index as i32,))?,
                ReplyKind::Bool,
            )
        }
        Mutation::SelectChild { index } => {
            ensure!(*index < 2048, "AT-SPI selection index exceeds limit");
            (
                builder("SelectChild")?
                    .interface("org.a11y.atspi.Selection")?
                    .build(&(*index as i32,))?,
                ReplyKind::Bool,
            )
        }
        Mutation::SetText { text } => {
            ensure!(text.len() <= 16384, "AT-SPI replacement text exceeds limit");
            (
                builder("SetTextContents")?
                    .interface("org.a11y.atspi.EditableText")?
                    .build(&(text,))?,
                ReplyKind::Bool,
            )
        }
        Mutation::SetNumeric { value } => {
            ensure!(value.is_finite(), "AT-SPI value must be finite");
            (
                builder("Set")?
                    .interface("org.freedesktop.DBus.Properties")?
                    .build(&(
                        "org.a11y.atspi.Value",
                        "CurrentValue",
                        zbus::zvariant::Value::F64(*value),
                    ))?,
                ReplyKind::Empty,
            )
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader},
        process::{Child, Command, Stdio},
        time::Duration,
    };
    struct Bus {
        child: Child,
        address: String,
        _directory: tempfile::TempDir,
    }
    impl Bus {
        fn start() -> Self {
            let directory = tempfile::tempdir().unwrap();
            let config = directory.path().join("bus.conf");
            std::fs::write(&config, format!("<busconfig><type>session</type><listen>unix:path={}</listen><auth>EXTERNAL</auth><policy context=\"default\"><allow send_destination=\"*\"/><allow receive_sender=\"*\"/><allow send_type=\"method_return\" send_requested_reply=\"false\"/><allow receive_type=\"method_return\" receive_requested_reply=\"false\"/><allow own=\"*\"/></policy></busconfig>", directory.path().join("bus").display())).unwrap();
            let mut child = Command::new("dbus-daemon")
                .arg("--config-file")
                .arg(config)
                .args(["--nofork", "--print-address=1", "--nopidfile", "--address"])
                .arg(format!(
                    "unix:path={}",
                    directory.path().join("bus").display()
                ))
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap();
            let mut address = String::new();
            BufReader::new(child.stdout.take().unwrap())
                .read_line(&mut address)
                .unwrap();
            Self {
                child,
                address: address.trim().into(),
                _directory: directory,
            }
        }
    }
    impl Drop for Bus {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
    fn deadline() -> Instant {
        Instant::now() + Duration::from_secs(3)
    }
    async fn call(bus: &Bus, server: &Connection, mutation: Mutation) -> PreparedCall {
        prepare(
            PrepareRequest {
                bus_address: &bus.address,
                unique_owner: server.unique_name().unwrap().as_str(),
                owner_pid: std::process::id(),
                owner_uid: unsafe { libc::geteuid() },
                object_path: "/fixture",
                mutation,
            },
            deadline(),
        )
        .await
        .unwrap()
    }
    fn commit(call: &mut PreparedCall) {
        use std::os::fd::AsRawFd;
        let fd = call.take_commit_fd().unwrap();
        assert!(call.take_commit_fd().is_err());
        let payload = call.payload();
        assert_eq!(
            unsafe {
                libc::send(
                    fd.as_raw_fd(),
                    payload.as_ptr().cast(),
                    payload.len(),
                    libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL,
                )
            },
            payload.len() as isize
        );
    }
    async fn request(stream: &mut MessageStream) -> Message {
        loop {
            let msg = tokio::time::timeout_at(deadline().into(), stream.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            if msg.header().message_type() == zbus::message::Type::MethodCall {
                return msg;
            }
        }
    }
    #[tokio::test]
    async fn real_bus_auth_hello_bool_and_empty_receipts() {
        let bus = Bus::start();
        let server = zbus::connection::Builder::address(bus.address.as_str())
            .unwrap()
            .build()
            .await
            .unwrap();
        let mut incoming = MessageStream::from(&server);
        for mutation in [
            Mutation::Invoke { index: 2 },
            Mutation::SelectChild { index: 1 },
            Mutation::SetText {
                text: "replacement".into(),
            },
            Mutation::SetNumeric { value: 42.5 },
        ] {
            let empty = matches!(mutation, Mutation::SetNumeric { .. });
            let mut prepared = call(&bus, &server, mutation).await;
            commit(&mut prepared);
            let message = request(&mut incoming).await;
            assert_eq!(
                message.primary_header().serial_num(),
                prepared.request_serial()
            );
            assert_eq!(message.header().path().unwrap().as_str(), "/fixture");
            if empty {
                let (interface, property, value): (String, String, zbus::zvariant::OwnedValue) =
                    message.body().deserialize().unwrap();
                assert_eq!(
                    (interface.as_str(), property.as_str()),
                    ("org.a11y.atspi.Value", "CurrentValue")
                );
                assert_eq!(f64::try_from(value).unwrap(), 42.5);
                server.reply(&message.header(), &()).await.unwrap();
            } else {
                server.reply(&message.header(), &true).await.unwrap();
            }
            assert_eq!(
                prepared.await_receipt(deadline()).await,
                CallReceipt {
                    uncertain: false,
                    reason: None,
                    terminal: Some(TerminalReceipt::Acknowledged)
                }
            );
        }
    }
    #[tokio::test]
    async fn unrelated_and_forged_replies_never_acknowledge_and_late_reply_keeps_unknown() {
        let bus = Bus::start();
        let server = zbus::connection::Builder::address(bus.address.as_str())
            .unwrap()
            .build()
            .await
            .unwrap();
        let attacker = zbus::connection::Builder::address(bus.address.as_str())
            .unwrap()
            .build()
            .await
            .unwrap();
        let mut incoming = MessageStream::from(&server);
        let mut prepared = call(&bus, &server, Mutation::Invoke { index: 0 }).await;
        commit(&mut prepared);
        let message = request(&mut incoming).await;
        attacker.reply(&message.header(), &true).await.unwrap();
        let unrelated = Message::method_call("/fixture", "Other")
            .unwrap()
            .sender(message.header().sender().unwrap())
            .unwrap()
            .build(&())
            .unwrap();
        server.reply(&unrelated.header(), &true).await.unwrap();
        let first = prepared
            .await_receipt(Instant::now() + Duration::from_millis(50))
            .await;
        assert_eq!(first.terminal, None);
        assert_eq!(first.reason, Some(UnknownReason::Deadline));
        server.reply(&message.header(), &false).await.unwrap();
        let late = prepared.await_receipt(deadline()).await;
        assert!(late.uncertain);
        assert_eq!(late.reason, Some(UnknownReason::Deadline));
        assert_eq!(late.terminal, Some(TerminalReceipt::Rejected));
    }
    #[tokio::test]
    async fn malformed_matching_reply_is_unknown_and_credentials_mismatch_refuses() {
        let bus = Bus::start();
        let server = zbus::connection::Builder::address(bus.address.as_str())
            .unwrap()
            .build()
            .await
            .unwrap();
        let mut incoming = MessageStream::from(&server);
        let refused = prepare(
            PrepareRequest {
                bus_address: &bus.address,
                unique_owner: server.unique_name().unwrap().as_str(),
                owner_pid: std::process::id() + 1,
                owner_uid: unsafe { libc::geteuid() },
                object_path: "/fixture",
                mutation: Mutation::Invoke { index: 0 },
            },
            deadline(),
        )
        .await;
        assert!(refused.is_err());
        let mut prepared = call(&bus, &server, Mutation::Invoke { index: 0 }).await;
        commit(&mut prepared);
        let message = request(&mut incoming).await;
        server
            .reply(&message.header(), &"wrong body")
            .await
            .unwrap();
        let receipt = prepared.await_receipt(deadline()).await;
        assert_eq!(receipt.reason, Some(UnknownReason::InvalidReply));
        assert_eq!(receipt.terminal, None);
    }
    #[tokio::test]
    async fn retained_connection_cannot_write_after_sealing() {
        let bus = Bus::start();
        let server = zbus::connection::Builder::address(bus.address.as_str())
            .unwrap()
            .build()
            .await
            .unwrap();
        let mut prepared = call(
            &bus,
            &server,
            Mutation::SetText {
                text: "sealed".into(),
            },
        )
        .await;
        let _actor_fd = prepared.take_commit_fd().unwrap();
        let message = Message::method_call("/fixture", "DoAction")
            .unwrap()
            .destination(server.unique_name().unwrap())
            .unwrap()
            .interface("org.a11y.atspi.Action")
            .unwrap()
            .build(&(0i32,))
            .unwrap();
        assert!(prepared._connection.send(&message).await.is_err());
        let mut incoming = MessageStream::from(&server);
        assert!(
            tokio::time::timeout(Duration::from_millis(50), request(&mut incoming))
                .await
                .is_err()
        );
    }
}
