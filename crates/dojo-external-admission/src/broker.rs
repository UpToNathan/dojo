//! Trusted local bootstrap for Niri's single-use Page.navigate writer. This is
//! transport preparation only; it never writes a computer mutation itself.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    net::{SocketAddrV4, TcpStream},
    os::{
        fd::AsRawFd,
        unix::{fs::PermissionsExt, net::UnixDatagram},
    },
    path::{Path, PathBuf},
    time::Duration,
};
#[derive(Debug, Serialize)]
pub struct NavigateIntent {
    pub version: u32,
    pub peer: u64,
    pub revision: u64,
    pub window_id: u64,
    pub browser_pid: u32,
    pub browser_start_time: u64,
    pub page_target: String,
    pub request_id: u64,
    pub url: String,
    pub intent_id: String,
    pub grant_id: String,
    pub lifetime_ms: u32,
    pub capture_geometry: serde_json::Value,
}
#[derive(Debug, Deserialize)]
pub struct AdmissionReceipt {
    pub request_id: u64,
    pub intent_id: String,
    pub effect: String,
    pub accepted_bytes: usize,
    pub message: String,
}
struct Cleanup(PathBuf);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
/// Caller must prove this page endpoint belongs to its fresh exact native window
/// through the browser observation engine, and commit Pi intent first and obtain host authorization.
/// Upgrade path is constructed from that same target and passed unchanged to Niri.
/// The socket is consumed, no WebSocket sink or writable clone survives transfer.
pub fn navigate(
    socket_path: &Path,
    endpoint: SocketAddrV4,
    intent: NavigateIntent,
) -> Result<AdmissionReceipt> {
    navigate_cancellable(socket_path, endpoint, intent, None)
}

