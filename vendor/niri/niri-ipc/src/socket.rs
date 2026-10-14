//! Helper for blocking communication over the niri socket.

use std::env;
use std::io::{self, BufRead, BufReader, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::Path;

use crate::{Event, Reply, Request};

/// Name of the environment variable containing the niri IPC socket path.
pub const SOCKET_PATH_ENV: &str = "NIRI_SOCKET";

/// Helper for blocking communication over the niri socket.
///
/// This struct is used to communicate with the niri IPC server. It handles the socket connection
/// and serialization/deserialization of messages.
pub struct Socket {
    stream: BufReader<UnixStream>,
}

/// A connection retirement capability with no request or read operations.
pub struct RetirementHandle(UnixStream);
impl RetirementHandle {
    /// Close both directions immediately. Repeated retirement is harmless.
    pub fn retire(&self) {
        let _ = self.0.shutdown(Shutdown::Both);
    }
}
impl Drop for RetirementHandle {
    fn drop(&mut self) {
        self.retire();
    }
}

impl Socket {
    /// Retain an opaque capability to retire this connection from another thread.
    pub fn retirement_handle(&self) -> io::Result<RetirementHandle> {
        self.stream.get_ref().try_clone().map(RetirementHandle)
    }

    /// Close both directions, including any retained retirement capabilities.
    pub fn retire(&self) {
        let _ = self.stream.get_ref().shutdown(Shutdown::Both);
    }
    /// Connects to the default niri IPC socket.
    ///
    /// This is equivalent to calling [`Self::connect_to`] with the path taken from the
    /// [`SOCKET_PATH_ENV`] environment variable.
    pub fn connect() -> io::Result<Self> {
        let socket_path = env::var_os(SOCKET_PATH_ENV).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("{SOCKET_PATH_ENV} is not set, are you running this within niri?"),
            )
        })?;
        Self::connect_to(socket_path)
    }

    /// Connects to the niri IPC socket at the given path.
    pub fn connect_to(path: impl AsRef<Path>) -> io::Result<Self> {
        let stream = UnixStream::connect(path.as_ref())?;
        let stream = BufReader::new(stream);
        Ok(Self { stream })
    }

    /// Returns the kernel-attested process ID of this socket's peer.
    /// This query does not dispatch a compositor request or render a frame.
    #[cfg(target_os = "linux")]
    pub fn peer_process_id(&self) -> io::Result<u32> {
        use std::os::fd::AsRawFd;
        let mut credentials = std::mem::MaybeUninit::<libc::ucred>::uninit();
        let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        // The kernel initializes the credentials on success. Check the returned size before reading.
        let result = unsafe {
            libc::getsockopt(
                self.stream.get_ref().as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                credentials.as_mut_ptr().cast(),
                &mut length,
            )
        };
        if result != 0 {
            return Err(io::Error::last_os_error());
        }
        if length as usize != std::mem::size_of::<libc::ucred>() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid peer credentials size",
            ));
        }
        // A successful SO_PEERCRED query with the checked size initialized every field.
        let credentials = unsafe { credentials.assume_init() };
        u32::try_from(credentials.pid)
            .ok()
            .filter(|pid| *pid > 0)
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "peer has no process identity")
            })
    }

    /// Sets the timeout for blocking reads. `None` (the default) waits indefinitely,
    /// including on event streams. After a timed-out request, discard this socket: a
    /// late reply could otherwise be mistaken for the next request's response.
    pub fn set_read_timeout(&self, timeout: Option<std::time::Duration>) -> io::Result<()> {
        self.stream.get_ref().set_read_timeout(timeout)
    }

    /// Sets the timeout for blocking writes. `None` (the default) waits indefinitely.
    pub fn set_write_timeout(&self, timeout: Option<std::time::Duration>) -> io::Result<()> {
        self.stream.get_ref().set_write_timeout(timeout)
    }

    /// Sends a request to niri and returns the response.
    ///
    /// Return values:
    ///
    /// * `Ok(Ok(response))`: successful [`Response`](crate::Response) from niri
    /// * `Ok(Err(message))`: error message from niri
    /// * `Err(error)`: error communicating with niri
    pub fn send(&mut self, request: Request) -> io::Result<Reply> {
        let mut buf = serde_json::to_string(&request).unwrap();
        buf.push('\n');
        self.stream.get_mut().write_all(buf.as_bytes())?;

        buf.clear();
        self.stream.read_line(&mut buf)?;

        let reply = serde_json::from_str(&buf)?;
        Ok(reply)
    }

    /// Starts reading event stream [`Event`]s from the socket.
    ///
    /// The returned function will block until the next [`Event`] arrives, then return it.
    ///
    /// Use this only after requesting an [`EventStream`][Request::EventStream].
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use niri_ipc::{Request, Response};
    /// use niri_ipc::socket::Socket;
    ///
    /// fn main() -> std::io::Result<()> {
    ///     let mut socket = Socket::connect()?;
    ///
    ///     let reply = socket.send(Request::EventStream)?;
    ///     if matches!(reply, Ok(Response::Handled)) {
    ///         let mut read_event = socket.read_events();
    ///         while let Ok(event) = read_event() {
    ///             println!("Received event: {event:?}");
    ///         }
    ///     }
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn read_events(self) -> impl FnMut() -> io::Result<Event> {
        let Self { mut stream } = self;
        let _ = stream.get_mut().shutdown(Shutdown::Write);

        let mut buf = String::new();
        move || {
            buf.clear();
            stream.read_line(&mut buf)?;
            let event = serde_json::from_str(&buf)?;
            Ok(event)
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn retirement_closes_the_connection_while_the_request_owner_still_exists() {
        use std::io::Read;
        let (stream, mut peer) = UnixStream::pair().unwrap();
        let socket = Socket {
            stream: BufReader::new(stream),
        };
        let handle = socket.retirement_handle().unwrap();
        handle.retire();
        peer.set_read_timeout(Some(std::time::Duration::from_secs(1)))
            .unwrap();
        assert_eq!(peer.read(&mut [0]).unwrap(), 0);
        assert!(socket.stream.get_ref().write_all(b"late request").is_err());
        handle.retire();
    }

    #[test]
    fn peer_process_id_comes_from_connected_socket_without_dispatch() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("niri-peer-{}-{stamp}.sock", std::process::id()));
        let listener = UnixListener::bind(&path).unwrap();
        let socket = Socket::connect_to(&path).unwrap();
        let (mut peer, _) = listener.accept().unwrap();
        std::fs::remove_file(path).unwrap();
        assert_eq!(socket.peer_process_id().unwrap(), std::process::id());
        peer.set_nonblocking(true).unwrap();
        use std::io::Read;
        assert_eq!(
            peer.read(&mut [0u8]).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
    }
}
