//! Optional single-use CDP final writer. Every send runs on Niri's event thread.
//! An operator-pinned trusted broker performs the HTTP upgrade and exact CDP
//! target/window correlation, relinquishes every writable clone, and transfers
//! the sole cleartext loopback writer. Unknown outcomes never permit follow-ons.
use crate::niri::State;
mod registration;
use anyhow::{ensure, Context};
use calloop::{
    generic::Generic,
    timer::{TimeoutAction, Timer},
    Interest, LoopHandle, Mode, PostAction,
};
use serde::{Deserialize, Serialize};
use std::{
    cell::Cell,
    io,
    net::TcpStream,
    os::{
        fd::{AsFd, AsRawFd, FromRawFd, OwnedFd},
        unix::{fs::PermissionsExt, net::UnixDatagram},
    },
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct TargetKey {
    window_id: u64,
    pid: u32,
    process_start: u64,
    compositor_pid: u32,
    compositor_start: u64,
}
impl TargetKey {
    fn of(request: &Navigate) -> Self {
        Self {
            window_id: request.window_id,
            pid: request.browser_pid,
            process_start: request.browser_start_time,
            compositor_pid: request.capture_geometry.compositor_pid,
            compositor_start: request.capture_geometry.compositor_start_time,
        }
    }
}
impl State {
    pub(crate) fn external_target_unresolved(&self, window_id: u64) -> bool {
        self.niri
            .external_admission_unknown
            .iter()
            .any(|key| key.window_id == window_id)
    }
}

const MAX_PACKET: usize = 8192;
const MAX_PENDING: usize = 16;
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Navigate {
    version: u32,
    peer: u64,
    revision: u64,
    window_id: u64,
    browser_pid: u32,
    browser_start_time: u64,
    /// Exact page target proven by the trusted broker during upgrade. Not inferred
    /// from PID alone. General browser/session sockets are intentionally refused.
    page_target: String,
    request_id: u64,
    url: String,
    /// Durable consent/intent identifiers are attestations by the trusted broker.
    /// They do not replace live compositor priority admission.
    intent_id: String,
    grant_id: String,
    lifetime_ms: u32,
    capture_geometry: niri_ipc::CapturedInputGeometry,
}
#[derive(Debug, Serialize)]
struct Receipt<'a> {
    request_id: u64,
    intent_id: &'a str,
    effect: &'a str,
    accepted_bytes: usize,
    message: &'a str,
}
struct SocketPath(PathBuf);
impl Drop for SocketPath {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
struct Writer {
    owner: Option<Rc<registration::RegisteredChild>>,
    socket: TcpStream,
    request: Navigate,
    frame: Vec<u8>,
    deadline: Instant,
    reply: Option<PathBuf>,
    channel: Rc<UnixDatagram>,
    pending: Rc<Cell<usize>>,
    reported: Cell<bool>,
}
impl AsFd for Writer {
    fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> {
        self.socket.as_fd()
    }
}
impl Drop for Writer {
    fn drop(&mut self) {
        self.pending.set(self.pending.get().saturating_sub(1));
        if !self.reported.get() {
            self.report("refused", 0, "writer retired before admission");
        }
    }
}
impl Writer {
    fn report(&self, effect: &str, accepted: usize, message: &str) {
        self.reported.set(true);
        let response = Receipt {
            request_id: self.request.request_id,
            intent_id: &self.request.intent_id,
            effect,
            accepted_bytes: accepted,
            message,
        };
        if let (Some(path), Ok(bytes)) = (&self.reply, serde_json::to_vec(&response)) {
            let _ = self.channel.send_to(&bytes, path);
        }
    }
    fn ready(&self, state: &mut State) -> PostAction {
        if self.reported.get() {
            return PostAction::Remove;
        }
        if self.owner.as_ref().is_some_and(|owner| {
            let (pid, start) = owner.process_identity();
            !owner.live() || !state.agent_peer_owned_by(self.request.peer, pid, start)
        }) {
            self.report("refused", 0, "registered child or its native peer retired");
            return PostAction::Remove;
        }
        if Instant::now() >= self.deadline {
            self.report("refused", 0, "expired intent");
            return PostAction::Remove;
        }
        if state.external_target_unresolved(self.request.window_id) {
            self.report(
                "refused",
                0,
                "unresolved external effect blocks this exact window",
            );
            return PostAction::Remove;
        }
        let authority = state.resolve_external_target(
            self.request.peer,
            self.request.revision,
            self.request.window_id,
        );
        let admitted = state
            .require_capture_geometry(&self.request.capture_geometry)
            .is_ok()
            && authority.is_ok_and(|(_, pid)| pid == self.request.browser_pid)
            && process_start(self.request.browser_pid).ok()
                == Some(self.request.browser_start_time);
        if !admitted {
            self.report(
                "refused",
                0,
                "human takeover, target retirement or process identity changed",
            );
            return PostAction::Remove;
        }
        // No await, lock, library buffer or transferable permit between the final
        // authority check and this one kernel write. Human events share this actor.
        let count = unsafe {
            libc::send(
                self.socket.as_raw_fd(),
                self.frame.as_ptr().cast(),
                self.frame.len(),
                libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL,
            )
        };
        if count < 0 {
            let error = io::Error::last_os_error();
            if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
            ) {
                return PostAction::Continue;
            }
            self.report(
                "unknown",
                0,
                "socket failed at dispatch; recover and re-observe",
            );
        } else if count == 0 {
            self.report("unknown", 0, "connection closed at dispatch");
        } else if count as usize == self.frame.len() {
            self.report(
                "unknown",
                count as usize,
                "admitted; application effect unconfirmed; fresh observation required",
            );
        } else {
            self.report(
                "unknown",
                count as usize,
                "partial frame; connection closed; never resend remainder",
            );
        }
        state
            .niri
            .external_admission_unknown
            .insert(TargetKey::of(&self.request));
        let _ = self.socket.shutdown(std::net::Shutdown::Write);
        PostAction::Remove
    }
}

