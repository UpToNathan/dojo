use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    io,
    os::fd::{AsRawFd, OwnedFd},
};

pub const MAX_FRAME: usize = 1024 * 1024;
pub const MAX_PENDING: usize = 32;
pub const EXTENSION_ORIGIN: &str = "chrome-extension://eibhgjafffkigblngnhafgbcipofaeon";

/// Linux doubles the requested buffer value. Verify the resulting capacity,
/// because a host sysctl may clamp it without returning an error.
pub fn configure_socket_buffers(socket: &impl AsRawFd) -> io::Result<()> {
    let requested: libc::c_int = (2 * MAX_FRAME) as _;
    for option in [libc::SO_SNDBUF, libc::SO_RCVBUF] {
        let set = unsafe {
            libc::setsockopt(
                socket.as_raw_fd(),
                libc::SOL_SOCKET,
                option,
                (&requested as *const libc::c_int).cast(),
                std::mem::size_of_val(&requested) as _,
            )
        };
        if set < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut actual: libc::c_int = 0;
        let mut length: libc::socklen_t = std::mem::size_of_val(&actual) as _;
        let get = unsafe {
            libc::getsockopt(
                socket.as_raw_fd(),
                libc::SOL_SOCKET,
                option,
                (&mut actual as *mut libc::c_int).cast(),
                &mut length,
            )
        };
        if get < 0 {
            return Err(io::Error::last_os_error());
        }
        if actual < requested {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "kernel socket buffer too small for bounded browser commands",
            ));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "lowercase", deny_unknown_fields)]
