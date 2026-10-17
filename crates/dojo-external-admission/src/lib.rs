//! Deterministic ticket-16 feasibility model. This does not admit production effects.
//! The enforceable point is one nonblocking transport commit on the coordination
//! actor's thread. Preliminary checks never create a transferable permit.
#[cfg(target_os = "linux")]
pub mod activation;
#[cfg(all(target_os = "linux", feature = "atspi-transport"))]
pub mod atspi_transport;
#[cfg(target_os = "linux")]
pub mod semantic_preparer;
pub mod broker;
#[cfg(feature = "socket-fixtures")]
pub mod sockets;

use std::{
    collections::BTreeMap,
    num::{NonZeroU32, NonZeroU64},
    sync::Arc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ClientIdentity {
    pub id: NonZeroU64,
    pub generation: NonZeroU64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessIdentity {
    pub pid: NonZeroU32,
    pub start_time: NonZeroU64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TargetIdentity {
    pub compositor_epoch: NonZeroU64,
    pub client: ClientIdentity,
    pub window: NonZeroU64,
    pub window_generation: NonZeroU64,
    pub process: ProcessIdentity,
    pub endpoint_generation: NonZeroU64,
    pub document_generation: NonZeroU64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    Cdp,
    Atspi,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mutation {
    CdpNavigate { url: String },
    CdpSetValue { element: String, value: String },
    CdpDownload { url: String },
    CdpDispatchKey { key: String },
    AtspiSetValue { object: String, value: String },
    AtspiInvoke { object: String, index: u32 },
}
impl Mutation {
    fn bounded(&self) -> bool {
        match self {
            Self::CdpNavigate { url } | Self::CdpDownload { url } => url.len() <= 4096,
            Self::CdpSetValue { element, value }
            | Self::AtspiSetValue {
                object: element,
                value,
            } => element.len() <= 1024 && value.len() <= 16384,
            Self::CdpDispatchKey { key } => key.len() <= 128,
            Self::AtspiInvoke { object, index } => object.len() <= 1024 && *index <= 128,
        }
    }
    pub fn route(&self) -> Route {
        match self {
            Self::CdpNavigate { .. }
            | Self::CdpSetValue { .. }
            | Self::CdpDownload { .. }
            | Self::CdpDispatchKey { .. } => Route::Cdp,
            Self::AtspiSetValue { .. } | Self::AtspiInvoke { .. } => Route::Atspi,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Requirement {
    NoTransportCommitAfterTakeover,
    NoApplicationEffectAfterTakeover,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    WrongActor,
    UnknownIntent,
    Consumed,
    StaleTarget,
    StaleRevision,
    StaleObservation,
    PendingEffect,
    HumanOwned,
    MissingGrant,
    Expired,
    UnsupportedOrdering,
    WrongRoute,
    WrongIntent,
    UnsupportedMutation,
    Disconnected,
    QueueFull,
    InvalidPayload,
}
impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Refusal {}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CancelReason {
    HumanTakeover,
    TargetChanged,
    SessionEnded,
    GrantRevoked,
    Deadline,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnknownReason {
    RevokedAfterCommit,
    PartialWrite,
    ReplyLost,
    DeadlineAfterCommit,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Receipt {
    Acknowledged,
    Rejected,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    CanceledBeforeCommit(CancelReason),
    Acknowledged {
        verified: bool,
    },
    RejectedAfterCommit,
    Unknown {
        reason: UnknownReason,
        late_receipt: Option<Receipt>,
    },
}
impl Outcome {
    pub fn requires_fresh_observation(self) -> bool {
        true
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Queued,
    InFlight,
    Finished(Outcome),
}
#[derive(Clone, Debug)]
pub struct IntentHandle {
    owner: Arc<()>,
    id: u64,
}
#[derive(Debug)]
pub struct ObservationStamp {
    owner: Arc<()>,
    target: TargetIdentity,
    revision: u64,
    generation: NonZeroU64,
}
#[derive(Clone, Debug)]
pub struct GrantHandle {
    owner: Arc<()>,
    id: u64,
}
pub struct Grant {
    owner: Arc<()>,
    target: TargetIdentity,
    mutation: Mutation,
    expires: u64,
    id: u64,
}
impl Grant {
    pub fn revocation_handle(&self) -> GrantHandle {
        GrantHandle {
            owner: self.owner.clone(),
            id: self.id,
        }
    }
}
/// A one-use borrowed permit created only inside `Actor::try_dispatch`.
/// Its lifetime prevents a transport from retaining it for a later send.
/// Approved transport adapters remain sealed until their commit path is audited.
pub struct ExternalPermit<'a> {
    handle: IntentHandle,
    target: TargetIdentity,
    mutation: &'a Mutation,
}
impl ExternalPermit<'_> {
    pub fn target(&self) -> TargetIdentity {
        self.target
    }
    pub fn mutation(&self) -> &Mutation {
        self.mutation
    }
}
struct Record {
    target: TargetIdentity,
    revision: u64,
    observation_generation: NonZeroU64,
    mutation: Mutation,
    deadline: u64,
    grant: u64,
    status: Status,
}
struct ClientState {
    human_owned: bool,
}
/// Owned by the compositor coordination event thread in a future integration.
/// Human events and try-commit run as bounded synchronous actor turns. Application
/// receipt/effect work occurs elsewhere and never borrows this actor.
pub struct Actor {
    owner: Arc<()>,
    epoch: NonZeroU64,
    revision: u64,
    clock: u64,
    next: u64,
    ended: bool,
    clients: BTreeMap<ClientIdentity, ClientState>,
    targets: BTreeMap<NonZeroU64, TargetIdentity>,
    records: BTreeMap<u64, Record>,
    grants: BTreeMap<u64, bool>,
    retired_observations: BTreeMap<NonZeroU64, NonZeroU64>,
    observed_generations: BTreeMap<NonZeroU64, NonZeroU64>,
}
impl Actor {
    pub fn new(compositor_epoch: NonZeroU64) -> Self {
        Self {
            owner: Arc::new(()),
            epoch: compositor_epoch,
            revision: 1,
            clock: 0,
            next: 0,
            ended: false,
            clients: BTreeMap::new(),
            targets: BTreeMap::new(),
            records: BTreeMap::new(),
            grants: BTreeMap::new(),
            retired_observations: BTreeMap::new(),
            observed_generations: BTreeMap::new(),
        }
    }
    fn next(&mut self) -> u64 {
        self.next = self
            .next
            .checked_add(1)
            .expect("prototype counter exhausted");
        self.next
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn install_target(&mut self, target: TargetIdentity) -> Result<(), Refusal> {
        if self.ended {
            return Err(Refusal::Disconnected);
        }
        if self.targets.len() >= 64 && !self.targets.contains_key(&target.window) {
            return Err(Refusal::QueueFull);
        }
        if self.clients.len() >= 64 && !self.clients.contains_key(&target.client) {
            return Err(Refusal::QueueFull);
        }
        if target.compositor_epoch != self.epoch {
            return Err(Refusal::StaleTarget);
        }
        if let Some(old) = self.targets.insert(target.window, target) {
            if old != target {
                self.retired_observations.remove(&target.window);
                self.observed_generations.remove(&target.window);
                self.cancel_matching(
                    |r| r.target.window == target.window,
                    CancelReason::TargetChanged,
                );
            }
        }
        self.clients
            .entry(target.client)
            .or_insert(ClientState { human_owned: false });
        Ok(())
    }
    /// Injected native evidence in this model, not a claim that a UI was observed.
    /// The production compositor bridge must re-prove target and scene generation.
    pub fn observe(
        &mut self,
        target: TargetIdentity,
        generation: NonZeroU64,
    ) -> Result<ObservationStamp, Refusal> {
        self.validate_target(target)?;
        if self
            .retired_observations
            .get(&target.window)
            .is_some_and(|retired| generation <= *retired)
        {
            return Err(Refusal::StaleObservation);
        }
        if self.pending_effect(target.client) {
            return Err(Refusal::PendingEffect);
        }
        self.observed_generations
            .entry(target.window)
            .and_modify(|current| *current = (*current).max(generation))
            .or_insert(generation);
        Ok(ObservationStamp {
            owner: self.owner.clone(),
            target,
            revision: self.revision,
            generation,
        })
    }
    /// Trusted approval-boundary injection for fixtures. Approval is bound to an
    /// exact target and mutation, and grants no human ownership or coordination lease.
    pub fn grant(
        &mut self,
        target: TargetIdentity,
        mutation: Mutation,
        expires: u64,
    ) -> Result<Grant, Refusal> {
        self.validate_target(target)?;
        if !mutation.bounded() {
            return Err(Refusal::InvalidPayload);
        }
        if self.grants.len() >= 64 {
            return Err(Refusal::QueueFull);
        }
        if expires <= self.clock {
            return Err(Refusal::Expired);
        }
        let id = self.next();
        self.grants.insert(id, true);
        Ok(Grant {
            owner: self.owner.clone(),
            target,
            mutation,
            expires,
            id,
        })
    }
    pub fn prepare(
        &mut self,
        observation: ObservationStamp,
        grant: Grant,
        deadline: u64,
    ) -> Result<IntentHandle, Refusal> {
        if self.records.len() >= 64 {
            return Err(Refusal::QueueFull);
        }
        if !Arc::ptr_eq(&self.owner, &observation.owner) || !Arc::ptr_eq(&self.owner, &grant.owner)
        {
            return Err(Refusal::WrongActor);
        }
        if observation.target != grant.target {
            return Err(Refusal::StaleTarget);
        }
        self.validate_target(grant.target)?;
        if self
            .retired_observations
            .get(&grant.target.window)
            .is_some_and(|retired| observation.generation <= *retired)
        {
            return Err(Refusal::StaleObservation);
        }
        if observation.revision != self.revision {
            return Err(Refusal::StaleRevision);
        }
        if self.clients[&grant.target.client].human_owned {
            return Err(Refusal::HumanOwned);
        }
        if deadline <= self.clock || grant.expires <= self.clock {
            return Err(Refusal::Expired);
        }
        if self
            .grants
            .get_mut(&grant.id)
            .is_none_or(|active| !std::mem::replace(active, false))
        {
            return Err(Refusal::MissingGrant);
        }
        let id = self.next();
        self.records.insert(
            id,
            Record {
                target: grant.target,
                revision: observation.revision,
                observation_generation: observation.generation,
                mutation: grant.mutation,
                deadline: deadline.min(grant.expires),
                grant: grant.id,
                status: Status::Queued,
            },
        );
        Ok(IntentHandle {
            owner: self.owner.clone(),
            id,
        })
    }
    fn pending_effect(&self, client: ClientIdentity) -> bool {
        self.records.values().any(|record| {
            record.target.client == client
                && matches!(
                    record.status,
                    Status::InFlight
                        | Status::Finished(Outcome::Unknown {
                            late_receipt: None,
                            ..
                        })
                )
        })
    }
    fn validate_target(&self, target: TargetIdentity) -> Result<(), Refusal> {
        if self.ended {
            return Err(Refusal::Disconnected);
        }
        if self.targets.get(&target.window) != Some(&target) {
            return Err(Refusal::StaleTarget);
        }
        Ok(())
    }
    fn record(&self, handle: &IntentHandle) -> Result<&Record, Refusal> {
        if !Arc::ptr_eq(&self.owner, &handle.owner) {
            return Err(Refusal::WrongActor);
        }
        self.records.get(&handle.id).ok_or(Refusal::UnknownIntent)
    }
    /// Diagnostic check only. It never creates a permit and never authorizes a send.
    pub fn inspect(&self, handle: &IntentHandle) -> Result<Status, Refusal> {
        Ok(self.record(handle)?.status)
    }
    /// Admission and the irreversible local transport commit share this actor turn.
    /// A WouldBlock result guarantees zero accepted bytes and keeps the intent queued.
    /// Human handlers never invoke this method while waiting for external readiness.
    pub fn try_dispatch(
        &mut self,
        handle: &IntentHandle,
        requirement: Requirement,
        transport: &mut impl CommitTransport,
    ) -> Result<Status, Refusal> {
        if requirement == Requirement::NoApplicationEffectAfterTakeover {
            return Err(Refusal::UnsupportedOrdering);
        }
        let record = self.record(handle)?;
        if record.status != Status::Queued {
            return Err(Refusal::Consumed);
        }
        self.validate_target(record.target)?;
        if self
            .retired_observations
            .get(&record.target.window)
            .is_some_and(|retired| record.observation_generation <= *retired)
        {
            return Err(Refusal::StaleObservation);
        }
        if record.revision != self.revision {
            return Err(Refusal::StaleRevision);
        }
        if self.clients[&record.target.client].human_owned {
            return Err(Refusal::HumanOwned);
        }
        if self.pending_effect(record.target.client) {
            return Err(Refusal::PendingEffect);
        }
        if record.deadline <= self.clock {
            return Err(Refusal::Expired);
        }
        if transport
            .intent_binding()
            .is_some_and(|bound| bound.id != handle.id || !Arc::ptr_eq(&bound.owner, &handle.owner))
        {
            return Err(Refusal::WrongIntent);
        }
        if transport
            .binding()
            .is_some_and(|binding| binding != record.target)
        {
            return Err(Refusal::StaleTarget);
        }
        if !transport.supports(&record.mutation) {
            return Err(Refusal::UnsupportedMutation);
        }
        if transport.route() != record.mutation.route() {
            return Err(Refusal::WrongRoute);
        }
        // The borrowed permit cannot escape or be held across an async suspension.
        let permit = ExternalPermit {
            handle: handle.clone(),
            target: record.target,
            mutation: &record.mutation,
        };
        let attempt = transport.try_commit(permit);
        let record = self.records.get_mut(&handle.id).unwrap();
        if matches!(attempt, CommitAttempt::Accepted | CommitAttempt::Partial) {
            // Match Dojo's session authority: any external mutation retires
            // every earlier actionable observation, including other windows.
            for (&window, &generation) in &self.observed_generations {
                self.retired_observations.insert(window, generation);
            }
        }
        match attempt {
            CommitAttempt::WouldBlock => {}
            CommitAttempt::Accepted => record.status = Status::InFlight,
            CommitAttempt::Partial => {
                record.status = Status::Finished(Outcome::Unknown {
                    reason: UnknownReason::PartialWrite,
                    late_receipt: None,
                })
            }
            CommitAttempt::Disconnected => {
                record.status =
                    Status::Finished(Outcome::CanceledBeforeCommit(CancelReason::SessionEnded))
            }
        }
        Ok(record.status)
    }
    /// Immediate human input turn. There is no transport, external lock, future,
    /// timeout, or application callback in this path.
    pub fn human_takeover(&mut self, client: ClientIdentity) {
        self.revision = self
            .revision
            .checked_add(1)
            .expect("prototype revision exhausted");
        if let Some(state) = self.clients.get_mut(&client) {
            state.human_owned = true;
        }
        // Global revision change conservatively cancels every queued intent.
        self.cancel_matching(
            |r| r.target.client == client || r.status == Status::Queued,
            CancelReason::HumanTakeover,
        );
    }
    pub fn remove_target(&mut self, window: NonZeroU64) {
        self.targets.remove(&window);
        self.cancel_matching(|r| r.target.window == window, CancelReason::TargetChanged);
    }
    pub fn human_release(&mut self, client: ClientIdentity) {
        self.revision = self
            .revision
            .checked_add(1)
            .expect("prototype revision exhausted");
        if let Some(state) = self.clients.get_mut(&client) {
            state.human_owned = false;
        }
        self.cancel_matching(|r| r.status == Status::Queued, CancelReason::HumanTakeover);
    }
    pub fn revoke_grant(&mut self, grant: &GrantHandle) -> Result<(), Refusal> {
        if !Arc::ptr_eq(&self.owner, &grant.owner) {
            return Err(Refusal::WrongActor);
        }
        let active = self
            .grants
            .get_mut(&grant.id)
            .ok_or(Refusal::MissingGrant)?;
        *active = false;
        self.cancel_matching(|r| r.grant == grant.id, CancelReason::GrantRevoked);
        Ok(())
    }
    pub fn end_session(&mut self) {
        self.ended = true;
        self.cancel_matching(|_| true, CancelReason::SessionEnded);
    }
    pub fn advance_to(&mut self, tick: u64) {
        assert!(
            tick >= self.clock,
            "prototype monotonic clock moved backwards"
        );
        self.clock = tick;
        self.cancel_matching(|r| r.deadline <= tick, CancelReason::Deadline);
    }
    fn cancel_matching(&mut self, predicate: impl Fn(&Record) -> bool, reason: CancelReason) {
        for record in self.records.values_mut().filter(|r| predicate(r)) {
            record.status = match record.status {
                Status::Queued => Status::Finished(Outcome::CanceledBeforeCommit(reason)),
                Status::InFlight => Status::Finished(Outcome::Unknown {
                    reason: if reason == CancelReason::Deadline {
                        UnknownReason::DeadlineAfterCommit
                    } else {
                        UnknownReason::RevokedAfterCommit
                    },
                    late_receipt: None,
                }),
                status => status,
            };
        }
    }
    pub fn receipt(&mut self, handle: &IntentHandle, receipt: Receipt) -> Result<Status, Refusal> {
        self.record(handle)?;
        let record = self.records.get_mut(&handle.id).unwrap();
        record.status = match record.status {
            Status::InFlight => Status::Finished(match receipt {
                Receipt::Acknowledged => Outcome::Acknowledged { verified: false },
                Receipt::Rejected => Outcome::RejectedAfterCommit,
            }),
            Status::Finished(Outcome::Unknown {
                reason,
                late_receipt: None,
            }) => Status::Finished(Outcome::Unknown {
                reason,
                late_receipt: Some(receipt),
            }),
            _ => return Err(Refusal::Consumed),
        };
        Ok(record.status)
    }
    pub fn reply_lost(&mut self, handle: &IntentHandle) -> Result<Status, Refusal> {
        self.record(handle)?;
        let record = self.records.get_mut(&handle.id).unwrap();
        if record.status != Status::InFlight {
            return Err(Refusal::Consumed);
        }
        record.status = Status::Finished(Outcome::Unknown {
            reason: UnknownReason::ReplyLost,
            late_receipt: None,
        });
        Ok(record.status)
    }
    pub fn observation_generation(&self, handle: &IntentHandle) -> Result<NonZeroU64, Refusal> {
        Ok(self.record(handle)?.observation_generation)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitAttempt {
    WouldBlock,
    Accepted,
    Partial,
    Disconnected,
}
mod sealed {
    pub trait Approved {}
}
/// Sealed to reviewed fixture transports. A real transport adapter must prove
/// nonblocking, bounded, no-buffered-retry behavior before being added here.
pub trait CommitTransport: sealed::Approved {
    fn route(&self) -> Route;
    fn intent_binding(&self) -> Option<&IntentHandle> {
        None
    }
    fn binding(&self) -> Option<TargetIdentity> {
        None
    }
    fn supports(&self, _mutation: &Mutation) -> bool {
        true
    }
    fn try_commit(&mut self, permit: ExternalPermit<'_>) -> CommitAttempt;
}
/// Controllable fake CDP/D-Bus transport. No sockets or desktop operations.
pub struct FixtureTransport {
    route: Route,
    mode: CommitAttempt,
    packets: Vec<Packet>,
    effects: Vec<Mutation>,
}
struct Packet {
    handle: IntentHandle,
    target: TargetIdentity,
    mutation: Mutation,
    stage: PacketStage,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PacketStage {
    Committed,
    Dispatched,
    EffectApplied,
    ReceiptReady,
    ReceiptReturned,
}
impl FixtureTransport {
    pub fn cdp() -> Self {
        Self::new(Route::Cdp)
    }
    pub fn dbus() -> Self {
        Self::new(Route::Atspi)
    }
    fn new(route: Route) -> Self {
        Self {
            route,
            mode: CommitAttempt::Accepted,
            packets: Vec::new(),
            effects: Vec::new(),
        }
    }
    pub fn set_attempt(&mut self, attempt: CommitAttempt) {
        self.mode = attempt;
    }
    pub fn committed(&self) -> usize {
        self.packets.len()
    }
    pub fn effects(&self) -> usize {
        self.effects.len()
    }
    pub fn dispatch(&mut self, index: usize) -> bool {
        let Some(packet) = self.packets.get_mut(index) else {
            return false;
        };
        if packet.stage != PacketStage::Committed {
            return false;
        }
        packet.stage = PacketStage::Dispatched;
        true
    }
    pub fn apply_effect(&mut self, index: usize) -> bool {
        let Some(packet) = self.packets.get_mut(index) else {
            return false;
        };
        if packet.stage != PacketStage::Dispatched {
            return false;
        }
        self.effects.push(packet.mutation.clone());
        packet.stage = PacketStage::EffectApplied;
        true
    }
    pub fn prepare_receipt(&mut self, index: usize) -> bool {
        let Some(packet) = self.packets.get_mut(index) else {
            return false;
        };
        if packet.stage != PacketStage::EffectApplied {
            return false;
        }
        packet.stage = PacketStage::ReceiptReady;
        true
    }
    pub fn acknowledge(&mut self, index: usize, actor: &mut Actor) -> Result<Status, Refusal> {
        let packet = self.packets.get_mut(index).ok_or(Refusal::UnknownIntent)?;
        if packet.stage != PacketStage::ReceiptReady {
            return Err(Refusal::Consumed);
        }
        let status = actor.receipt(&packet.handle, Receipt::Acknowledged)?;
        packet.stage = PacketStage::ReceiptReturned;
        Ok(status)
    }
    pub fn target(&self, index: usize) -> Option<TargetIdentity> {
        self.packets.get(index).map(|p| p.target)
    }
}
impl sealed::Approved for FixtureTransport {}
impl CommitTransport for FixtureTransport {
    fn route(&self) -> Route {
        self.route
    }
    fn try_commit(&mut self, permit: ExternalPermit<'_>) -> CommitAttempt {
        if self.packets.len() >= 64 {
            return CommitAttempt::WouldBlock;
        }
        if matches!(self.mode, CommitAttempt::Accepted | CommitAttempt::Partial) {
            self.packets.push(Packet {
                handle: permit.handle,
                target: permit.target,
                mutation: permit.mutation.clone(),
                stage: PacketStage::Committed,
            });
        }
        self.mode
    }
}

/// Fixture identity only, used by the deterministic executable and tests.
pub fn fixture_target() -> TargetIdentity {
    TargetIdentity {
        compositor_epoch: NonZeroU64::new(1).unwrap(),
        client: ClientIdentity {
            id: NonZeroU64::new(2).unwrap(),
            generation: NonZeroU64::new(3).unwrap(),
        },
        window: NonZeroU64::new(4).unwrap(),
        window_generation: NonZeroU64::new(5).unwrap(),
        process: ProcessIdentity {
            pid: NonZeroU32::new(6).unwrap(),
            start_time: NonZeroU64::new(7).unwrap(),
        },
        endpoint_generation: NonZeroU64::new(8).unwrap(),
        document_generation: NonZeroU64::new(9).unwrap(),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn setup(route: Route) -> (Actor, TargetIdentity, IntentHandle, FixtureTransport) {
        let target = fixture_target();
        let mut actor = Actor::new(target.compositor_epoch);
        actor.install_target(target).unwrap();
        let intent = prepare(&mut actor, target, route);
        (
            actor,
            target,
            intent,
            match route {
                Route::Cdp => FixtureTransport::cdp(),
                Route::Atspi => FixtureTransport::dbus(),
            },
        )
    }
    fn mutation(route: Route) -> Mutation {
        match route {
            Route::Cdp => Mutation::CdpSetValue {
                element: "field-1".into(),
                value: "fixture".into(),
            },
            Route::Atspi => Mutation::AtspiSetValue {
                object: "/fixture/field".into(),
                value: "fixture".into(),
            },
        }
    }
    fn prepare(actor: &mut Actor, target: TargetIdentity, route: Route) -> IntentHandle {
        let observed = actor.observe(target, NonZeroU64::new(1).unwrap()).unwrap();
        let granted = actor.grant(target, mutation(route), 100).unwrap();
        actor.prepare(observed, granted, 80).unwrap()
    }
    #[test]
    fn takeover_at_each_boundary_has_explicit_commit_semantics() {
        for route in [Route::Cdp, Route::Atspi] {
            // Boundary 0 = prepared; 1 = preliminary query; 2 = blocked write;
            // 3 = committed; 4 = external dispatch; 5 = effect; 6 = receipt ready.
            for boundary in 0..=6 {
                let (mut actor, target, intent, mut transport) = setup(route);
                let follow_on = prepare(&mut actor, target, route);
                if boundary >= 1 {
                    assert_eq!(actor.inspect(&intent).unwrap(), Status::Queued);
                }
                if boundary == 2 {
                    transport.set_attempt(CommitAttempt::WouldBlock);
                    assert_eq!(
                        actor
                            .try_dispatch(
                                &intent,
                                Requirement::NoTransportCommitAfterTakeover,
                                &mut transport
                            )
                            .unwrap(),
                        Status::Queued
                    );
                }
                if boundary >= 3 {
                    assert_eq!(
                        actor
                            .try_dispatch(
                                &intent,
                                Requirement::NoTransportCommitAfterTakeover,
                                &mut transport
                            )
                            .unwrap(),
                        Status::InFlight
                    );
                }
                if boundary >= 4 {
                    assert!(transport.dispatch(0));
                }
                if boundary >= 5 {
                    assert!(transport.apply_effect(0));
                }
                if boundary >= 6 {
                    assert!(transport.prepare_receipt(0));
                }
                // Human processing does not touch or wait on this transport.
                let previous_revision = actor.revision();
                actor.human_takeover(target.client);
                assert!(actor.revision() > previous_revision);
                assert_eq!(
                    actor.inspect(&follow_on).unwrap(),
                    Status::Finished(Outcome::CanceledBeforeCommit(CancelReason::HumanTakeover))
                );
                assert_eq!(
                    actor.try_dispatch(
                        &follow_on,
                        Requirement::NoTransportCommitAfterTakeover,
                        &mut transport
                    ),
                    Err(Refusal::Consumed)
                );
                if boundary < 3 {
                    assert_eq!(transport.committed(), 0);
                    assert_eq!(
                        actor.inspect(&intent).unwrap(),
                        Status::Finished(Outcome::CanceledBeforeCommit(
                            CancelReason::HumanTakeover
                        ))
                    );
                    assert_eq!(
                        actor.try_dispatch(
                            &intent,
                            Requirement::NoTransportCommitAfterTakeover,
                            &mut transport
                        ),
                        Err(Refusal::Consumed)
                    );
                } else {
                    assert_eq!(
                        actor.inspect(&intent).unwrap(),
                        Status::Finished(Outcome::Unknown {
                            reason: UnknownReason::RevokedAfterCommit,
                            late_receipt: None
                        })
                    );
                    if boundary < 4 {
                        assert!(transport.dispatch(0));
                    }
                    if boundary < 5 {
                        assert!(transport.apply_effect(0));
                    }
                    if boundary < 6 {
                        assert!(transport.prepare_receipt(0));
                    }
                    assert_eq!(
                        transport.acknowledge(0, &mut actor).unwrap(),
                        Status::Finished(Outcome::Unknown {
                            reason: UnknownReason::RevokedAfterCommit,
                            late_receipt: Some(Receipt::Acknowledged)
                        })
                    );
                    assert_eq!(transport.effects(), 1);
                    assert_eq!(
                        actor.try_dispatch(
                            &intent,
                            Requirement::NoTransportCommitAfterTakeover,
                            &mut transport
                        ),
                        Err(Refusal::Consumed)
                    );
                }
            }
        }
    }
    #[test]
    fn target_changes_in_every_identity_dimension_cancel_queued_and_inflight() {
        for field in 0..7 {
            let (mut actor, target, intent, mut transport) = setup(Route::Cdp);
            let flight = prepare(&mut actor, target, Route::Cdp);
            actor
                .try_dispatch(
                    &flight,
                    Requirement::NoTransportCommitAfterTakeover,
                    &mut transport,
                )
                .unwrap();
            let mut changed = target;
            let next = NonZeroU64::new(99).unwrap();
            match field {
                0 => changed.client.id = next,
                1 => changed.client.generation = next,
                2 => changed.window_generation = next,
                3 => changed.process.pid = NonZeroU32::new(99).unwrap(),
                4 => changed.process.start_time = next,
                5 => changed.endpoint_generation = next,
                6 => changed.document_generation = next,
                _ => unreachable!(),
            }
            actor.install_target(changed).unwrap();
            assert_eq!(
                actor.inspect(&intent).unwrap(),
                Status::Finished(Outcome::CanceledBeforeCommit(CancelReason::TargetChanged))
            );
            assert_eq!(
                actor.inspect(&flight).unwrap(),
                Status::Finished(Outcome::Unknown {
                    reason: UnknownReason::RevokedAfterCommit,
                    late_receipt: None
                })
            );
            assert_eq!(
                actor.observe(target, next).unwrap_err(),
                Refusal::StaleTarget
            );
        }
    }
    #[test]
    fn stamps_and_handles_do_not_cross_actor_or_restart() {
        let (mut actor, target, intent, _) = setup(Route::Cdp);
        let mut other = Actor::new(target.compositor_epoch);
        other.install_target(target).unwrap();
        assert_eq!(other.inspect(&intent), Err(Refusal::WrongActor));
        let observed = actor.observe(target, NonZeroU64::new(1).unwrap()).unwrap();
        let grant = other.grant(target, mutation(Route::Cdp), 100).unwrap();
        assert_eq!(
            actor.prepare(observed, grant, 80).unwrap_err(),
            Refusal::WrongActor
        );
        let mut different_epoch = target;
        different_epoch.compositor_epoch = NonZeroU64::new(2).unwrap();
        assert_eq!(
            actor.install_target(different_epoch),
            Err(Refusal::StaleTarget)
        );
    }
    #[test]
    fn revocation_and_session_end_cancel_without_touching_transport() {
        let target = fixture_target();
        let mut actor = Actor::new(target.compositor_epoch);
        actor.install_target(target).unwrap();
        let stamp = actor.observe(target, NonZeroU64::new(1).unwrap()).unwrap();
        let grant = actor.grant(target, mutation(Route::Atspi), 100).unwrap();
        let revoke = grant.revocation_handle();
        let intent = actor.prepare(stamp, grant, 90).unwrap();
        actor.revoke_grant(&revoke).unwrap();
        assert_eq!(
            actor.inspect(&intent).unwrap(),
            Status::Finished(Outcome::CanceledBeforeCommit(CancelReason::GrantRevoked))
        );
        let follow = prepare(&mut actor, target, Route::Atspi);
        actor.end_session();
        assert_eq!(
            actor.inspect(&follow).unwrap(),
            Status::Finished(Outcome::CanceledBeforeCommit(CancelReason::SessionEnded))
        );
    }
    #[test]
    fn partial_write_and_lost_reply_never_retry() {
        let (mut actor, _, intent, mut transport) = setup(Route::Cdp);
        transport.set_attempt(CommitAttempt::Partial);
        assert_eq!(
            actor
                .try_dispatch(
                    &intent,
                    Requirement::NoTransportCommitAfterTakeover,
                    &mut transport
                )
                .unwrap(),
            Status::Finished(Outcome::Unknown {
                reason: UnknownReason::PartialWrite,
                late_receipt: None
            })
        );
        assert_eq!(
            actor.try_dispatch(
                &intent,
                Requirement::NoTransportCommitAfterTakeover,
                &mut transport
            ),
            Err(Refusal::Consumed)
        );
        assert_eq!(transport.committed(), 1);
        let (mut actor, _, intent, mut transport) = setup(Route::Atspi);
        actor
            .try_dispatch(
                &intent,
                Requirement::NoTransportCommitAfterTakeover,
                &mut transport,
            )
            .unwrap();
        assert_eq!(
            actor.reply_lost(&intent).unwrap(),
            Status::Finished(Outcome::Unknown {
                reason: UnknownReason::ReplyLost,
                late_receipt: None
            })
        );
        assert_eq!(
            actor.try_dispatch(
                &intent,
                Requirement::NoTransportCommitAfterTakeover,
                &mut transport
            ),
            Err(Refusal::Consumed)
        );
    }
    #[test]
    fn blocked_transport_is_revalidated_before_retry() {
        let (mut actor, target, intent, mut transport) = setup(Route::Atspi);
        transport.set_attempt(CommitAttempt::WouldBlock);
        actor
            .try_dispatch(
                &intent,
                Requirement::NoTransportCommitAfterTakeover,
                &mut transport,
            )
            .unwrap();
        actor.human_takeover(target.client);
        actor.human_release(target.client);
        transport.set_attempt(CommitAttempt::Accepted);
        assert_eq!(
            actor.try_dispatch(
                &intent,
                Requirement::NoTransportCommitAfterTakeover,
                &mut transport
            ),
            Err(Refusal::Consumed)
        );
        assert_eq!(transport.committed(), 0);
        let fresh = prepare(&mut actor, target, Route::Atspi);
        assert_eq!(
            actor
                .try_dispatch(
                    &fresh,
                    Requirement::NoTransportCommitAfterTakeover,
                    &mut transport
                )
                .unwrap(),
            Status::InFlight
        );
    }
    #[test]
    fn deadlines_distinguish_queued_from_already_committed() {
        let (mut actor, target, intent, mut transport) = setup(Route::Cdp);
        let queued = prepare(&mut actor, target, Route::Cdp);
        actor
            .try_dispatch(
                &intent,
                Requirement::NoTransportCommitAfterTakeover,
                &mut transport,
            )
            .unwrap();
        actor.advance_to(81);
        assert_eq!(
            actor.inspect(&queued).unwrap(),
            Status::Finished(Outcome::CanceledBeforeCommit(CancelReason::Deadline))
        );
        assert_eq!(
            actor.inspect(&intent).unwrap(),
            Status::Finished(Outcome::Unknown {
                reason: UnknownReason::DeadlineAfterCommit,
                late_receipt: None
            })
        );
    }
    #[test]
    fn stronger_effect_ordering_and_wrong_routes_refuse_before_commit() {
        let (mut actor, _, intent, mut transport) = setup(Route::Cdp);
        assert_eq!(
            actor.try_dispatch(
                &intent,
                Requirement::NoApplicationEffectAfterTakeover,
                &mut transport
            ),
            Err(Refusal::UnsupportedOrdering)
        );
        let mut wrong = FixtureTransport::dbus();
        assert_eq!(
            actor.try_dispatch(
                &intent,
                Requirement::NoTransportCommitAfterTakeover,
                &mut wrong
            ),
            Err(Refusal::WrongRoute)
        );
        assert_eq!(transport.committed() + wrong.committed(), 0);
    }
    #[test]
    fn successful_receipt_is_delivery_not_ui_verification() {
        let (mut actor, _, intent, mut transport) = setup(Route::Cdp);
        actor
            .try_dispatch(
                &intent,
                Requirement::NoTransportCommitAfterTakeover,
                &mut transport,
            )
            .unwrap();
        assert!(transport.dispatch(0));
        assert!(transport.apply_effect(0));
        assert!(transport.prepare_receipt(0));
        let result = transport.acknowledge(0, &mut actor).unwrap();
        assert_eq!(
            result,
            Status::Finished(Outcome::Acknowledged { verified: false })
        );
        if let Status::Finished(outcome) = result {
            assert!(outcome.requires_fresh_observation());
        }
        assert_eq!(transport.acknowledge(0, &mut actor), Err(Refusal::Consumed));
    }
    #[test]
    fn committed_and_ambiguous_effects_retire_prior_observation_generations() {
        for attempt in [CommitAttempt::Accepted, CommitAttempt::Partial] {
            let (mut actor, target, intent, mut transport) = setup(Route::Cdp);
            let old = actor.observe(target, NonZeroU64::new(1).unwrap()).unwrap();
            let grant = actor.grant(target, mutation(Route::Cdp), 100).unwrap();
            let queued = prepare(&mut actor, target, Route::Cdp);
            transport.set_attempt(attempt);
            actor
                .try_dispatch(
                    &intent,
                    Requirement::NoTransportCommitAfterTakeover,
                    &mut transport,
                )
                .unwrap();
            assert_eq!(
                actor.prepare(old, grant, 80).unwrap_err(),
                Refusal::StaleObservation
            );
            assert_eq!(
                actor.try_dispatch(
                    &queued,
                    Requirement::NoTransportCommitAfterTakeover,
                    &mut transport
                ),
                Err(Refusal::StaleObservation)
            );
            assert_eq!(
                actor
                    .observe(target, NonZeroU64::new(1).unwrap())
                    .unwrap_err(),
                Refusal::StaleObservation
            );
            assert_eq!(
                actor
                    .observe(target, NonZeroU64::new(2).unwrap())
                    .unwrap_err(),
                Refusal::PendingEffect
            );
        }
    }
    #[test]
    fn cross_window_grant_and_removed_window_are_refused() {
        let (mut actor, target, intent, _) = setup(Route::Atspi);
        let mut other = target;
        other.window = NonZeroU64::new(10).unwrap();
        actor.install_target(other).unwrap();
        let stamp = actor.observe(target, NonZeroU64::new(1).unwrap()).unwrap();
        let grant = actor.grant(other, mutation(Route::Atspi), 100).unwrap();
        assert_eq!(
            actor.prepare(stamp, grant, 80).unwrap_err(),
            Refusal::StaleTarget
        );
        actor.remove_target(target.window);
        assert_eq!(
            actor.inspect(&intent).unwrap(),
            Status::Finished(Outcome::CanceledBeforeCommit(CancelReason::TargetChanged))
        );
        assert_eq!(
            actor
                .observe(target, NonZeroU64::new(2).unwrap())
                .unwrap_err(),
            Refusal::StaleTarget
        );
    }
    #[test]
    fn unresolved_inflight_effect_blocks_new_actionable_observation_until_receipt() {
        let (mut actor, target, intent, mut transport) = setup(Route::Atspi);
        actor
            .try_dispatch(
                &intent,
                Requirement::NoTransportCommitAfterTakeover,
                &mut transport,
            )
            .unwrap();
        actor.human_takeover(target.client);
        actor.human_release(target.client);
        assert_eq!(
            actor
                .observe(target, NonZeroU64::new(2).unwrap())
                .unwrap_err(),
            Refusal::PendingEffect
        );
        transport.dispatch(0);
        transport.apply_effect(0);
        assert!(transport.prepare_receipt(0));
        transport.acknowledge(0, &mut actor).unwrap();
        assert!(actor.observe(target, NonZeroU64::new(2).unwrap()).is_ok());
    }
    #[test]
    fn committing_one_window_retires_prior_observations_for_other_windows() {
        let (mut actor, target, intent, mut transport) = setup(Route::Cdp);
        let mut other = target;
        other.window = NonZeroU64::new(12).unwrap();
        actor.install_target(other).unwrap();
        let old = actor.observe(other, NonZeroU64::new(1).unwrap()).unwrap();
        let grant = actor.grant(other, mutation(Route::Cdp), 100).unwrap();
        actor
            .try_dispatch(
                &intent,
                Requirement::NoTransportCommitAfterTakeover,
                &mut transport,
            )
            .unwrap();
        assert_eq!(
            actor.prepare(old, grant, 80).unwrap_err(),
            Refusal::StaleObservation
        );
    }
}

pub mod semantic;