/// Install only after the operator pins the trusted broker PID and start time.
/// This route remains opt-in and must not be advertised as a general CDP gate.
pub fn start(
    loop_handle: &LoopHandle<'static, State>,
    path: PathBuf,
    broker_pid: u32,
    broker_start: u64,
) -> anyhow::Result<()> {
    ensure!(
        broker_pid > 0 && process_start(broker_pid)? == broker_start,
        "broker identity mismatch"
    );
    start_inner(loop_handle, path, broker_pid, broker_start, None)
}

/// The operator selects a live launch attester and immutable recipe. Only its
/// enrolled child can submit writers, bound to that child's native IPC peers.
pub fn start_with_launcher(
    loop_handle: &LoopHandle<'static, State>,
    path: PathBuf,
    launcher_pid: u32,
    launcher_start: u64,
    recipe_sha256: String,
) -> anyhow::Result<()> {
    let launcher =
        registration::LauncherEnrollment::open(launcher_pid, launcher_start, recipe_sha256)?;
    start_inner(
        loop_handle,
        path,
        launcher_pid,
        launcher_start,
        Some(launcher),
    )
}

fn start_inner(
    loop_handle: &LoopHandle<'static, State>,
    path: PathBuf,
    broker_pid: u32,
    broker_start: u64,
    launcher: Option<registration::LauncherEnrollment>,
) -> anyhow::Result<()> {
    let child = Rc::new(std::cell::RefCell::new(
        None::<Rc<registration::RegisteredChild>>,
    ));
    let channel = UnixDatagram::bind(&path).context("bind external admission socket")?;
    let cleanup = SocketPath(path.clone());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    channel.set_nonblocking(true)?;
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
        "enable credential delivery"
    );
    let channel = Rc::new(channel);
    let receiver = channel.try_clone()?;
    let pending = Rc::new(Cell::new(0));
    let seen = Rc::new(std::cell::RefCell::new(
        std::collections::HashSet::<String>::new(),
    ));
    let handle = loop_handle.clone();
    loop_handle
        .insert_source(
            Generic::new(receiver, Interest::READ, Mode::Level),
            move |_, socket, state| {
                let _keep_path = &cleanup;
                // One bounded packet per actor turn keeps native human input responsive.
                let packet = receive_descriptors(socket.as_raw_fd(), if launcher.is_some() { 2 } else { 1 });
                let Ok(Some((raw, mut descriptors, credentials, reply))) = packet else {
                    return Ok(PostAction::Continue);
                };
                let owner = if let Some(launcher) = &launcher {
                    if descriptors.len() == 2 {
                        // A new generation cannot replace a live child. A dead
                        // generation's outstanding writers still hold its handle
                        // and will refuse before attempting any kernel send.
                        if child.borrow().as_ref().is_some_and(|owner| owner.live()) {
                            return Ok(PostAction::Continue);
                        }
                        let Ok(claim) = serde_json::from_slice::<registration::ChildClaim>(&raw) else {
                            return Ok(PostAction::Continue);
                        };
                        let control = descriptors.pop().unwrap();
                        let pidfd = descriptors.pop().unwrap();
                        if let Ok(registered) = launcher.accept(credentials, claim, pidfd, control) {
                            let registered = Rc::new(registered);
                            let (pid, start) = registered.process_identity();
                            if let Some(old) = child.borrow_mut().take() {
                                old.revoke();
                            }
                            *child.borrow_mut() = Some(registered);
                            if let Some(path) = &reply {
                                if let Ok(bytes) = serde_json::to_vec(&serde_json::json!({
                                    "version":1,"child_pid":pid,"child_start":start,
                                    "actor_pid":std::process::id(),"enrolled":true
                                })) { let _ = channel.send_to(&bytes, path); }
                            }
                        }
                        return Ok(PostAction::Continue);
                    }
                    let Some(owner) = child.borrow().as_ref().filter(|owner| owner.matches(credentials)).cloned() else {
                        return Ok(PostAction::Continue);
                    };
                    Some(owner)
                } else {
                    if credentials.pid <= 0
                    || credentials.pid as u32 != broker_pid
                    || credentials.uid != unsafe { libc::geteuid() }
                    || process_start(broker_pid).ok() != Some(broker_start)
                    { return Ok(PostAction::Continue); }
                    None
                };
                let fd = descriptors.pop();
                let Some(fd) = fd else {
                    #[derive(Deserialize)]
                    #[serde(deny_unknown_fields)]
                    struct Probe { version:u32, nonce:String }
                    if let Ok(probe) = serde_json::from_slice::<Probe>(&raw) {
                        if probe.version == 1 && !probe.nonce.is_empty() && probe.nonce.len() <= 128 {
                            if let Some(path) = &reply {
                                if let Ok(bytes) = serde_json::to_vec(&serde_json::json!({"nonce":probe.nonce,"actor_pid":std::process::id()})) { let _ = channel.send_to(&bytes, path); }
                            }
                        }
                    }
                    return Ok(PostAction::Continue);
                };
                let request: Navigate = match serde_json::from_slice(&raw) {
                    Ok(request) => request,
                    Err(_) => return Ok(PostAction::Continue),
                };
                let refuse = |message: &str| {
                    if let Some(path) = &reply {
                        let receipt = Receipt {
                            request_id: request.request_id,
                            intent_id: &request.intent_id,
                            effect: "refused",
                            accepted_bytes: 0,
                            message,
                        };
                        if let Ok(bytes) = serde_json::to_vec(&receipt) {
                            let _ = channel.send_to(&bytes, path);
                        }
                    }
                };
                if pending.get() >= MAX_PENDING || validate_request(&request).is_err() {
                    refuse("invalid or over-budget registration");
                    return Ok(PostAction::Continue);
                }
                if owner.as_ref().is_some_and(|owner| {
                    let (pid, start) = owner.process_identity();
                    !state.agent_peer_owned_by(request.peer, pid, start)
                }) {
                    refuse("native peer does not belong to the registered child");
                    return Ok(PostAction::Continue);
                }
                if state.external_target_unresolved(request.window_id) {
                    refuse("unresolved external effect blocks follow-on mutations in this seat");
                    return Ok(PostAction::Continue);
                }
                // Read authoritative credentials on the event thread before accepting FD.
                if !state
                    .resolve_external_target(request.peer, request.revision, request.window_id)
                    .is_ok_and(|(_, pid)| pid == request.browser_pid)
                {
                    refuse("target retired or human ownership changed before registration");
                    return Ok(PostAction::Continue);
                }
                let socket = TcpStream::from(fd);
                if verify_endpoint(&socket, request.browser_pid, request.browser_start_time)
                    .is_err()
                {
                    refuse("endpoint ownership unproved");
                    return Ok(PostAction::Continue);
                }
                let frame = match navigate_frame(&request) {
                    Ok(frame) => frame,
                    Err(_) => return Ok(PostAction::Continue),
                };
                // Burn an immutable intent at registration. A failed/expired/partial
                // attempt never authorizes resubmission under the same durable ID.
                if seen.borrow().len() >= 4096
                    || !seen.borrow_mut().insert(request.intent_id.clone())
                {
                    if let Some(path) = &reply {
                        let response = Receipt {
                            request_id: request.request_id,
                            intent_id: &request.intent_id,
                            effect: "refused",
                            accepted_bytes: 0,
                            message: "intent already used or admission history full",
                        };
                        if let Ok(bytes) = serde_json::to_vec(&response) {
                            let _ = channel.send_to(&bytes, path);
                        }
                    }
                    return Ok(PostAction::Continue);
                }
                let deadline =
                    Instant::now() + Duration::from_millis(u64::from(request.lifetime_ms));
                pending.set(pending.get() + 1);
                let writer = Writer {
                    owner,
                    socket,
                    request,
                    frame,
                    deadline,
                    reply,
                    channel: channel.clone(),
                    pending: pending.clone(),
                    reported: Cell::new(false),
                };
                let token = match handle.insert_source(
                    Generic::new(writer, Interest::WRITE, Mode::Level),
                    |_, writer, state| Ok(writer.ready(state)),
                ) {
                    Ok(token) => token,
                    Err(_) => return Ok(PostAction::Continue),
                };
                let retire = handle.clone();
                if handle
                    .insert_source(Timer::from_deadline(deadline), move |_, _, _| {
                        retire.remove(token);
                        TimeoutAction::Drop
                    })
                    .is_err()
                {
                    handle.remove(token);
                }
                Ok(PostAction::Continue)
            },
        )
        .map_err(|e| anyhow::anyhow!("install external admission source: {e}"))?;
    Ok(())
}
fn validate_request(r: &Navigate) -> anyhow::Result<()> {
    ensure!(
        r.version == 1
            && r.peer != 0
            && r.window_id != 0
            && r.browser_pid > 0
            && r.browser_start_time > 0
            && r.request_id > 0,
        "invalid identity/version"
    );
    ensure!((1..=5000).contains(&r.lifetime_ms), "invalid deadline");
    ensure!(
        r.capture_geometry.window_id == r.window_id
            && r.capture_geometry.pid == r.browser_pid
            && r.capture_geometry.coordination_revision == r.revision,
        "capture authority mismatch"
    );
    ensure!(
        !r.page_target.is_empty()
            && r.page_target.len() <= 128
            && r.page_target
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-'),
        "exact page target required"
    );
    ensure!(
        !r.intent_id.is_empty()
            && r.intent_id.len() <= 256
            && !r.grant_id.is_empty()
            && r.grant_id.len() <= 256,
        "intent and capture grant required"
    );
    ensure!(
        r.url.len() <= 4096
            && (r.url.starts_with("https://")
                || r.url.starts_with("http://")
                || r.url == "about:blank")
            && !r.url.chars().any(char::is_control),
        "invalid navigation URL"
    );
    Ok(())
}
fn navigate_frame(r: &Navigate) -> anyhow::Result<Vec<u8>> {
    validate_request(r)?;
    let payload = serde_json::to_vec(
        &serde_json::json!({"id":r.request_id,"method":"Page.navigate","params":{"url":r.url}}),
    )?;
    let mut mask = [0u8; 4];
    // Fresh mask from the OS. No state shared with another websocket writer.
    ensure!(
        unsafe { libc::getrandom(mask.as_mut_ptr().cast(), mask.len(), libc::GRND_NONBLOCK) } == 4,
        "websocket masking entropy unavailable"
    );
    let mut frame = vec![0x81];
    if payload.len() < 126 {
        frame.push(0x80 | payload.len() as u8);
    } else {
        frame.push(0x80 | 126);
        frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    }
    frame.extend_from_slice(&mask);
    frame.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    Ok(frame)
}
pub(crate) fn process_start(pid: u32) -> anyhow::Result<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let (_, fields) = stat.rsplit_once(')').context("invalid process stat")?;
    let start = fields
        .split_whitespace()
        .nth(19)
        .context("missing process start")?
        .parse()?;
    Ok(start)
}
fn verify_endpoint(socket: &TcpStream, pid: u32, start: u64) -> anyhow::Result<()> {
    ensure!(process_start(pid)? == start, "browser generation changed");
    let mut socket_type: libc::c_int = 0;
    let mut size = std::mem::size_of_val(&socket_type) as libc::socklen_t;
    ensure!(
        unsafe {
            libc::getsockopt(
                socket.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_TYPE,
                (&mut socket_type as *mut libc::c_int).cast(),
                &mut size,
            )
        } == 0
            && socket_type == libc::SOCK_STREAM,
        "writer FD is not a stream socket"
    );
    let mut protocol: libc::c_int = 0;
    size = std::mem::size_of_val(&protocol) as libc::socklen_t;
    ensure!(
        unsafe {
            libc::getsockopt(
                socket.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PROTOCOL,
                (&mut protocol as *mut libc::c_int).cast(),
                &mut size,
            )
        } == 0
            && protocol == libc::IPPROTO_TCP,
        "writer FD is not TCP"
    );
    let peer = socket.peer_addr()?;
    let local = socket.local_addr()?;
    ensure!(
        peer.ip().is_loopback() && local.ip().is_loopback() && peer.is_ipv4(),
        "only cleartext IPv4 loopback endpoints supported"
    );
    socket.set_nonblocking(true)?;
    // Linux proc inode proves this exact listening endpoint is held by the
    // authoritative browser process, not another process on a caller's port.
    use std::io::Read as _;
    let mut table = String::new();
    std::fs::File::open("/proc/net/tcp")?
        .take(1_048_577)
        .read_to_string(&mut table)?;
    ensure!(
        table.len() <= 1_048_576,
        "socket attestation table exceeds actor budget"
    );
    let mut inodes = Vec::new();
    for row in table.lines().skip(1) {
        let fields = row.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 10 || fields[3] != "0A" {
            continue;
        }
        let Some((ip, port)) = fields[1].split_once(':') else {
            continue;
        };
        if ip == "0100007F" && u16::from_str_radix(port, 16).ok() == Some(peer.port()) {
            inodes.push(fields[9].to_owned());
        }
    }
    ensure!(inodes.len() == 1, "endpoint listener identity ambiguous");
    let expected = format!("socket:[{}]", inodes[0]);
    let mut owns = false;
    for entry in std::fs::read_dir(format!("/proc/{pid}/fd"))?.take(4096) {
        let entry = entry?;
        if std::fs::read_link(entry.path())
            .ok()
            .is_some_and(|p| p == Path::new(&expected))
        {
            owns = true;
            break;
        }
    }
    ensure!(
        owns && process_start(pid)? == start,
        "endpoint is not owned by exact browser process"
    );
    Ok(())
}
#[cfg(test)]
fn receive(
    fd: i32,
) -> io::Result<Option<(Vec<u8>, Option<OwnedFd>, libc::ucred, Option<PathBuf>)>> {
    Ok(
        receive_descriptors(fd, 1)?.map(|(bytes, mut descriptors, credentials, reply)| {
            (bytes, descriptors.pop(), credentials, reply)
        }),
    )
}
fn receive_descriptors(
    fd: i32,
    maximum: usize,
) -> io::Result<Option<(Vec<u8>, Vec<OwnedFd>, libc::ucred, Option<PathBuf>)>> {
    let mut bytes = [0u8; MAX_PACKET];
    // cmsg headers require native alignment; usize storage provides it.
    let mut control = [0usize; 32];
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    let mut iov = libc::iovec {
        iov_base: bytes.as_mut_ptr().cast(),
        iov_len: bytes.len(),
    };
    let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
    message.msg_name = (&mut address as *mut libc::sockaddr_un).cast();
    message.msg_namelen = std::mem::size_of_val(&address) as _;
    message.msg_iov = &mut iov;
    message.msg_iovlen = 1;
    message.msg_control = control.as_mut_ptr().cast();
    message.msg_controllen = std::mem::size_of_val(&control);
    let count = unsafe {
        libc::recvmsg(
            fd,
            &mut message,
            libc::MSG_DONTWAIT | libc::MSG_CMSG_CLOEXEC,
        )
    };
    if count < 0 {
        let error = io::Error::last_os_error();
        return if error.kind() == io::ErrorKind::WouldBlock {
            Ok(None)
        } else {
            Err(error)
        };
    }
    let mut descriptors = Vec::new();
    let mut credentials = None;
    let mut invalid = false;
    let mut header = unsafe { libc::CMSG_FIRSTHDR(&message) };
    while !header.is_null() {
        let h = unsafe { &*header };
        if h.cmsg_level == libc::SOL_SOCKET && h.cmsg_type == libc::SCM_RIGHTS {
            let size = h
                .cmsg_len
                .saturating_sub(unsafe { libc::CMSG_LEN(0) } as usize);
            if size % std::mem::size_of::<i32>() != 0 {
                invalid = true;
            }
            for i in 0..size / std::mem::size_of::<i32>() {
                let raw = unsafe {
                    (libc::CMSG_DATA(header) as *const i32)
                        .add(i)
                        .read_unaligned()
                };
                if raw >= 0 {
                    descriptors.push(unsafe { OwnedFd::from_raw_fd(raw) });
                } else {
                    invalid = true;
                }
            }
        } else if h.cmsg_level == libc::SOL_SOCKET
            && h.cmsg_type == libc::SCM_CREDENTIALS
            && h.cmsg_len
                == unsafe { libc::CMSG_LEN(std::mem::size_of::<libc::ucred>() as _) } as usize
        {
            if credentials.is_some() {
                invalid = true;
            }
            credentials =
                Some(unsafe { (libc::CMSG_DATA(header) as *const libc::ucred).read_unaligned() });
        } else {
            invalid = true;
        }
        header = unsafe { libc::CMSG_NXTHDR(&message, header) };
    }
    if invalid
        || descriptors.len() > maximum
        || credentials.is_none()
        || message.msg_flags & (libc::MSG_TRUNC | libc::MSG_CTRUNC) != 0
    {
        return Ok(None);
    }
    let pathbytes = address
        .sun_path
        .iter()
        .take_while(|c| **c != 0)
        .map(|c| *c as u8)
        .collect::<Vec<_>>();
    use std::os::unix::ffi::OsStringExt;
    let reply = if pathbytes.is_empty() {
        None
    } else {
        Some(PathBuf::from(std::ffi::OsString::from_vec(pathbytes)))
    };
    Ok(Some((
        bytes[..count as usize].to_vec(),
        descriptors,
        credentials.unwrap(),
        reply,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> Navigate {
        Navigate {
            version: 1,
            peer: 1,
            revision: 1,
            window_id: 7,
            browser_pid: std::process::id(),
            browser_start_time: process_start(std::process::id()).unwrap(),
            page_target: "PAGE-1".into(),
            request_id: 9,
            url: "https://example.invalid/test".into(),
            intent_id: "pi-intent-1".into(),
            grant_id: "pi-approval-1".into(),
            lifetime_ms: 500,
            capture_geometry: niri_ipc::CapturedInputGeometry {
                window_id: 7,
                pid: std::process::id(),
                compositor_pid: std::process::id(),
                compositor_start_time: process_start(std::process::id()).unwrap(),
                coordination_revision: 1,
                output: "fixture".into(),
                output_geometry: niri_ipc::LogicalOutput {
                    x: 0,
                    y: 0,
                    width: 800,
                    height: 600,
                    scale: 1.,
                    transform: niri_ipc::Transform::Normal,
                },
                rect: (0., 0., 800., 600.),
            },
        }
    }
    #[test]
    fn actor_builds_only_masked_navigation_and_preserves_id() {
        let frame = navigate_frame(&request()).unwrap();
        assert_eq!(frame[0], 0x81);
        assert_ne!(frame[1] & 0x80, 0);
        let (length, offset) = if frame[1] & 0x7f == 126 {
            (u16::from_be_bytes([frame[2], frame[3]]) as usize, 4)
        } else {
            ((frame[1] & 0x7f) as usize, 2)
        };
        let mask = &frame[offset..offset + 4];
        let body = frame[offset + 4..]
            .iter()
            .enumerate()
            .map(|(i, b)| b ^ mask[i % 4])
            .collect::<Vec<_>>();
        assert_eq!(body.len(), length);
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            value,
            serde_json::json!({"id":9,"method":"Page.navigate","params":{"url":"https://example.invalid/test"}})
        );
        let mut r = request();
        r.url = "javascript:evil()".into();
        assert!(navigate_frame(&r).is_err());
        r = request();
        r.page_target = "browser/root".into();
        assert!(validate_request(&r).is_err());
        r = request();
        r.grant_id.clear();
        assert!(validate_request(&r).is_err());
        r = request();
        r.lifetime_ms = 5001;
        assert!(validate_request(&r).is_err());
    }
    #[test]
    fn real_listener_inode_and_process_generation_are_required() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let socket = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let _peer = listener.accept().unwrap().0;
        let start = process_start(std::process::id()).unwrap();
        verify_endpoint(&socket, std::process::id(), start).unwrap();
        assert!(verify_endpoint(&socket, std::process::id(), start + 1).is_err());
        assert!(verify_endpoint(&socket, 1, process_start(1).unwrap_or(0)).is_err());
    }
    #[test]
    fn receive_attests_real_credentials_and_one_fd_and_rejects_plain_packet() {
        let (sender, receiver) = UnixDatagram::pair().unwrap();
        let enabled: libc::c_int = 1;
        assert_eq!(
            unsafe {
                libc::setsockopt(
                    receiver.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_PASSCRED,
                    (&enabled as *const libc::c_int).cast(),
                    std::mem::size_of_val(&enabled) as _,
                )
            },
            0
        );
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let transfer = listener.as_raw_fd();
        let payload = b"bounded packet";
        let mut iov = libc::iovec {
            iov_base: payload.as_ptr().cast_mut().cast(),
            iov_len: payload.len(),
        };
        let mut control = [0usize; 8];
        let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
        message.msg_iov = &mut iov;
        message.msg_iovlen = 1;
        message.msg_control = control.as_mut_ptr().cast();
        message.msg_controllen =
            unsafe { libc::CMSG_SPACE(std::mem::size_of_val(&transfer) as _) } as usize;
        unsafe {
            let header = libc::CMSG_FIRSTHDR(&message);
            (*header).cmsg_level = libc::SOL_SOCKET;
            (*header).cmsg_type = libc::SCM_RIGHTS;
            (*header).cmsg_len = libc::CMSG_LEN(std::mem::size_of_val(&transfer) as _) as usize;
            (libc::CMSG_DATA(header) as *mut i32).write_unaligned(transfer);
        }
        assert_eq!(
            unsafe { libc::sendmsg(sender.as_raw_fd(), &message, libc::MSG_NOSIGNAL) },
            payload.len() as isize
        );
        let (bytes, fd, credentials, reply) = receive(receiver.as_raw_fd()).unwrap().unwrap();
        assert_eq!(bytes, payload);
        assert_eq!(credentials.pid, std::process::id() as i32);
        assert!(reply.is_none());
        assert!(
            unsafe { libc::fcntl(fd.unwrap().as_raw_fd(), libc::F_GETFD) } & libc::FD_CLOEXEC != 0
        );
        sender.send(b"no descriptor").unwrap();
        assert!(receive(receiver.as_raw_fd()).unwrap().unwrap().1.is_none());
    }
    pub(super) fn fixture_window(
        f: &mut crate::tests::fixture::Fixture,
    ) -> (
        crate::tests::client::ClientId,
        wayland_client::protocol::wl_surface::WlSurface,
        u64,
    ) {
        let client = f.add_client();
        let window = f.client(client).create_window();
        let surface = window.surface.clone();
        window.commit();
        f.double_roundtrip(client);
        let window = f.client(client).window(&surface);
        window.attach_new_buffer();
        window.ack_last_and_commit();
        f.double_roundtrip(client);
        f.niri_complete_animations();
        let id = f.niri().layout.focus().unwrap().id().get();
        (client, surface, id)
    }
    pub(super) fn fixture_writer(
        f: &mut crate::tests::fixture::Fixture,
        target: u64,
    ) -> (Writer, TcpStream) {
        f.niri_state()
            .apply_agent_input(
                1,
                niri_ipc::AgentInput::Create {
                    name: "external-test".into(),
                },
            )
            .unwrap();
        f.niri_state().yield_human_input().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let socket = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let peer = listener.accept().unwrap().0;
        peer.set_nonblocking(true).unwrap();
        let mut r = request();
        r.window_id = target;
        r.revision = f.niri().input_coordination.revision;
        r.lifetime_ms = 1000;
        let output = f.niri().global_space.outputs().next().unwrap().clone();
        let output_geometry = crate::utils::logical_output(&output);
        let placements = f.niri().layout.capture_placements(&output).unwrap();
        let (_, rect, _) = placements
            .into_iter()
            .find(|(w, _, _)| w.id().get() == target)
            .unwrap();
        r.capture_geometry = niri_ipc::CapturedInputGeometry {
            window_id: target,
            pid: std::process::id(),
            compositor_pid: std::process::id(),
            compositor_start_time: process_start(std::process::id()).unwrap(),
            coordination_revision: r.revision,
            output: output.name(),
            output_geometry,
            rect: (rect.loc.x, rect.loc.y, rect.size.w, rect.size.h),
        };
        let frame = navigate_frame(&r).unwrap();
        let (channel, _) = UnixDatagram::pair().unwrap();
        (
            Writer {
                owner: None,
                socket,
                request: r,
                frame,
                deadline: Instant::now() + Duration::from_secs(1),
                reply: None,
                channel: Rc::new(channel),
                pending: Rc::new(Cell::new(1)),
                reported: Cell::new(false),
            },
            peer,
        )
    }
    fn human_key(f: &mut crate::tests::fixture::Fixture) {
        use crate::tests::test_input_backend::{TestInputBackend, TestKeyboardKeyEvent};
        use smithay::backend::input::{InputEvent, InputTime, KeyState, Keycode};
        f.niri_state()
            .process_input_event(InputEvent::<TestInputBackend>::Keyboard {
                event: TestKeyboardKeyEvent {
                    time: InputTime::now(),
                    code: Keycode::from(48_u32 + 8),
                    state: KeyState::Pressed,
                    count: 1,
                },
            });
    }
    #[test]
    fn real_coordinator_takeover_before_write_sends_no_navigation_bytes() {
        use std::io::Read;
        let mut f = crate::tests::fixture::Fixture::with_agent_input();
        f.add_output(1, (800, 600));
        let (client, surface, target) = fixture_window(&mut f);
        let (writer, mut peer) = fixture_writer(&mut f, target);
        human_key(&mut f);
        assert!(matches!(writer.ready(f.niri_state()), PostAction::Remove));
        let mut bytes = [0u8; 8192];
        assert_eq!(
            peer.read(&mut bytes).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        f.double_roundtrip(client);
        assert!(f
            .client(client)
            .state
            .recent_keyboard_events(&surface)
            .any(|event| matches!(
                event,
                crate::tests::client::KeyboardEvent::Key { key: 48, .. }
            )));
    }
    #[test]
    fn real_coordinator_takeover_after_write_keeps_unknown_and_never_resends() {
        use std::io::Read;
        let mut f = crate::tests::fixture::Fixture::with_agent_input();
        f.add_output(1, (800, 600));
        let (client, surface, target) = fixture_window(&mut f);
        let (writer, mut peer) = fixture_writer(&mut f, target);
        assert!(matches!(writer.ready(f.niri_state()), PostAction::Remove));
        assert!(writer.reported.get());
        let mut bytes = [0u8; 8192];
        let first = peer.read(&mut bytes).unwrap();
        assert_eq!(first, writer.frame.len());
        f.niri_state()
            .apply_agent_input(
                2,
                niri_ipc::AgentInput::Create {
                    name: "another-peer".into(),
                },
            )
            .unwrap();
        let native = f
            .niri_state()
            .apply_agent_input(2, niri_ipc::AgentInput::Focus { window_id: target })
            .unwrap();
        assert!(
            matches!(native, niri_ipc::Response::AgentInputPaused { reason, .. } if reason.contains("unresolved external effect"))
        );
        assert!(matches!(
            f.niri_state()
                .apply_agent_input(2, niri_ipc::AgentInput::Reset)
                .unwrap(),
            niri_ipc::Response::Handled
        ));
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let socket = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let mut other_peer = listener.accept().unwrap().0;
        other_peer.set_nonblocking(true).unwrap();
        let mut request = writer.request.clone();
        request.peer = 2;
        request.intent_id = "different-intent".into();
        request.request_id = 12;
        let another = Writer {
            owner: None,
            socket,
            frame: navigate_frame(&request).unwrap(),
            request,
            deadline: Instant::now() + Duration::from_secs(1),
            reply: None,
            channel: writer.channel.clone(),
            pending: Rc::new(Cell::new(1)),
            reported: Cell::new(false),
        };
        assert!(matches!(another.ready(f.niri_state()), PostAction::Remove));
        assert_eq!(
            other_peer.read(&mut bytes).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        human_key(&mut f);
        assert!(matches!(writer.ready(f.niri_state()), PostAction::Remove));
        assert_eq!(peer.read(&mut bytes).unwrap(), 0);
        f.double_roundtrip(client);
        assert!(f
            .client(client)
            .state
            .recent_keyboard_events(&surface)
            .any(|event| matches!(
                event,
                crate::tests::client::KeyboardEvent::Key { key: 48, .. }
            )));
    }

    #[test]
    fn actual_broker_upgrade_fd_registration_actor_send_and_unknown_receipt() {
        use dojo_external_admission::broker::{navigate, NavigateIntent};
        let mut f = crate::tests::fixture::Fixture::with_agent_input();
        f.add_output(1, (800, 600));
        let (_client, _surface, target) = fixture_window(&mut f);
        let (sample, _) = fixture_writer(&mut f, target);
        let mut entropy = [0u8; 8];
        assert_eq!(
            unsafe { libc::getrandom(entropy.as_mut_ptr().cast(), entropy.len(), 0) },
            8
        );
        let directory = std::env::temp_dir().join(format!(
            "dojo-external-test-{}-{}",
            std::process::id(),
            u64::from_le_bytes(entropy)
        ));
        std::fs::create_dir(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = directory.join("actor.sock");
        start(
            &f.state.server.event_loop.handle(),
            path.clone(),
            std::process::id(),
            process_start(std::process::id()).unwrap(),
        )
        .unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let stream = listener.accept().unwrap().0;
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut ws = tungstenite::accept_hdr(
                stream,
                |request: &tungstenite::handshake::server::Request, response| {
                    assert_eq!(request.uri().path(), "/devtools/page/PAGE-1");
                    Ok(response)
                },
            )
            .unwrap();
            let payload = ws.read().unwrap().into_text().unwrap();
            let value: serde_json::Value = serde_json::from_str(&payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({"id":9,"method":"Page.navigate","params":{"url":"https://example.invalid/test"}})
            );
        });
        let std::net::SocketAddr::V4(endpoint) = address else {
            panic!("ipv4")
        };
        let r = &sample.request;
        let intent = NavigateIntent {
            version: 1,
            peer: r.peer,
            revision: r.revision,
            window_id: target,
            browser_pid: r.browser_pid,
            browser_start_time: r.browser_start_time,
            page_target: r.page_target.clone(),
            request_id: r.request_id,
            url: r.url.clone(),
            intent_id: r.intent_id.clone(),
            grant_id: r.grant_id.clone(),
            lifetime_ms: 1000,
            capture_geometry: serde_json::to_value(&r.capture_geometry).unwrap(),
        };
        // sample owns no actor source. Remove it so only the broker connection is
        // registered; its no-effect Drop cannot change target admission state.
        drop(sample);
        let broker = std::thread::spawn(move || navigate(&path, endpoint, intent));
        let deadline = Instant::now() + Duration::from_secs(3);
        while !broker.is_finished() && Instant::now() < deadline {
            f.dispatch();
            std::thread::sleep(Duration::from_millis(1));
        }
        let receipt = broker.join().unwrap().unwrap();
        assert_eq!(receipt.effect, "unknown");
        assert!(receipt.accepted_bytes > 0);
        assert_eq!(receipt.intent_id, "pi-intent-1");
        server.join().unwrap();
        drop(f);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