pub enum Request {
    Bind {
        id: u64,
        lease: String,
        generation: u64,
    },
    Register {
        id: u64,
    },
    Send {
        id: u64,
        lease: String,
        generation: u64,
        command: Value,
    },
    Retire {
        id: u64,
        lease: String,
        generation: u64,
    },
    Settle {
        extension_id: u64,
        completed: bool,
    },
    Pong {
        payload: Vec<u8>,
    },
    Disconnect,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum Event {
    Result {
        id: u64,
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    Extension {
        message: Value,
    },
    Connected,
    Disconnected,
}
impl Event {
    pub fn result(id: u64, status: &str, reason: Option<&str>) -> Self {
        Self::Result {
            id,
            status: status.into(),
            reason: reason.map(str::to_owned),
        }
    }
}

pub fn server_frame(opcode: u8, payload: &[u8]) -> io::Result<Vec<u8>> {
    if payload.len() > MAX_FRAME
        || !matches!(opcode, 1 | 8 | 10)
        || (opcode != 1 && payload.len() > 125)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unbounded or unsupported frame",
        ));
    }
    let mut bytes = vec![0x80 | opcode];
    match payload.len() {
        n @ 0..=125 => bytes.push(n as u8),
        n @ 126..=65535 => {
            bytes.push(126);
            bytes.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            bytes.push(127);
            bytes.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    bytes.extend_from_slice(payload);
    Ok(bytes)
}

#[derive(Debug, PartialEq, Eq)]
pub enum WriteOutcome {
    Full,
    WouldBlock,
    Unknown,
}
pub fn write_frame(fd: &OwnedFd, bytes: &[u8]) -> WriteOutcome {
    let count = unsafe {
        libc::send(
            fd.as_raw_fd(),
            bytes.as_ptr().cast(),
            bytes.len(),
            libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL,
        )
    };
    classify_write(count, bytes.len(), io::Error::last_os_error().kind())
}
fn classify_write(count: isize, len: usize, error: io::ErrorKind) -> WriteOutcome {
    if count == len as isize {
        WriteOutcome::Full
    } else if count < 0
        && matches!(
            error,
            io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
        )
    {
        WriteOutcome::WouldBlock
    } else {
        WriteOutcome::Unknown
    }
}

struct Pending {
    id: u64,
    lease: String,
    generation: u64,
    extension_id: u64,
    frame: Vec<u8>,
}
struct InFlight {
    lease: String,
    generation: u64,
    extension_id: u64,
}
/// Only the native input actor calls dispatch. It supplies current human ownership
/// in the same synchronous turn as the write closure.
#[derive(Default)]
pub struct Gate {
    retired: BTreeMap<String, u64>,
    live: BTreeMap<String, u64>,
    pending: VecDeque<Pending>,
    controls: VecDeque<Vec<u8>>,
    in_flight: Option<InFlight>,
    disconnected: bool,
    uncertain: bool,
    seen: BTreeSet<u64>,
}
impl Gate {
    pub fn bind(&mut self, lease: String, generation: u64) -> Result<(), &'static str> {
        if self.disconnected || self.uncertain {
            return Err("channel unavailable or outcome unresolved");
        }
        if lease.is_empty() || lease.len() > 256 || generation == 0 {
            return Err("invalid lease");
        }
        if self
            .retired
            .get(&lease)
            .is_some_and(|old| generation <= *old)
        {
            return Err("retired generation");
        }
        if self.live.get(&lease).is_some_and(|old| *old != generation) {
            return Err("different live generation");
        }
        if !self.live.contains_key(&lease) && self.live.len() >= 128 {
            return Err("lease limit");
        }
        self.live.insert(lease, generation);
        Ok(())
    }
    pub fn enqueue(
        &mut self,
        id: u64,
        lease: String,
        generation: u64,
        command: Value,
    ) -> Result<(), &'static str> {
        if self.disconnected || self.uncertain {
            return Err("channel unavailable or outcome unresolved");
        }
        if lease.is_empty() || lease.len() > 256 || generation == 0 {
            return Err("invalid lease");
        }
        if self.retired.get(&lease).is_some_and(|g| generation <= *g) {
            return Err("retired generation");
        }
        if self.live.get(&lease) != Some(&generation) {
            return Err("lease was not bound before execution");
        }
        if self.pending.len() >= MAX_PENDING || self.live.len() >= 128 {
            return Err("queue or lease limit");
        }
        let extension_id = command
            .get("id")
            .and_then(Value::as_u64)
            .ok_or("missing command id")?;
        if self.seen.len() >= 65536 || self.seen.contains(&extension_id) {
            return Err("command id consumed or channel history full");
        }
        if !command.get("method").is_some_and(Value::is_string)
            || command.get("dojoLease").and_then(Value::as_str) != Some(&lease)
            || command.get("dojoGeneration").and_then(Value::as_u64) != Some(generation)
        {
            return Err("unbound command");
        }
        if self.pending.iter().any(|p| p.extension_id == extension_id)
            || self
                .in_flight
                .as_ref()
                .is_some_and(|p| p.extension_id == extension_id)
        {
            return Err("duplicate command id");
        }
        let payload = serde_json::to_vec(&command).map_err(|_| "invalid command")?;
        let frame = server_frame(1, &payload).map_err(|_| "oversized command")?;
        self.seen.insert(extension_id);
        self.pending.push_back(Pending {
            id,
            lease,
            generation,
            extension_id,
            frame,
        });
        Ok(())
    }
    pub fn retire(&mut self, lease: &str, generation: u64) -> Vec<Event> {
        if self
            .retired
            .get(lease)
            .is_some_and(|old| *old >= generation)
        {
            return Vec::new();
        }
        self.retired
            .entry(lease.into())
            .and_modify(|g| *g = (*g).max(generation))
            .or_insert(generation);
        if self.live.get(lease).is_some_and(|g| *g <= generation) {
            self.live.remove(lease);
        }
        let mut events = Vec::new();
        self.pending.retain(|p| {
            if p.lease == lease && p.generation <= generation {
                events.push(Event::result(
                    p.id,
                    "refused",
                    Some("lease retired before admission"),
                ));
                false
            } else {
                true
            }
        });
        if self
            .in_flight
            .as_ref()
            .is_some_and(|p| p.lease == lease && p.generation <= generation)
        {
            self.uncertain = true;
        }
        if self.controls.len() < MAX_PENDING {
            let payload = serde_json::to_vec(&serde_json::json!({"method":"dojo.retire","params":{"lease":lease,"generation":generation}})).unwrap();
            self.controls.push_back(server_frame(1, &payload).unwrap());
        } else {
            self.disconnected = true;
        }
        events
    }
    pub fn takeover(&mut self) -> Vec<Event> {
        let leases: Vec<_> = self.live.iter().map(|(l, g)| (l.clone(), *g)).collect();
        let mut events = Vec::new();
        for (lease, generation) in leases {
            events.extend(self.retire(&lease, generation));
        }
        events
    }
    pub fn settle(&mut self, extension_id: u64, completed: bool) -> Vec<Event> {
        if self
            .in_flight
            .as_ref()
            .is_some_and(|p| p.extension_id == extension_id)
        {
            self.in_flight = None;
            if !completed {
                self.uncertain = true;
                return self.takeover();
            }
        }
        Vec::new()
    }
    pub fn pong(&mut self, payload: Vec<u8>) -> Result<(), &'static str> {
        if self.controls.len() >= MAX_PENDING {
            return Err("control queue full");
        }
        self.controls
            .push_back(server_frame(10, &payload).map_err(|_| "invalid pong")?);
        Ok(())
    }
    pub fn disconnect(&mut self) -> Vec<Event> {
        self.disconnected = true;
        self.uncertain |= self.in_flight.is_some();
        self.controls.clear();
        self.pending
            .drain(..)
            .map(|p| Event::result(p.id, "refused", Some("channel disconnected")))
            .collect()
    }
    pub fn dispatch(
        &mut self,
        permitted: bool,
        mut send: impl FnMut(&[u8]) -> WriteOutcome,
    ) -> Vec<Event> {
        if self.disconnected {
            return Vec::new();
        }
        if let Some(frame) = self.controls.front() {
            match send(frame) {
                WriteOutcome::Full => {
                    self.controls.pop_front();
                }
                WriteOutcome::WouldBlock => {}
                WriteOutcome::Unknown => {
                    return self.disconnect();
                }
            }
            return Vec::new();
        }
        if !permitted {
            return self.takeover();
        }
        if self.in_flight.is_some() || self.uncertain {
            return Vec::new();
        }
        let Some(p) = self.pending.front() else {
            return Vec::new();
        };
        match send(&p.frame) {
            WriteOutcome::WouldBlock => Vec::new(),
            WriteOutcome::Full => {
                let p = self.pending.pop_front().unwrap();
                self.in_flight = Some(InFlight {
                    lease: p.lease,
                    generation: p.generation,
                    extension_id: p.extension_id,
                });
                vec![Event::result(p.id, "admitted", None)]
            }
            WriteOutcome::Unknown => {
                let p = self.pending.pop_front().unwrap();
                let mut events = vec![Event::result(
                    p.id,
                    "unknown",
                    Some("partial write or socket failure; never retry"),
                )];
                events.extend(self.disconnect());
                events
            }
        }
    }
    pub fn is_disconnected(&self) -> bool {
        self.disconnected
    }
    pub fn reconnect(&mut self) {
        self.takeover();
        self.in_flight = None;
        self.disconnected = false;
        self.uncertain = false;
        self.controls.clear();
        self.seen.clear();
    }
}

