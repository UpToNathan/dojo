//! The greeter's end of the greeter socket: connect, say hello, read the display manager's
//! events on a thread of their own and send requests from any thread.
//!
//! The greeter trusts the socket only as far as the protocol decodes: a datagram that does
//! not decode, an `Init` that comes twice, or a closed socket are the caller's cue to exit,
//! because `dojo-dm` starts a fresh greeter.

use std::ffi::OsStr;
use std::fmt;
use std::io;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use super::DecodeError;
use super::protocol::{
    self, Event, Login, Request, SessionEntry, SessionId, VERSION,
};

/// Where `dojo-dm` listens.
pub const SOCKET: &str = "/run/dojo-dm/greeter.sock";

/// How long the display manager has to answer `Hello`.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

/// The socket to connect to. Test builds may point it elsewhere with `DOJO_DM_SOCKET`.
#[cfg(not(feature = "fake-pam-stack"))]
pub fn socket_path() -> PathBuf {
    PathBuf::from(SOCKET)
}

#[cfg(feature = "fake-pam-stack")]
pub fn socket_path() -> PathBuf {
    std::env::var_os("DOJO_DM_SOCKET").map_or_else(|| PathBuf::from(SOCKET), PathBuf::from)
}

/// The display manager runs as root.
#[cfg(not(feature = "fake-pam-stack"))]
fn server_uid() -> u32 {
    0
}

/// A test build talks to a fake display manager that the same user started.
#[cfg(feature = "fake-pam-stack")]
fn server_uid() -> u32 {
    unsafe { libc::geteuid() }
}

/// What the display manager says first.
#[derive(Clone, PartialEq, Debug)]
pub struct Init {
    pub last_login: Option<Login>,
    pub last_session: Option<SessionId>,
    pub sessions: Vec<SessionEntry>,
}

#[derive(Debug)]
pub enum HandshakeError {
    Io(io::Error),
    TimedOut,
    /// The display manager closed the socket without answering.
    Closed,
    Malformed(DecodeError),
    /// Something other than `Init` came first.
    Unexpected(Event),
}

impl fmt::Display for HandshakeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HandshakeError::Io(error) => write!(f, "{error}"),
            HandshakeError::TimedOut => f.write_str("the display manager did not answer hello"),
            HandshakeError::Closed => f.write_str("the display manager closed the socket"),
            HandshakeError::Malformed(error) => write!(f, "the display manager's answer: {error}"),
            HandshakeError::Unexpected(event) => {
                write!(f, "the display manager began with {event:?}, not init")
            }
        }
    }
}

impl std::error::Error for HandshakeError {}

/// What the reader thread passes on.
#[derive(Debug)]
pub enum Incoming {
    Event(Event),
    /// A datagram that did not decode. The socket is still usable; the greeter should not be.
    Malformed(DecodeError),
    /// The display manager went away. `None` is a clean end of stream.
    Closed(Option<io::Error>),
}

/// A connection. Cheap to clone: every clone shares the socket.
#[derive(Clone)]
pub struct Client {
    socket: Arc<OwnedFd>,
}

impl Client {
    /// Connects to the display manager's socket, whose other end must be the display manager.
    pub fn connect(path: &Path) -> io::Result<Client> {
        Client::connect_to(path, server_uid())
    }

    /// Connects to `path`, refusing a server that does not run as `server`.
    pub fn connect_to(path: &Path, server: u32) -> io::Result<Client> {
        let fd = unsafe {
            libc::socket(libc::AF_UNIX, libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC, 0)
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let socket = unsafe { OwnedFd::from_raw_fd(fd) };
        let address = unix_address(path)?;
        let rc = unsafe {
            libc::connect(
                socket.as_raw_fd(),
                (&address as *const libc::sockaddr_un).cast(),
                size_of::<libc::sockaddr_un>() as libc::socklen_t,
            )
        };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
        if protocol::peer_uid(socket.as_fd())? != server {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("the greeter socket is not served by uid {server}"),
            ));
        }
        Ok(Client::over(socket))
    }

    /// A client on a socket that is already connected.
    pub fn over(socket: OwnedFd) -> Client {
        Client {
            socket: Arc::new(socket),
        }
    }

    /// Sends `Hello` and waits for `Init`.
    pub fn handshake(&self, timeout: Duration) -> Result<Init, HandshakeError> {
        self.send(&Request::Hello { version: VERSION })
            .map_err(HandshakeError::Io)?;
        self.set_receive_timeout(Some(timeout))
            .map_err(HandshakeError::Io)?;
        let first = protocol::receive::<Event>(self.socket.as_fd());
        self.set_receive_timeout(None).map_err(HandshakeError::Io)?;
        match first {
            Ok(Some(Ok(Event::Init {
                last_login,
                last_session,
                sessions,
            }))) => Ok(Init {
                last_login,
                last_session,
                sessions,
            }),
            Ok(Some(Ok(event))) => Err(HandshakeError::Unexpected(event)),
            Ok(Some(Err(error))) => Err(HandshakeError::Malformed(error)),
            Ok(None) => Err(HandshakeError::Closed),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                Err(HandshakeError::TimedOut)
            }
            Err(error) => Err(HandshakeError::Io(error)),
        }
    }

    pub fn send(&self, request: &Request) -> io::Result<()> {
        protocol::send(self.socket.as_fd(), request)
    }

    /// Reads events until the socket closes, passing each as `wrap(incoming)` to `sender`.
    /// The thread ends with the socket, or when the receiver is dropped.
    pub fn read_in_background<T: Send + 'static>(
        &self,
        sender: async_channel::Sender<T>,
        wrap: fn(Incoming) -> T,
    ) -> io::Result<std::thread::JoinHandle<()>> {
        let socket = self.socket.clone();
        std::thread::Builder::new()
            .name("dojo-greeter-dm".into())
            .spawn(move || {
                loop {
                    let incoming = match protocol::receive::<Event>(socket.as_fd()) {
                        Ok(Some(Ok(event))) => Incoming::Event(event),
                        Ok(Some(Err(error))) => Incoming::Malformed(error),
                        Ok(None) => Incoming::Closed(None),
                        Err(error) => Incoming::Closed(Some(error)),
                    };
                    let last = matches!(incoming, Incoming::Closed(_));
                    if sender.send_blocking(wrap(incoming)).is_err() || last {
                        return;
                    }
                }
            })
    }

    fn set_receive_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        let value = libc::timeval {
            tv_sec: timeout.map_or(0, |t| t.as_secs() as libc::time_t),
            tv_usec: timeout.map_or(0, |t| t.subsec_micros() as libc::suseconds_t),
        };
        let rc = unsafe {
            libc::setsockopt(
                self.socket.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_RCVTIMEO,
                (&value as *const libc::timeval).cast(),
                size_of::<libc::timeval>() as libc::socklen_t,
            )
        };
        if rc == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
}

fn unix_address(path: &Path) -> io::Result<libc::sockaddr_un> {
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    let bytes = OsStr::as_bytes(path.as_os_str());
    if bytes.len() >= address.sun_path.len() || bytes.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the socket path does not fit a unix address",
        ));
    }
    for (slot, byte) in address.sun_path.iter_mut().zip(bytes) {
        *slot = *byte as libc::c_char;
    }
    Ok(address)
}

#[cfg(test)]
mod tests;