/// Cancellation before transfer is a no-effect refusal. Once the FD is sent,
/// admission may already have happened and cancellation cannot promise rollback.
pub fn navigate_cancellable(
    socket_path: &Path,
    endpoint: SocketAddrV4,
    intent: NavigateIntent,
    cancelled: Option<&std::sync::atomic::AtomicBool>,
) -> Result<AdmissionReceipt> {
    let check_cancelled = || -> Result<()> {
        ensure!(
            !cancelled.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Acquire)),
            "navigation cancelled before descriptor transfer"
        );
        Ok(())
    };
    check_cancelled()?;
    ensure!(
        endpoint.ip().is_loopback(),
        "CDP endpoint must be IPv4 loopback"
    );
    ensure!(
        intent.version == 1
            && !intent.page_target.is_empty()
            && intent.page_target.len() <= 128
            && intent
                .page_target
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-'),
        "exact page target required"
    );
    ensure!(
        (1..=5000).contains(&intent.lifetime_ms),
        "invalid intent lifetime"
    );
    let mut entropy = [0u8; 16];
    getrandom::getrandom(&mut entropy)?;
    let directory = std::env::temp_dir().join(format!(
        "dojo-admission-{}-{}",
        std::process::id(),
        entropy
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    ));
    std::fs::create_dir(&directory)?;
    let _cleanup = Cleanup(directory.clone());
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?;
    let channel = UnixDatagram::bind(directory.join("reply.sock"))?;
    channel.connect(socket_path)?;
    channel.set_read_timeout(Some(Duration::from_millis(
        u64::from(intent.lifetime_ms) + 1000,
    )))?;
    enable_credentials(&channel)?;
    let actor_pid = intent.capture_geometry["compositor_pid"]
        .as_u64()
        .and_then(|pid| u32::try_from(pid).ok())
        .context("capture compositor PID required")?;
    let actor_start = intent.capture_geometry["compositor_start_time"]
        .as_u64()
        .context("capture compositor start required")?;
    let nonce = entropy
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    channel.send(&serde_json::to_vec(
        &serde_json::json!({"version":1,"nonce":nonce}),
    )?)?;
    let probe = authenticated_receive(&channel, actor_pid, actor_start)?;
    let probe: serde_json::Value = serde_json::from_slice(&probe)?;
    ensure!(
        probe["nonce"].as_str() == Some(nonce.as_str()),
        "uncorrelated actor authentication probe"
    );
    check_cancelled()?;
    let socket = upgrade(endpoint, &intent.page_target)?;
    let payload = serde_json::to_vec(&intent)?;
    ensure!(payload.len() <= 8192, "bounded broker packet");
    check_cancelled()?;
    transfer(&channel, &socket, &payload)?;
    // Drop the only broker-side socket owner immediately. The compositor receives
    // its own FD; ordinary Drop does not call shutdown on that shared connection.
    drop(socket);
    let bytes = authenticated_receive(&channel, actor_pid, actor_start)
        .context("admission response missing; outcome unknown, never replay")?;
    let receipt: AdmissionReceipt = serde_json::from_slice(&bytes)?;
    ensure!(
        receipt.request_id == intent.request_id && receipt.intent_id == intent.intent_id,
        "uncorrelated admission receipt"
    );
    ensure!(
        matches!(receipt.effect.as_str(), "refused" | "unknown"),
        "unrecognized admission outcome"
    );
    Ok(receipt)
}
fn enable_credentials(channel: &UnixDatagram) -> Result<()> {
    let enabled: libc::c_int = 1;
    ensure!(
        unsafe {
            libc::setsockopt(
                channel.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PASSCRED,
                (&enabled as *const libc::c_int).cast(),
                std::mem::size_of_val(&enabled) as _,
            )
        } == 0,
        "broker credential delivery unavailable"
    );
    Ok(())
}
fn authenticated_receive(
    channel: &UnixDatagram,
    actor_pid: u32,
    actor_start: u64,
) -> Result<Vec<u8>> {
    use std::os::fd::{FromRawFd, OwnedFd};
    let mut bytes = [0u8; 8192];
    let mut control = [0usize; 32];
    let mut iov = libc::iovec {
        iov_base: bytes.as_mut_ptr().cast(),
        iov_len: bytes.len(),
    };
    let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
    message.msg_iov = &mut iov;
    message.msg_iovlen = 1;
    message.msg_control = control.as_mut_ptr().cast();
    message.msg_controllen = std::mem::size_of_val(&control);
    let count = unsafe { libc::recvmsg(channel.as_raw_fd(), &mut message, libc::MSG_CMSG_CLOEXEC) };
    ensure!(
        count >= 0,
        "actor response failed: {}",
        std::io::Error::last_os_error()
    );
    let mut credential = None;
    let mut invalid = false;
    let mut header = unsafe { libc::CMSG_FIRSTHDR(&message) };
    while !header.is_null() {
        let h = unsafe { &*header };
        if h.cmsg_level == libc::SOL_SOCKET
            && h.cmsg_type == libc::SCM_CREDENTIALS
            && h.cmsg_len
                == unsafe { libc::CMSG_LEN(std::mem::size_of::<libc::ucred>() as _) } as usize
        {
            if credential.is_some() {
                invalid = true;
            }
            credential =
                Some(unsafe { (libc::CMSG_DATA(header) as *const libc::ucred).read_unaligned() });
        } else {
            invalid = true;
            if h.cmsg_level == libc::SOL_SOCKET && h.cmsg_type == libc::SCM_RIGHTS {
                let count = h
                    .cmsg_len
                    .saturating_sub(unsafe { libc::CMSG_LEN(0) } as usize)
                    / std::mem::size_of::<i32>();
                for i in 0..count {
                    let fd = unsafe {
                        (libc::CMSG_DATA(header) as *const i32)
                            .add(i)
                            .read_unaligned()
                    };
                    if fd >= 0 {
                        drop(unsafe { OwnedFd::from_raw_fd(fd) });
                    }
                }
            }
        }
        header = unsafe { libc::CMSG_NXTHDR(&message, header) };
    }
    ensure!(
        !invalid && message.msg_flags & (libc::MSG_TRUNC | libc::MSG_CTRUNC) == 0,
        "invalid actor ancillary response"
    );
    let credential = credential.context("actor credentials absent")?;
    ensure!(
        credential.pid > 0
            && credential.pid as u32 == actor_pid
            && credential.uid == unsafe { libc::geteuid() },
        "response is not from the admitted compositor"
    );
    let stat = std::fs::read_to_string(format!("/proc/{actor_pid}/stat"))?;
    let start = stat
        .rsplit_once(')')
        .context("actor process stat")?
        .1
        .split_whitespace()
        .nth(19)
        .context("actor start missing")?
        .parse::<u64>()?;
    ensure!(
        start == actor_start,
        "admitted compositor generation changed"
    );
    Ok(bytes[..count as usize].to_vec())
}