#[cfg(test)]
fn fresh_gate() -> Gate {
    let mut gate = Gate::default();
    gate.bind("task".into(), 1).unwrap();
    gate
}

#[cfg(test)]
mod tests {
    use super::*;
    fn command(id: u64) -> Value {
        serde_json::json!({"id":id,"method":"Runtime.evaluate","dojoLease":"task","dojoGeneration":1})
    }
    #[test]
    fn takeover_before_send_rejects_queued_work() {
        let mut g = fresh_gate();
        g.enqueue(1, "task".into(), 1, command(7)).unwrap();
        let events = g.dispatch(false, |_| panic!("human must preempt write"));
        assert!(matches!(&events[0],Event::Result{status,..} if status=="refused"));
        assert!(g.enqueue(2, "task".into(), 1, command(8)).is_err());
    }
    #[test]
    fn takeover_after_send_allows_no_follow_on_even_after_late_reply() {
        let mut g = fresh_gate();
        g.enqueue(1, "task".into(), 1, command(7)).unwrap();
        g.enqueue(2, "task".into(), 1, command(8)).unwrap();
        assert!(
            matches!(&g.dispatch(true, |_|WriteOutcome::Full)[0],Event::Result{status,..} if status=="admitted")
        );
        assert_eq!(g.takeover().len(), 1);
        g.settle(7, true);
        g.dispatch(true, |_| WriteOutcome::Full); // fixed tombstone, no app command
        assert!(g.dispatch(true, |_| panic!("no follow-on")).is_empty());
    }
    #[test]
    fn would_block_requires_fresh_native_ownership_check() {
        let mut g = fresh_gate();
        g.enqueue(1, "task".into(), 1, command(7)).unwrap();
        assert!(g.dispatch(true, |_| WriteOutcome::WouldBlock).is_empty());
        assert_eq!(
            g.dispatch(false, |_| panic!("no write after takeover"))
                .len(),
            1
        );
    }
    #[test]
    fn partial_write_retires_all_without_resending_remainder() {
        let mut g = fresh_gate();
        g.enqueue(1, "task".into(), 1, command(7)).unwrap();
        g.enqueue(2, "task".into(), 1, command(8)).unwrap();
        let events = g.dispatch(true, |_| classify_write(3, 100, io::ErrorKind::Other));
        assert!(matches!(&events[0],Event::Result{status,..} if status=="unknown"));
        assert_eq!(events.len(), 2);
        assert!(g.is_disconnected());
        assert!(g.dispatch(true, |_| panic!("never resend")).is_empty());
    }
    #[test]
    fn disconnect_rejects_queued_and_tombstone_cannot_be_reused() {
        let mut g = fresh_gate();
        g.enqueue(1, "task".into(), 1, command(7)).unwrap();
        assert_eq!(g.disconnect().len(), 1);
        assert!(g.enqueue(2, "task".into(), 1, command(8)).is_err());
    }
    #[test]
    fn server_frames_have_no_mask_or_fragmentation() {
        assert_eq!(server_frame(1, b"hi").unwrap(), b"\x81\x02hi");
    }
}

