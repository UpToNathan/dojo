//! Candidate A: Niri owns lazy activation and service-generation lifetime.
//! This is a type/signature sketch; bodies are intentionally not implemented.

// vendor/niri/src/external_admission.rs

/// Immutable operator enrollment loaded at compositor startup. This authorizes
/// Niri to launch the service; it does not itself authorize a navigation.
struct ServiceEnrollment {
    artifact: EnrolledArtifact, // identity/provenance format is unresolved
    policy_digest: PolicyDigest,
}

/// A single compositor-owned child generation. pidfd and control-channel
/// lifetime are both revocation signals; compositor generation prevents stale
/// messages after restart.
struct Activation {
    id: ActivationId,
    compositor: CompositorGeneration,
    child: ProcessGeneration,
    pidfd: OwnedFd,
    control: ActivationControl,
    sessions: HashMap<SessionId, SessionRegistration>,
    peers: HashMap<AgentPeerId, PeerBinding>,
    writers: HashMap<WriterId, PendingWriter>,
}

/// Credential provenance is captured at IPC accept, never from JSON.
struct PeerBinding {
    peer: AgentPeerId,
    owner: ProcessGeneration,
    session: SessionId,
    alive: PeerLiveness,
}

struct SessionRegistration {
    id: SessionId,
    activation: ActivationId,
    peer: AgentPeerId,
    live: bool,
}

/// The private activation channel is a capability to use this generation.
/// It is non-cloneable in the service and cannot register a second process.
struct ActivationControl { /* private socket endpoint + generation nonce */ }

/// Domain request parsed from the bounded private wire protocol.
struct PreparedPage {
    intent: WriterIntentId,
    pi_intent: PiIntentId,
    target: ExactPageTarget,
    native: NativeWindowBinding,
    browser: BrowserProcessGeneration,
    endpoint: VerifiedLoopbackEndpoint,
    peer: AgentPeerId,
    session: SessionId,
    fd: OwnedFd,
    expiry: Instant,
}

/// Actor state owns the only writable pending source. No service-side writer.
struct PendingWriter {
    activation: ActivationId,
    peer: AgentPeerId,
    session: SessionId,
    target: TargetKey,
    fd: OwnedFd,
    expiry: Instant,
    state: WriterState, // Ready | Unknown | RefusedBeforeWrite
}

impl Niri {
    /// Starts from the operator-enrolled artifact on first demand, or returns
    /// the existing live activation. Never accepts caller-supplied executable.
    async fn activate_dojo_service(&mut self) -> Result<ServiceEndpoint, ActivationError>;

    /// Accept one registration/session only over the child capability channel;
    /// cross-check kernel peer credentials, process start, activation ID and
    /// compositor generation. Idempotent for the same live session nonce.
    fn register_service_session(
        &mut self,
        control: &ActivationControl,
        observed: KernelPeer,
        hello: BoundedServiceHello,
    ) -> Result<SessionId, RegistrationError>;

    /// Submit the single transferred FD under its owning live session/peer.
    fn admit_prepared_page(
        &mut self,
        activation: ActivationId,
        observed: KernelPeer,
        request: BoundedPreparedPage,
        fd: OwnedFd,
    ) -> Result<WriterReceipt, AdmissionError>;

    /// Unified actor teardown: mark activation/peer/session dead first; remove
    /// every matching writable source and close FDs before acknowledging loss.
    fn revoke_scope(&mut self, scope: RevocationScope) -> RevocationReport;
}

enum RevocationScope {
    Activation(ActivationId),
    Session(SessionId),
    Peer(AgentPeerId),
    Process(ProcessGeneration),
}

// src/bin/dojo-agent/service.rs
impl ActiveSession {
    async fn connect_or_activate(
        compositor_socket: SocketPath,
        policy: PolicyIdentity,
    ) -> Result<Self, ServiceError>;
}

impl Service {
    async fn navigate(
        &mut self,
        request: BrowserNavigateRequest,
        cancel: CancellationToken,
    ) -> Result<NavigationReceipt, NavigationError>;
}

// crates/dojo-external-admission/src/broker.rs
/// Prepare exact page connection, transfer once, and regard receipt loss after
/// transfer as Unknown. Cancellation before transfer prevents handoff; after
/// handoff it is sent as actor revocation and does not mean rollback.
async fn navigate_cancellable(
    target: ExactPageTarget,
    binding: BrowserWindowBinding,
    grant: SingleUseCaptureGrant,
    activation: &ActivationClient,
    cancel: CancellationToken,
) -> NavigationReceipt;

enum NavigationReceipt {
    RefusedBeforeWrite(Refusal),
    Unknown(UnknownCorrelation),
}

// Pi's intent ID is correlation only. It is not a token that grants consent.
struct PiIntentId(NonEmptyBoundedString);