fn upgrade(endpoint: SocketAddrV4, target: &str) -> Result<TcpStream> {
    let mut socket = TcpStream::connect_timeout(&endpoint.into(), Duration::from_secs(2))?;
    socket.set_read_timeout(Some(Duration::from_secs(2)))?;
    socket.set_write_timeout(Some(Duration::from_secs(2)))?;
    let key = tungstenite::handshake::client::generate_key();
    let request = format!(
        "GET /devtools/page/{target} HTTP/1.1\r\nHost: {endpoint}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
    );
    socket.write_all(request.as_bytes())?;
    // One byte at a time prevents hidden library read-ahead buffers or unread
    // bootstrap control frames from being discarded when the FD is transferred.
    let mut response = Vec::new();
    while !response.ends_with(b"\r\n\r\n") {
        ensure!(response.len() < 8192, "upgrade header limit");
        let mut byte = [0u8; 1];
        socket.read_exact(&mut byte)?;
        response.push(byte[0]);
    }
    let response = std::str::from_utf8(&response)?;
    let mut lines = response.split("\r\n");
    ensure!(
        lines.next().is_some_and(|s| s.starts_with("HTTP/1.1 101 ")),
        "websocket upgrade refused"
    );
    let mut headers = std::collections::HashMap::new();
    for line in lines.filter(|s| !s.is_empty()) {
        let (name, value) = line.split_once(':').context("invalid upgrade header")?;
        ensure!(
            headers
                .insert(name.to_ascii_lowercase(), value.trim())
                .is_none(),
            "duplicate upgrade header"
        );
    }
    ensure!(
        headers
            .get("upgrade")
            .is_some_and(|s| s.eq_ignore_ascii_case("websocket"))
            && headers.get("connection").is_some_and(|s| s
                .split(',')
                .any(|s| s.trim().eq_ignore_ascii_case("upgrade"))),
        "invalid upgrade headers"
    );
    ensure!(
        headers.get("sec-websocket-accept")
            == Some(&tungstenite::handshake::derive_accept_key(key.as_bytes()).as_str()),
        "invalid upgrade accept key"
    );
    ensure!(
        !headers.contains_key("sec-websocket-extensions")
            && !headers.contains_key("sec-websocket-protocol"),
        "unrequested websocket negotiation"
    );
    Ok(socket)
}
fn transfer(channel: &UnixDatagram, socket: &TcpStream, payload: &[u8]) -> Result<()> {
    let fd = socket.as_raw_fd();
    let mut control = [0usize; 8];
    let mut iov = libc::iovec {
        iov_base: payload.as_ptr().cast_mut().cast(),
        iov_len: payload.len(),
    };
    let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
    message.msg_iov = &mut iov;
    message.msg_iovlen = 1;
    message.msg_control = control.as_mut_ptr().cast();
    message.msg_controllen = unsafe { libc::CMSG_SPACE(std::mem::size_of_val(&fd) as _) } as usize;
    unsafe {
        let header = libc::CMSG_FIRSTHDR(&message);
        (*header).cmsg_level = libc::SOL_SOCKET;
        (*header).cmsg_type = libc::SCM_RIGHTS;
        (*header).cmsg_len = libc::CMSG_LEN(std::mem::size_of_val(&fd) as _) as usize;
        (libc::CMSG_DATA(header) as *mut i32).write_unaligned(fd);
    }
    let count = unsafe { libc::sendmsg(channel.as_raw_fd(), &message, libc::MSG_NOSIGNAL) };
    ensure!(
        count == payload.len() as isize,
        "descriptor registration failed: {}",
        std::io::Error::last_os_error()
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_upgrade_path_and_unread_control_frame_survive_bootstrap() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut header = Vec::new();
            while !header.ends_with(b"\r\n\r\n") {
                let mut b = [0];
                socket.read_exact(&mut b).unwrap();
                header.push(b[0]);
            }
            let request = String::from_utf8(header).unwrap();
            assert!(request.starts_with("GET /devtools/page/PAGE-1 HTTP/1.1\r\n"));
            let key = request
                .lines()
                .find_map(|s| s.strip_prefix("Sec-WebSocket-Key: "))
                .unwrap()
                .trim();
            let accept = tungstenite::handshake::derive_accept_key(key.as_bytes());
            socket.write_all(format!("HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Accept: {accept}\r\n\r\n").as_bytes()).unwrap();
            socket.write_all(&[0x89, 0]).unwrap();
        });
        let std::net::SocketAddr::V4(endpoint) = endpoint else {
            panic!("ipv4")
        };
        let mut socket = upgrade(endpoint, "PAGE-1").unwrap();
        let mut frame = [0; 2];
        socket.read_exact(&mut frame).unwrap();
        assert_eq!(frame, [0x89, 0]);
        server.join().unwrap();
    }
    #[test]
    fn substituted_actor_pid_is_rejected_before_fd_transfer() {
        let (server, client) = UnixDatagram::pair().unwrap();
        enable_credentials(&client).unwrap();
        server.send(br#"{"nonce":"probe"}"#).unwrap();
        let error =
            authenticated_receive(&client, std::process::id().saturating_add(1), 1).unwrap_err();
        assert!(error.to_string().contains("admitted compositor"));
    }
}
