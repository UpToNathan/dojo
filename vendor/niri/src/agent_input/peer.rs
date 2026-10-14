//! Kernel process binding for IPC seats. Identity does not enroll a broker.
use anyhow::{ensure, Context, Result};
use std::{
    fs::File,
    io::Read,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::net::UnixStream,
    },
};

pub(crate) struct KernelPeer {
    pid: u32,
    uid: u32,
    start: u64,
    pidfd: OwnedFd,
    connection: UnixStream,
}
impl KernelPeer {
    pub(crate) fn capture(stream: &UnixStream) -> Result<Self> {
        let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
        let mut length = std::mem::size_of_val(&credentials) as libc::socklen_t;
        ensure!(
            unsafe {
                libc::getsockopt(
                    stream.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_PEERCRED,
                    (&mut credentials as *mut libc::ucred).cast(),
                    &mut length,
                )
            } == 0
                && length as usize == std::mem::size_of_val(&credentials)
                && credentials.pid > 0,
            "IPC peer credentials unavailable"
        );
        // This handle comes from the socket's actual peer, not a numeric PID
        // lookup that could attach to a replacement after the peer died.
        let mut raw = -1i32;
        length = std::mem::size_of_val(&raw) as _;
        ensure!(
            unsafe {
                libc::getsockopt(
                    stream.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_PEERPIDFD,
                    (&mut raw as *mut i32).cast(),
                    &mut length,
                )
            } == 0
                && raw >= 0,
            "generation-bound IPC peer handle unavailable"
        );
        let pidfd = unsafe { OwnedFd::from_raw_fd(raw) };
        ensure!(
            length as usize == std::mem::size_of_val(&raw),
            "invalid IPC peer handle size"
        );
        ensure!(live(&pidfd), "IPC peer already exited");
        let pid = credentials.pid as u32;
        let mut bytes = Vec::new();
        File::open(format!("/proc/{pid}/stat"))?
            .take(8193)
            .read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= 8192, "IPC process metadata exceeds limit");
        let start =
            start_time(std::str::from_utf8(&bytes)?).context("IPC peer generation unavailable")?;
        ensure!(live(&pidfd), "IPC peer exited during identity capture");
        Ok(Self {
            pid,
            uid: credentials.uid,
            start,
            pidfd,
            connection: stream.try_clone()?,
        })
    }
    pub(crate) fn matches_process(&self, pid: u32, start: u64) -> bool {
        self.pid == pid
            && self.start == start
            && self.uid == unsafe { libc::geteuid() }
            && live(&self.pidfd)
            && connection_live(&self.connection)
    }
}
fn connection_live(connection: &UnixStream) -> bool {
    // RDHUP detects a closed sender even when unread protocol bytes precede EOF.
    // This is checked by the final writer before the IPC cleanup callback runs.
    let mut descriptor = libc::pollfd {
        fd: connection.as_raw_fd(),
        events: libc::POLLRDHUP,
        revents: 0,
    };
    let result = unsafe { libc::poll(&mut descriptor, 1, 0) };
    result >= 0
        && descriptor.revents & (libc::POLLRDHUP | libc::POLLHUP | libc::POLLERR | libc::POLLNVAL)
            == 0
}
fn live(handle: &OwnedFd) -> bool {
    let mut descriptor = libc::pollfd {
        fd: handle.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    unsafe { libc::poll(&mut descriptor, 1, 0) == 0 }
}
fn start_time(stat: &str) -> Option<u64> {
    stat.rsplit_once(')')?
        .1
        .split_whitespace()
        .nth(19)?
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_socket_peer_binds_kernel_generation_without_json_identity() {
        let (stream, _other) = UnixStream::pair().unwrap();
        let peer = KernelPeer::capture(&stream).unwrap();
        assert_eq!(peer.pid, std::process::id());
        assert!(peer.matches_process(peer.pid, peer.start));
        assert!(!peer.matches_process(peer.pid, peer.start + 1));
        assert!(!peer.matches_process(peer.pid + 1, peer.start));
        assert_ne!(
            unsafe { libc::fcntl(peer.pidfd.as_raw_fd(), libc::F_GETFD) } & libc::FD_CLOEXEC,
            0
        );
    }
    #[test]
    fn process_names_with_parentheses_do_not_shift_the_generation_field() {
        let mut fields = vec!["S".to_owned()];
        fields.extend((4..=21).map(|field| field.to_string()));
        fields.push("987654".to_owned());
        assert_eq!(
            start_time(&format!("42 (name ) with spaces) {}", fields.join(" "))),
            Some(987654)
        );
    }
    #[test]
    fn closed_connection_retires_identity_even_with_unread_bytes_and_live_process() {
        use std::io::{Read, Write};
        let (mut stream, mut other) = UnixStream::pair().unwrap();
        let peer = KernelPeer::capture(&stream).unwrap();
        other.write_all(b"queued request").unwrap();
        assert!(peer.matches_process(peer.pid, peer.start));
        other.shutdown(std::net::Shutdown::Write).unwrap();
        assert!(live(&peer.pidfd));
        assert!(!peer.matches_process(peer.pid, peer.start));
        let mut bytes = [0; 14];
        stream.read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes, b"queued request");
    }
}