pub mod unix {
    use super::*;
    use std::os::fd::FromRawFd;
    use std::os::unix::net::UnixDatagram;
    use std::path::{Path, PathBuf};
    pub fn enable_credentials(socket: &UnixDatagram) -> io::Result<()> {
        let enabled: libc::c_int = 1;
        if unsafe {
            libc::setsockopt(
                socket.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PASSCRED,
                (&enabled as *const libc::c_int).cast(),
                std::mem::size_of_val(&enabled) as _,
            )
        } < 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    pub fn send(
        socket: &UnixDatagram,
        path: &Path,
        request: &Request,
        fd: Option<&OwnedFd>,
    ) -> io::Result<()> {
        send_packet(socket, path, request, fd)
    }
    pub fn send_packet<T: serde::Serialize>(
        socket: &UnixDatagram, path: &Path, request: &T, fd: Option<&OwnedFd>,
    ) -> io::Result<()> {
        let bytes = serde_json::to_vec(request)?;
        if bytes.len() > MAX_FRAME {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "packet too large",
            ));
        }
        socket.connect(path)?;
        let mut iov = libc::iovec {
            iov_base: bytes.as_ptr() as *mut _,
            iov_len: bytes.len(),
        };
        let mut control = [0usize; 8];
        let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        if let Some(fd) = fd {
            msg.msg_control = control.as_mut_ptr().cast();
            msg.msg_controllen =
                unsafe { libc::CMSG_SPACE(std::mem::size_of::<i32>() as _) as usize };
            unsafe {
                let header = libc::CMSG_FIRSTHDR(&msg);
                (*header).cmsg_level = libc::SOL_SOCKET;
                (*header).cmsg_type = libc::SCM_RIGHTS;
                (*header).cmsg_len = libc::CMSG_LEN(std::mem::size_of::<i32>() as _) as usize;
                (libc::CMSG_DATA(header) as *mut i32).write_unaligned(fd.as_raw_fd());
            }
        }
        let n = unsafe { libc::sendmsg(socket.as_raw_fd(), &msg, libc::MSG_NOSIGNAL) };
        if n != bytes.len() as isize {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    pub fn receive(
        socket: &UnixDatagram,
    ) -> io::Result<Option<(Request, Option<OwnedFd>, libc::ucred, PathBuf)>> {
        receive_packet(socket)
    }
    pub fn receive_packet<T: serde::de::DeserializeOwned>(
        socket: &UnixDatagram,
    ) -> io::Result<Option<(T, Option<OwnedFd>, libc::ucred, PathBuf)>> {
        let mut bytes = vec![0u8; MAX_FRAME];
        let mut control = [0usize; 32];
        let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
        let mut iov = libc::iovec {
            iov_base: bytes.as_mut_ptr().cast(),
            iov_len: bytes.len(),
        };
        let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
        msg.msg_name = (&mut address as *mut libc::sockaddr_un).cast();
        msg.msg_namelen = std::mem::size_of_val(&address) as _;
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = control.as_mut_ptr().cast();
        msg.msg_controllen = std::mem::size_of_val(&control);
        let n = unsafe {
            libc::recvmsg(
                socket.as_raw_fd(),
                &mut msg,
                libc::MSG_DONTWAIT | libc::MSG_CMSG_CLOEXEC,
            )
        };
        if n < 0 {
            let e = io::Error::last_os_error();
            return if e.kind() == io::ErrorKind::WouldBlock {
                Ok(None)
            } else {
                Err(e)
            };
        }
        let mut fds = Vec::new();
        let mut credentials = None;
        let mut valid = true;
        let mut header = unsafe { libc::CMSG_FIRSTHDR(&msg) };
        while !header.is_null() {
            unsafe {
                let h = &*header;
                if h.cmsg_level == libc::SOL_SOCKET && h.cmsg_type == libc::SCM_RIGHTS {
                    let len = h.cmsg_len.saturating_sub(libc::CMSG_LEN(0) as usize);
                    if len % std::mem::size_of::<i32>() != 0 {
                        valid = false;
                    }
                    for i in 0..len / std::mem::size_of::<i32>() {
                        let raw = (libc::CMSG_DATA(header) as *const i32)
                            .add(i)
                            .read_unaligned();
                        if raw >= 0 {
                            fds.push(OwnedFd::from_raw_fd(raw));
                        } else {
                            valid = false;
                        }
                    }
                } else if h.cmsg_level == libc::SOL_SOCKET
                    && h.cmsg_type == libc::SCM_CREDENTIALS
                    && h.cmsg_len
                        == libc::CMSG_LEN(std::mem::size_of::<libc::ucred>() as _) as usize
                {
                    if credentials.is_some() {
                        valid = false;
                    }
                    credentials =
                        Some((libc::CMSG_DATA(header) as *const libc::ucred).read_unaligned());
                } else {
                    valid = false;
                }
                header = libc::CMSG_NXTHDR(&msg, header);
            }
        }
        if !valid || fds.len() > 1 || msg.msg_flags & (libc::MSG_TRUNC | libc::MSG_CTRUNC) != 0 {
            return Ok(None);
        }
        let Some(credentials) = credentials else {
            return Ok(None);
        };
        let pathbytes: Vec<u8> = address
            .sun_path
            .iter()
            .take_while(|c| **c != 0)
            .map(|c| *c as u8)
            .collect();
        if pathbytes.is_empty() {
            return Ok(None);
        }
        use std::os::unix::ffi::OsStringExt;
        let request = serde_json::from_slice(&bytes[..n as usize])?;
        Ok(Some((
            request,
            fds.pop(),
            credentials,
            PathBuf::from(std::ffi::OsString::from_vec(pathbytes)),
        )))
    }
}

#[cfg(test)]
mod socket_tests {
    use super::*;
    use std::{
        io::Read,
        os::unix::net::{UnixDatagram, UnixStream},
    };
    #[test]
    fn real_kernel_frame_and_fd_transfer_have_one_commit_boundary() {
        let root = std::env::temp_dir().join(format!("dojo-browser-wire-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("sender");
        let destination = root.join("receiver");
        let sender = UnixDatagram::bind(&source).unwrap();
        let receiver = UnixDatagram::bind(&destination).unwrap();
        unix::enable_credentials(&receiver).unwrap();
        let (writer, mut reader) = UnixStream::pair().unwrap();
        reader.set_nonblocking(true).unwrap();
        let writer: OwnedFd = writer.into();
        unix::send(
            &sender,
            &destination,
            &Request::Register { id: 0 },
            Some(&writer),
        )
        .unwrap();
        drop(writer);
        let (_, fd, credentials, reply) = unix::receive(&receiver).unwrap().unwrap();
        assert_eq!(credentials.uid, unsafe { libc::geteuid() });
        assert_eq!(reply, source);
        let fd = fd.unwrap();
        let frame = server_frame(1, b"literal command").unwrap();
        assert_eq!(write_frame(&fd, &frame), WriteOutcome::Full);
        let mut bytes = [0u8; 128];
        let n = reader.read(&mut bytes).unwrap();
        assert_eq!(&bytes[..n], frame.as_slice());
        assert_eq!(
            reader.read(&mut bytes).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        drop(fd);
        drop(reader);
        drop(sender);
        drop(receiver);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn closed_kernel_socket_reports_unknown_without_replay() {
        let (writer, reader) = UnixStream::pair().unwrap();
        let writer: OwnedFd = writer.into();
        drop(reader);
        assert_eq!(
            write_frame(&writer, &server_frame(1, b"action").unwrap()),
            WriteOutcome::Unknown
        );
    }
    #[test]
    fn reconnect_keeps_exact_generation_tombstones() {
        let mut gate = fresh_gate();
        gate.enqueue(1,"task".into(),1,serde_json::json!({"id":7,"method":"Runtime.evaluate","dojoLease":"task","dojoGeneration":1})).unwrap();
        gate.dispatch(true, |_| WriteOutcome::Full);
        gate.disconnect();
        gate.reconnect();
        assert!(gate.enqueue(2,"task".into(),1,serde_json::json!({"id":8,"method":"Runtime.evaluate","dojoLease":"task","dojoGeneration":1})).is_err());
        gate.bind("task".into(), 2).unwrap();
        gate.enqueue(3,"task".into(),2,serde_json::json!({"id":9,"method":"Runtime.evaluate","dojoLease":"task","dojoGeneration":2})).unwrap();
    }
}

#[cfg(test)]
mod partial_socket_test {
    use super::*;
    use std::{io::Read, os::unix::net::UnixStream};
    #[test]
    fn real_partial_kernel_write_becomes_unknown_and_never_replays() {
        let (writer, mut reader) = UnixStream::pair().unwrap();
        let writer: OwnedFd = writer.into();
        let size: libc::c_int = 4096;
        assert_eq!(
            unsafe {
                libc::setsockopt(
                    writer.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_SNDBUF,
                    (&size as *const libc::c_int).cast(),
                    std::mem::size_of_val(&size) as _,
                )
            },
            0
        );
        reader.set_nonblocking(true).unwrap();
        let mut gate = fresh_gate();
        let command = serde_json::json!({"id":7,"method":"Runtime.evaluate","dojoLease":"task","dojoGeneration":1,"params":{"expression":"x".repeat(60000)}});
        gate.enqueue(1, "task".into(), 1, command).unwrap();
        let events = gate.dispatch(true, |frame| write_frame(&writer, frame));
        assert!(matches!(&events[0],Event::Result{status,..} if status=="unknown"));
        let mut bytes = vec![0; 65536];
        let received = reader.read(&mut bytes).unwrap();
        assert!(received > 0 && received < 60000);
        assert!(
            gate.dispatch(true, |_| panic!("never resend remainder"))
                .is_empty()
        );
        assert_eq!(
            reader.read(&mut bytes).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
    }
}

#[cfg(test)]
mod lease_binding_tests {
    use super::*;
    #[test]
    fn takeover_retires_a_bound_script_before_its_first_command() {
        let mut gate = Gate::default();
        gate.bind("task".into(), 1).unwrap();
        gate.bind("task".into(), 1).unwrap();
        assert!(gate.takeover().is_empty());
        assert!(gate.enqueue(1,"task".into(),1,serde_json::json!({"id":7,"method":"Runtime.evaluate","dojoLease":"task","dojoGeneration":1})).is_err());
        assert!(gate.bind("task".into(), 1).is_err());
    }
    #[test]
    fn command_cannot_create_its_own_native_lease() {
        let mut gate = Gate::default();
        assert!(gate.enqueue(1,"task".into(),1,serde_json::json!({"id":7,"method":"Runtime.evaluate","dojoLease":"task","dojoGeneration":1})).is_err());
    }
}

#[cfg(test)]
mod large_payload_tests {
    use super::*;
    use std::{
        io::Read,
        net::{TcpListener, TcpStream},
        os::unix::net::UnixDatagram,
    };
    #[test]
    fn playwright_helper_over_64k_crosses_real_unix_datagram_and_tcp_frame() {
        let root =
            std::env::temp_dir().join(format!("dojo-browser-large-wire-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let sender = UnixDatagram::bind(root.join("sender")).unwrap();
        let receiver = UnixDatagram::bind(root.join("receiver")).unwrap();
        unix::enable_credentials(&receiver).unwrap();
        configure_socket_buffers(&sender).unwrap();
        configure_socket_buffers(&receiver).unwrap();
        let command = serde_json::json!({"id":7,"method":"Runtime.evaluate","dojoLease":"task","dojoGeneration":1,"params":{"expression":"x".repeat(400000)}});
        let request = Request::Send {
            id: 1,
            lease: "task".into(),
            generation: 1,
            command: command.clone(),
        };
        unix::send(&sender, &root.join("receiver"), &request, None).unwrap();
        let (received, fd, _, _) = unix::receive(&receiver).unwrap().unwrap();
        assert!(fd.is_none());
        assert!(matches!(received,Request::Send{command:observed,..} if observed==command));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let writer: OwnedFd = listener.accept().unwrap().0.into();
        configure_socket_buffers(&writer).unwrap();
        let payload = serde_json::to_vec(&command).unwrap();
        assert!(payload.len() > 65536);
        let frame = server_frame(1, &payload).unwrap();
        assert_eq!(write_frame(&writer, &frame), WriteOutcome::Full);
        let mut bytes = vec![0u8; frame.len()];
        peer.read_exact(&mut bytes).unwrap();
        assert_eq!(bytes, frame);
        drop(sender);
        drop(receiver);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn payload_above_one_mib_is_refused_before_transport() {
        assert!(server_frame(1, &vec![b'x'; 1048577]).is_err());
    }
}
