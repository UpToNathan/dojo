//! AT-SPI writers share Niri's native input actor and kernel peer identity.
use crate::niri::State;
use calloop::{
    Interest, LoopHandle, Mode, PostAction,
    generic::Generic,
    timer::{TimeoutAction, Timer},
};
use dojo_browser_control::unix;
use dojo_external_admission::semantic::{Effect, Receipt, Request};
use smithay::wayland::seat::WaylandFocus;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    io,
    os::{
        fd::{AsFd, AsRawFd, OwnedFd},
        unix::{fs::PermissionsExt, net::UnixDatagram},
    },
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

#[derive(Clone)]
struct Pending {
    peer: u64,
    owner: (u32, u64),
    target: niri_ipc::AccessibilityTarget,
    process_start: u64,
}
pub(crate) struct Channel {
    pub generation: u64,
    unresolved: HashMap<String, Pending>,
}
impl Default for Channel {
    fn default() -> Self {
        Self {
            generation: 0,
            unresolved: HashMap::new(),
        }
    }
}
impl State {
    pub(crate) fn semantic_input_activity(&mut self) {
        self.niri.semantic_admission.generation =
            self.niri.semantic_admission.generation.saturating_add(1);
    }
    pub(crate) fn semantic_target_unresolved(&self, window: u64) -> bool {
        let pid = self
            .niri
            .layout
            .windows()
            .find(|(_, mapped)| mapped.id().get() == window)
            .and_then(|(_, mapped)| mapped.credentials())
            .map(|credentials| credentials.pid);
        self.niri
            .semantic_admission
            .unresolved
            .values()
            .any(|pending| {
                // An admitted action can affect sibling windows in the same app.
                // Retiring its original surface does not establish quiescence.
                pid.is_some()
                    && pending.target.pid == pid
                    && pid
                        .and_then(|pid| u32::try_from(pid).ok())
                        .is_some_and(|pid| {
                            crate::external_admission::process_start(pid).ok()
                                == Some(pending.process_start)
                        })
            })
    }
    pub(crate) fn accessibility_target(
        &self,
        window_id: u64,
    ) -> Result<niri_ipc::AccessibilityTarget, String> {
        let mapped = self
            .niri
            .layout
            .windows()
            .find(|(_, w)| w.id().get() == window_id)
            .map(|(_, w)| w)
            .ok_or("window disappeared")?;
        let surface = mapped
            .window
            .wl_surface()
            .ok_or("window has no native surface")?;
        let pid = crate::utils::get_credentials_for_surface(&surface).map(|c| c.pid);
        let (generation, gtk) = crate::protocols::gtk_shell::endpoint(&surface)?;
        let (annotation_generation, generic) =
            crate::protocols::dbus_annotation::endpoint(&surface)?;
        let endpoint = match (gtk, generic) {
            (Some(a), Some(b)) if a != b => {
                return Err("conflicting accessibility annotations".into());
            }
            (Some(a), _) | (_, Some(a)) => Some(a),
            _ => None,
        };
        Ok(niri_ipc::AccessibilityTarget {
            window_id,
            pid,
            generation,
            annotation_generation,
            endpoint,
            coordination_revision: self.niri.input_coordination.revision,
            input_generation: self.niri.semantic_admission.generation,
        })
    }
}
struct Writer {
    fd: OwnedFd,
    id: String,
    pending: Pending,
    revision: u64,
    input_generation: u64,
    payload: Vec<u8>,
    deadline: Instant,
    reply: PathBuf,
    socket: Rc<UnixDatagram>,
    reported: Cell<bool>,
    count: Rc<Cell<usize>>,
    preparer: Rc<dojo_external_admission::semantic_preparer::TrustedPreparer>,
}
impl AsFd for Writer {
    fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> {
        self.fd.as_fd()
    }
}
impl Writer {
    fn report(&self, effect: Effect, count: usize, message: &str) {
        self.reported.set(true);
        let receipt = Receipt {
            id: self.id.clone(),
            effect,
            accepted_bytes: count,
            message: message.into(),
        };
        if let Ok(bytes) = serde_json::to_vec(&receipt) {
            let _ = self.socket.send_to(&bytes, &self.reply);
        }
    }
    fn ready(&self, state: &mut State) -> PostAction {
        if self.reported.get() {
            return PostAction::Remove;
        }
        let (owner, start) = self.pending.owner;
        let pid = self
            .pending
            .target
            .pid
            .and_then(|pid| u32::try_from(pid).ok());
        let refusal = if Instant::now() >= self.deadline {
            Some("semantic handoff expired".to_owned())
        } else if !self.preparer.matches(owner, start) {
            Some("pinned semantic preparer retired".to_owned())
        } else if !state.agent_peer_owned_by(self.pending.peer, owner, start) {
            Some("semantic native peer retired".to_owned())
        } else if self.input_generation != state.niri.semantic_admission.generation {
            Some("input occurred after the accessibility observation".to_owned())
        } else if state.semantic_target_unresolved(self.pending.target.window_id) {
            Some("application has an unresolved semantic effect".to_owned())
        } else if state
            .accessibility_target(self.pending.target.window_id)
            .ok()
            .as_ref()
            != Some(&self.pending.target)
        {
            Some("native accessibility binding changed".to_owned())
        } else if !pid.is_some_and(|pid| {
            crate::external_admission::process_start(pid).ok() == Some(self.pending.process_start)
        }) {
            Some("accessibility process generation changed".to_owned())
        } else {
            state
                .resolve_external_target(
                    self.pending.peer,
                    self.revision,
                    self.pending.target.window_id,
                )
                .err()
        };
        if let Some(reason) = refusal {
            self.report(Effect::Refused, 0, &reason);
            return PostAction::Remove;
        }
        // Native human input and this sole mutation send execute on this actor.
        let count = unsafe {
            libc::send(
                self.fd.as_raw_fd(),
                self.payload.as_ptr().cast(),
                self.payload.len(),
                libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL,
            )
        };
        if count < 0
            && matches!(
                io::Error::last_os_error().kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
            )
        {
            return PostAction::Continue;
        }
        state.semantic_input_activity();
        state
            .niri
            .semantic_admission
            .unresolved
            .insert(self.id.clone(), self.pending.clone());
        self.report(
            Effect::Unknown,
            count.max(0) as usize,
            if count == self.payload.len() as isize {
                "dispatched; awaiting correlated application reply"
            } else {
                "dispatch uncertain; never retry or send remainder"
            },
        );
        // The read-only zbus half remains alive in the service. Shut down writes
        // even for partial messages so no late library send can complete them.
        if count != self.payload.len() as isize {
            unsafe {
                libc::shutdown(self.fd.as_raw_fd(), libc::SHUT_WR);
            }
        }
        PostAction::Remove
    }
}
impl Drop for Writer {
    fn drop(&mut self) {
        self.count.set(self.count.get().saturating_sub(1));
        if !self.reported.get() {
            self.report(Effect::Refused, 0, "writer retired before dispatch");
        }
    }
}
struct Cleanup(PathBuf);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
pub(crate) fn start(handle: &LoopHandle<'static, State>, path: PathBuf) -> anyhow::Result<()> {
    let pinned = std::env::var_os("DOJO_SEMANTIC_AGENT_EXE")
        .ok_or_else(|| anyhow::anyhow!("pinned semantic preparer is not configured"))?;
    let preparer = Rc::new(
        dojo_external_admission::semantic_preparer::TrustedPreparer::open(std::path::Path::new(
            &pinned,
        ))?,
    );
    let socket = UnixDatagram::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    socket.set_nonblocking(true)?;
    unix::enable_credentials(&socket)?;
    let socket = Rc::new(socket);
    let receiver = socket.try_clone()?;
    let cleanup = Cleanup(path);
    let count = Rc::new(Cell::new(0));
    let seen = Rc::new(RefCell::new(std::collections::HashSet::<String>::new()));
    let seen_generation = Cell::new(u64::MAX);
    let actor = handle.clone();
    handle.insert_source(Generic::new(receiver,Interest::READ,Mode::Level),move |_,socket_fd,state|{
        let _keep=&cleanup;
        let Ok(Some((request,fd,credentials,reply)))=unix::receive_packet::<Request>(socket_fd) else {return Ok(PostAction::Continue)};
        if credentials.uid != unsafe{libc::geteuid()} || credentials.pid<=0 {return Ok(PostAction::Continue)}
        let owner=credentials.pid as u32;
        let Ok(start)=crate::external_admission::process_start(owner) else{return Ok(PostAction::Continue)};
        if !preparer.matches(owner,start) {return Ok(PostAction::Continue)}
        state.niri.semantic_admission.unresolved.retain(|_,pending| {
            pending.target.pid.and_then(|p|u32::try_from(p).ok()).is_some_and(|pid|
                crate::external_admission::process_start(pid).ok()==Some(pending.process_start))
        });
        // Every possible commit advances this generation. Old packets then
        // fail freshness checks, so their ID tombstones need not live forever.
        if seen_generation.get() != state.niri.semantic_admission.generation {
            seen.borrow_mut().clear();
            seen_generation.set(state.niri.semantic_admission.generation);
        }
        match request {
            Request::Terminal{id,peer} => {
                if fd.is_none()
                    && state.niri.semantic_admission.unresolved.get(&id).is_some_and(|p|p.peer==peer && p.owner==(owner,start)) {
                    state.niri.semantic_admission.unresolved.remove(&id);
                    let receipt=Receipt{id,effect:Effect::Terminal,accepted_bytes:0,message:"correlated application reply received; UI result requires observation".into()};
                    if let Ok(bytes)=serde_json::to_vec(&receipt){let _=socket.send_to(&bytes,&reply);}
                }
            }
            Request::Dispatch{id,peer,revision,input_generation,target,process_start,lifetime_ms,payload} => {
                let refuse=|message:&str| {
                    if let Ok(bytes)=serde_json::to_vec(&Receipt{id:id.clone(),effect:Effect::Refused,accepted_bytes:0,message:message.into()}) {let _=socket.send_to(&bytes,&reply);}
                };
                if id.is_empty() || id.len()>128 || seen.borrow().len()>=4096 || seen.borrow().contains(&id)
                    || count.get()>=16 || !(1..=5000).contains(&lifetime_ms) || payload.is_empty() || payload.len()>65536
                    || target.endpoint.is_none() || revision!=target.coordination_revision || input_generation!=target.input_generation
                    || !state.agent_peer_owned_by(peer,owner,start) {
                    refuse("invalid, stale or over-budget semantic handoff");return Ok(PostAction::Continue);
                }
                let Some(fd)=fd else{refuse("missing one-use writer");return Ok(PostAction::Continue)};
                // Only AF_UNIX stream sockets are accepted for the local AT-SPI bus.
                let mut domain=0i32;let mut length=std::mem::size_of_val(&domain) as libc::socklen_t;
                let valid=unsafe{libc::getsockopt(fd.as_raw_fd(),libc::SOL_SOCKET,libc::SO_DOMAIN,(&mut domain as *mut i32).cast(),&mut length)}==0 && domain==libc::AF_UNIX;
                let mut kind=0i32;length=std::mem::size_of_val(&kind) as _;
                let valid=valid && unsafe{libc::getsockopt(fd.as_raw_fd(),libc::SOL_SOCKET,libc::SO_TYPE,(&mut kind as *mut i32).cast(),&mut length)}==0 && kind==libc::SOCK_STREAM;
                if !valid {refuse("writer is not a local bus stream");return Ok(PostAction::Continue)}
                seen.borrow_mut().insert(id.clone());count.set(count.get()+1);
                let writer=Writer{fd,id,pending:Pending{peer,owner:(owner,start),target,process_start},revision,input_generation,payload,deadline:Instant::now()+Duration::from_millis(lifetime_ms as u64),reply,socket:socket.clone(),reported:Cell::new(false),count:count.clone(),preparer:preparer.clone()};
                let reported=writer.reported.clone();
                // Timer removal bounds a writer that never becomes writable.
                let Ok(token)=actor.insert_source(Generic::new(writer,Interest::WRITE,Mode::Level),|_,writer,state|Ok(writer.ready(state))) else {return Ok(PostAction::Continue)};
                let expiry=actor.clone();
                if actor.insert_source(Timer::from_duration(Duration::from_millis(lifetime_ms as u64)),move |_,_,_|{let _=&reported;expiry.remove(token);TimeoutAction::Drop}).is_err() {actor.remove(token);}
            }
        }
        Ok(PostAction::Continue)
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_effect_fences_same_process_siblings_until_terminal_or_process_retirement() {
        let mut fixture = crate::tests::fixture::Fixture::with_agent_input();
        fixture.add_output(1, (800, 600));
        let mut windows = Vec::new();
        for _ in 0..2 {
            let client = fixture.add_client();
            let window = fixture.client(client).create_window();
            let surface = window.surface.clone();
            window.commit();
            fixture.double_roundtrip(client);
            let window = fixture.client(client).window(&surface);
            window.attach_new_buffer();
            window.ack_last_and_commit();
            fixture.double_roundtrip(client);
            fixture.niri_complete_animations();
            windows.push(fixture.niri().layout.focus().unwrap().id().get());
        }
        let target = fixture
            .niri_state()
            .accessibility_target(windows[0])
            .unwrap();
        let start = crate::external_admission::process_start(std::process::id()).unwrap();
        fixture
            .niri_state()
            .niri
            .semantic_admission
            .unresolved
            .insert(
                "test".into(),
                Pending {
                    peer: 1,
                    owner: (std::process::id(), start),
                    target,
                    process_start: start,
                },
            );
        assert!(fixture.niri_state().semantic_target_unresolved(windows[0]));
        assert!(fixture.niri_state().semantic_target_unresolved(windows[1]));
        fixture
            .niri_state()
            .niri
            .semantic_admission
            .unresolved
            .get_mut("test")
            .unwrap()
            .process_start = start + 1;
        assert!(!fixture.niri_state().semantic_target_unresolved(windows[1]));
        fixture
            .niri_state()
            .niri
            .semantic_admission
            .unresolved
            .clear();
        assert!(!fixture.niri_state().semantic_target_unresolved(windows[0]));
    }
}
