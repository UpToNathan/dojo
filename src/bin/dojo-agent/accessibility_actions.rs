//! Session-owned semantic references. Preparation never mutates an application.
use anyhow::{Context, Result, ensure};
use dojo_accessibility::{
    Node,
    actions::{Action, SelectedAction, require_same_element, select},
};
use dojo_external_admission::{
    atspi_transport::{self, Mutation, PrepareRequest, PreparedCall, TerminalReceipt},
    semantic::{self, Effect, Receipt, Request},
};
use niri_ipc::AccessibilityTarget;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    os::{
        fd::OwnedFd,
        unix::{fs::PermissionsExt, net::UnixDatagram},
    },
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

#[derive(Debug)]
pub(super) struct Reference {
    bus_guid: String,
    target: AccessibilityTarget,
    process_start: u64,
    node: Node,
    expires: Instant,
}
#[derive(Default)]
pub(super) struct Session {
    references: HashMap<String, Reference>,
    pending: Vec<Pending>,
    completed: Vec<Value>,
}
struct Channel {
    socket: UnixDatagram,
    path: PathBuf,
    actor: PathBuf,
    actor_pid: u32,
    actor_start: u64,
}
impl Drop for Channel {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
impl Channel {
    fn open(native: Option<&Path>, actor_pid: u32) -> Result<Self> {
        let native = native
            .map(Path::to_path_buf)
            .or_else(|| std::env::var_os("NIRI_SOCKET").map(PathBuf::from))
            .context("native socket unavailable")?;
        let root = PathBuf::from(
            std::env::var_os("XDG_RUNTIME_DIR").context("runtime directory unavailable")?,
        )
        .join("dojo-desktop");
        let metadata = std::fs::symlink_metadata(&root)?;
        ensure!(
            metadata.is_dir() && metadata.permissions().mode() & 0o777 == 0o700,
            "semantic reply directory is not private"
        );
        let path = root.join(format!("a-{}.sock", uuid::Uuid::new_v4().simple()));
        let socket = UnixDatagram::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        dojo_browser_control::unix::enable_credentials(&socket)?;
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket,
            path,
            actor: semantic::socket_path(&native),
            actor_pid,
            actor_start: super::screenshot::process_start_time(actor_pid)?,
        })
    }
    fn send(&self, request: &Request, fd: Option<&OwnedFd>) -> Result<()> {
        ensure!(
            super::screenshot::process_start_time(self.actor_pid)? == self.actor_start,
            "compositor generation changed"
        );
        dojo_browser_control::unix::send_packet(&self.socket, &self.actor, request, fd)?;
        Ok(())
    }
    fn receive(&self, id: &str, deadline: Instant, cancelled: &AtomicBool) -> Result<Receipt> {
        loop {
            ensure!(
                !cancelled.load(Ordering::Acquire),
                "semantic dispatch canceled after possible handoff"
            );
            if let Some((receipt, fd, credentials, _)) =
                dojo_browser_control::unix::receive_packet::<Receipt>(&self.socket)?
            {
                ensure!(
                    fd.is_none()
                        && credentials.pid > 0
                        && credentials.pid as u32 == self.actor_pid
                        && credentials.uid == unsafe { libc::geteuid() }
                        && super::screenshot::process_start_time(self.actor_pid)?
                            == self.actor_start,
                    "semantic receipt sender mismatch"
                );
                ensure!(receipt.id == id, "semantic receipt id mismatch");
                return Ok(receipt);
            }
            ensure!(
                Instant::now() < deadline,
                "semantic dispatch receipt deadline exceeded"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}
struct Pending {
    id: String,
    peer: u64,
    channel: Channel,
    runtime: tokio::runtime::Runtime,
    prepared: PreparedCall,
}
impl Pending {
    fn terminal(&mut self, deadline: Instant) -> Option<TerminalReceipt> {
        let receipt = self.runtime.block_on(self.prepared.await_receipt(deadline));
        if let Some(terminal) = receipt.terminal {
            let request = Request::Terminal {
                id: self.id.clone(),
                peer: self.peer,
            };
            // A failed terminal notification leaves Niri's barrier in place.
            if self.channel.send(&request, None).is_ok() {
                let never = AtomicBool::new(false);
                if self
                    .channel
                    .receive(
                        &self.id,
                        Instant::now() + Duration::from_millis(100),
                        &never,
                    )
                    .is_ok_and(|r| r.effect == Effect::Terminal)
                {
                    return Some(terminal);
                }
            }
        }
        None
    }
}
impl Session {
    pub fn active(&self) -> bool {
        !self.pending.is_empty() || self.references.values().any(|r| r.expires > Instant::now())
    }
    pub fn next_expiry(&self) -> Option<Instant> {
        self.references
            .values()
            .map(|r| r.expires)
            .filter(|expiry| *expiry > Instant::now())
            .min()
    }
    pub fn retire(&mut self) {
        self.references.clear();
    }
    pub fn poll(&mut self) {
        self.pending.retain_mut(|pending| {
            if let Some(terminal)=pending.terminal(Instant::now()+Duration::from_millis(1)) {
                if self.completed.len()<32 {self.completed.push(json!({"operation_id":pending.id,"effect":"unknown", "late_terminal":match terminal {TerminalReceipt::Acknowledged=>"acknowledged",TerminalReceipt::Rejected=>"rejected"}}));}
                false
            } else {true}
        });
    }
    pub fn take_receipts(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.completed)
    }
    pub fn issue(&mut self, observation: &mut super::accessibility::NativeObservation) {
        self.retire();
        observation.element_refs.clear();
        observation.tree.actionable = false;
        if !observation.input_stable
            || !observation.tree.available
            || !observation.tree.window_scoped
            || observation.target.endpoint.is_none()
            || self.pending.len() >= 16
        {
            return;
        }
        for node in &observation.tree.nodes {
            if node.redacted || node.enabled != Some(true) {
                continue;
            }
            let possible = select(node, &Action::Invoke).is_ok()
                || select(node, &Action::Select).is_ok()
                || select(
                    node,
                    &Action::ReplaceText {
                        text: String::new(),
                    },
                )
                .is_ok()
                || node
                    .numeric_range
                    .is_some_and(|(value, _)| select(node, &Action::SetNumeric { value }).is_ok());
            if !possible {
                continue;
            }
            let token = format!("a:{}", uuid::Uuid::new_v4().simple());
            self.references.insert(
                token.clone(),
                Reference {
                    bus_guid: match &observation.tree.bus_guid {
                        Some(guid) => guid.clone(),
                        None => continue,
                    },
                    target: observation.target.clone(),
                    process_start: observation.tree.process_start_time,
                    node: node.clone(),
                    expires: Instant::now() + Duration::from_secs(15),
                },
            );
            observation.element_refs.insert(node.index, token);
        }
        observation.tree.actionable = !observation.element_refs.is_empty();
    }
    pub fn consume(&mut self, token: &str) -> Result<Reference> {
        let reference = self.references.remove(token);
        self.retire();
        let reference = reference
            .context("unknown, consumed or foreign accessibility reference; observe again")?;
        ensure!(
            Instant::now() < reference.expires,
            "accessibility reference expired; observe again"
        );
        Ok(reference)
    }
    pub fn perform(
        &mut self,
        native: Option<&Path>,
        peer: u64,
        reference: Reference,
        action: Action,
        cancelled: &AtomicBool,
    ) -> Result<Value> {
        ensure!(
            self.pending.is_empty(),
            "a semantic operation is unresolved; wait for its actual reply"
        );
        ensure!(
            self.pending.len() < 16,
            "unresolved semantic operation budget exceeded"
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        ensure!(
            !cancelled.load(Ordering::Acquire),
            "desktop session disconnected"
        );
        let mut socket = super::desktop_socket(native)?;
        let actor_pid = socket.peer_process_id()?;
        ensure!(
            super::accessibility::target(
                &mut socket,
                reference.target.window_id,
                deadline,
                cancelled
            )? == reference.target,
            "accessibility target or ownership changed; observe again"
        );
        let live = super::accessibility::observe_bound(
            &mut socket,
            reference.target.window_id,
            deadline,
            cancelled,
        )?
        .context("exact accessibility binding retired")?;
        ensure!(
            live.target == reference.target
                && live.tree.process_start_time == reference.process_start
                && live.tree.bus_guid.as_deref() == Some(reference.bus_guid.as_str()),
            "accessibility generation changed; observe again"
        );
        let node = live
            .tree
            .nodes
            .iter()
            .find(|node| {
                node.bus_owner == reference.node.bus_owner
                    && node.object_path == reference.node.object_path
            })
            .context("element no longer belongs to the selected root; observe again")?;
        require_same_element(&reference.node, node)?;
        let selected = select(node, &action)?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let mut mutation_owner = node.bus_owner.clone();
        let mut mutation_path = node.object_path.clone();
        let mutation = match selected {
            SelectedAction::Invoke { index } => Mutation::Invoke {
                index: index.try_into()?,
            },
            SelectedAction::Select { parent } => {
                mutation_owner = parent.bus_owner;
                mutation_path = parent.object_path;
                Mutation::SelectChild {
                    index: parent.index.try_into()?,
                }
            }
            SelectedAction::ReplaceText { text } => Mutation::SetText { text },
            SelectedAction::SetNumeric { value } => Mutation::SetNumeric { value },
        };
        let pid: u32 = reference
            .target
            .pid
            .context("target PID unavailable")?
            .try_into()?;
        let mut prepared = runtime.block_on(async {
            tokio::time::timeout_at(deadline.into(), async {
                let session = zbus::Connection::session().await?;
                let proxy =
                    zbus::Proxy::new(&session, "org.a11y.Bus", "/org/a11y/bus", "org.a11y.Bus")
                        .await?;
                let address: String = proxy.call("GetAddress", &()).await?;
                atspi_transport::prepare(
                    PrepareRequest {
                        bus_address: &address,
                        unique_owner: &mutation_owner,
                        owner_pid: pid,
                        owner_uid: unsafe { libc::geteuid() },
                        object_path: &mutation_path,
                        mutation,
                    },
                    deadline,
                )
                .await
            })
            .await
        })??;
        ensure!(
            prepared.bus_guid() == reference.bus_guid,
            "accessibility bus generation changed during preparation"
        );
        ensure!(
            !cancelled.load(Ordering::Acquire),
            "semantic action canceled before handoff"
        );
        ensure!(
            super::accessibility::target(
                &mut socket,
                reference.target.window_id,
                deadline,
                cancelled
            )? == reference.target,
            "accessibility target changed during preparation"
        );
        let lifetime_ms = reference
            .expires
            .saturating_duration_since(Instant::now())
            .as_millis()
            .min(1000) as u32;
        ensure!(
            lifetime_ms > 0,
            "accessibility reference expired during preparation"
        );
        let channel = Channel::open(native, actor_pid)?;
        let id = uuid::Uuid::new_v4().simple().to_string();
        let request = Request::Dispatch {
            id: id.clone(),
            peer,
            revision: reference.target.coordination_revision,
            input_generation: reference.target.input_generation,
            target: reference.target,
            process_start: reference.process_start,
            lifetime_ms,
            payload: prepared.payload().to_vec(),
        };
        let fd = prepared.take_commit_fd()?;
        channel.send(&request, Some(&fd))?;
        drop(fd);
        let mut pending = Pending {
            id: id.clone(),
            peer,
            channel,
            runtime,
            prepared,
        };
        let dispatch =
            pending
                .channel
                .receive(&id, Instant::now() + Duration::from_secs(2), cancelled);
        if let Ok(receipt) = &dispatch {
            if receipt.effect == Effect::Refused {
                return Ok(
                    json!({"operation_id":id,"reference_consumed":true,"effect":"refused","reason":receipt.message,"verified":false,"requires_fresh_observation":true}),
                );
            }
        }
        if dispatch.is_err() {
            pending
                .prepared
                .mark_unknown(if cancelled.load(Ordering::Acquire) {
                    atspi_transport::UnknownReason::CanceledAfterCommit
                } else {
                    atspi_transport::UnknownReason::Deadline
                });
        }
        let terminal = pending.terminal(Instant::now() + Duration::from_millis(750));
        let uncertain = pending.prepared.receipt().uncertain;
        let effect = match (uncertain, terminal) {
            (false, Some(TerminalReceipt::Acknowledged)) => "acknowledged",
            (false, Some(TerminalReceipt::Rejected)) => "rejected",
            _ => "unknown",
        };
        let result = json!({"operation_id":id,"reference_consumed":true,"effect":effect,"late_terminal":if uncertain {terminal.map(|r| match r {TerminalReceipt::Acknowledged=>"acknowledged",TerminalReceipt::Rejected=>"rejected"})} else {None},"dispatch":dispatch.as_ref().ok(),"dispatch_error":dispatch.err().map(|e|e.to_string()),"verified":false,"requires_fresh_observation":true,"automatic_retry_allowed":false});
        if terminal.is_none() {
            self.pending.push(pending);
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn observation() -> super::super::accessibility::NativeObservation {
        let node = Node {
            index: 1,
            selection_parent: None,
            parent: Some(0),
            depth: 1,
            bus_owner: ":1.8".into(),
            object_path: "/button".into(),
            role: "button".into(),
            name: Some("Save".into()),
            description: None,
            text: None,
            value: None,
            redacted: false,
            enabled: Some(true),
            checked: None,
            selected: None,
            states: vec!["enabled".into(), "sensitive".into()],
            actions: vec!["click".into()],
            interfaces: vec!["org.a11y.atspi.Action".into()],
            numeric_range: None,
            reported_screen_bounds: None,
            bounds_authoritative: false,
        };
        super::super::accessibility::NativeObservation {
            requested_window_id: 4,
            binding_generation: Some(1),
            semantic_complete: true,
            element_refs: Default::default(),
            input_stable: true,
            target: AccessibilityTarget {
                window_id: 4,
                pid: Some(12),
                generation: 1,
                annotation_generation: 0,
                coordination_revision: 0,
                input_generation: 0,
                endpoint: Some(niri_ipc::AccessibilityEndpoint {
                    bus_name: ":1.8".into(),
                    object_path: "/root".into(),
                }),
            },
            tree: dojo_accessibility::Observation {
                bus_guid: Some("fixture-guid".into()),
                pid: 12,
                process_start_time: 9,
                scope: "window",
                actionable: false,
                window_scoped: true,
                available: true,
                complete: true,
                query_filtered: false,
                issues: Default::default(),
                nodes_visited: 1,
                nodes_pending: 0,
                nodes: vec![node],
            },
        }
    }
    #[test]
    fn references_are_one_use_session_owned_and_snapshot_scoped() {
        let mut owner = Session::default();
        let mut foreign = Session::default();
        let mut tree = observation();
        owner.issue(&mut tree);
        let token = tree.element_refs[&1].clone();
        assert!(foreign.consume(&token).is_err());
        assert!(owner.consume(&token).is_ok());
        assert!(owner.consume(&token).is_err());
        owner.issue(&mut tree);
        let old = tree.element_refs[&1].clone();
        let mut newer = observation();
        owner.issue(&mut newer);
        assert!(owner.consume(&old).is_err());
        // A failed attempt consumes the whole current snapshot too.
        assert!(owner.consume(&newer.element_refs[&1]).is_err());
    }
    #[test]
    fn positive_refs_do_not_claim_complete_coverage_and_keep_the_session_alive() {
        let mut session = Session::default();
        let mut tree = observation();
        tree.semantic_complete = false;
        tree.tree.complete = false;
        tree.tree.query_filtered = true;
        session.issue(&mut tree);
        assert!(!tree.element_refs.is_empty());
        assert!(session.active());
        assert!(session.next_expiry().is_some());
        for reference in session.references.values_mut() {
            reference.expires = Instant::now() - Duration::from_secs(1);
        }
        assert!(!session.active());
        assert!(session.next_expiry().is_none());
        assert!(
            session
                .consume(&tree.element_refs[&1])
                .unwrap_err()
                .to_string()
                .contains("expired")
        );
    }
    #[test]
    fn expiration_privacy_and_unbound_scope_refuse_action_refs() {
        let mut owner = Session::default();
        let mut tree = observation();
        owner.issue(&mut tree);
        let token = tree.element_refs[&1].clone();
        owner.references.get_mut(&token).unwrap().expires = Instant::now() - Duration::from_secs(1);
        assert!(owner.consume(&token).is_err());
        for variant in 0..3 {
            let mut tree = observation();
            match variant {
                0 => tree.tree.nodes[0].redacted = true,
                1 => tree.tree.window_scoped = false,
                _ => tree.input_stable = false,
            }
            owner.issue(&mut tree);
            assert!(tree.element_refs.is_empty());
        }
    }
}
