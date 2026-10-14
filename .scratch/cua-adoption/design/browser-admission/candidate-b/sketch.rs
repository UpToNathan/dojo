//! Design only. Opaque fields represent boundary-validated values, not wire structs.
//! Public callers do not choose launch modes, pass broker PIDs, or handle SCM_RIGHTS.
use std::{path::Path, time::Duration, sync::atomic::AtomicBool};

type Result<T> = std::result::Result<T, Error>;
struct Error;
struct ObserveInput;
struct Observation;
struct PageBinding;
struct AdmittedAction;
struct NiriSocket;
struct ProcessGeneration { pid: u32, start: u64, pidfd: OwnedPidFd }
struct OwnedPidFd;
struct CompositorGeneration { process: ProcessGeneration, nonce: [u8; 32] }
struct RecipeDigest([u8; 32]);
struct PolicyDigest([u8; 32]);
struct LaunchRecipe {
    executable: OpenedImmutableElf,
    argv: FixedArgv,
    environment: FixedEnvironment,
    installation: ImmutableInstallation,
    policy: PolicyDigest,
    digest: RecipeDigest,
}
struct OpenedImmutableElf;
struct FixedArgv;
struct FixedEnvironment;
struct ImmutableInstallation;

/// Constructed only by explicit operator enrollment; config parsing cannot mint it.
/// Authorizes this live launcher to start this recipe, with no caller-selected mode.
struct EnrolledLauncher {
    process: ProcessGeneration,
    compositor: CompositorGeneration,
    recipe: LaunchRecipe,
    channel: AuthenticatedLauncherChannel,
}
struct AuthenticatedLauncherChannel;

/// Lazy launch/connect coordination; one attester child slot per compositor.
/// No durable state or effect scheduling. Lost child/start replies are reconciled
/// through the same child slot, never by launching a second concurrent generation.
struct LazyDesktop;
struct DesktopSession;
impl LazyDesktop {
    fn connect(socket: &Path, name: &str, idle: Duration) -> Result<DesktopSession> {
        // TODO authenticate compositor and enrolled launcher endpoint; request the
        // fixed child slot. Validate service socket peer against that attested child.
        // Socket names and Hello.service_pid are discovery, never identity proof.
        unimplemented!("not implemented")
    }
}
impl DesktopSession {
    fn browser_observe(&mut self, input: ObserveInput) -> Result<Observation> {
        unimplemented!("not implemented")
    }
}

struct ActivationKey([u8; 32]);
struct PeerKey { compositor: CompositorGeneration, serial: u64 }
struct ScopeKey { activation: ActivationKey, peer: PeerKey, serial: u64 }
struct SessionLifetime { scope: ScopeKey, seat: AuthenticatedSeatConnection }
struct AuthenticatedSeatConnection;
struct IntentId([u8; 16]);
struct CaptureAuthority;
struct ExactNativeWindow;
struct ExactPageTarget;
struct ListenerOwner;

/// Private constructor consumes capture and read episode authority once.
/// Native, browser-root, and endpoint-listener identities stay separate.
struct PreparedNavigation {
    intent: IntentId,
    capture: CaptureAuthority,
    native: ExactNativeWindow,
    page: ExactPageTarget,
    listener: ListenerOwner,
    url: ValidNavigationUrl,
}
struct ValidNavigationUrl;
impl PreparedNavigation {
    fn from_observation(page: PageBinding, admitted: AdmittedAction, url: String)
        -> Result<Self> {
        // TODO validate exact supported topology; retire read-only connections;
        // consume capture and mint Rust one-use authority, never model-supplied proof.
        unimplemented!("not implemented")
    }
}

/// One call hides authentication, exact upgrade, unique ownership transfer,
/// receipt correlation and post-handoff cancellation. This owns no durable intent.
struct BrowserAdmission { activation: LiveActivation }
struct LiveActivation;
enum NavigationOutcome {
    RefusedZero(AuthenticatedZeroDispatch),
    Unknown(AttemptIdentity),
}
struct AuthenticatedZeroDispatch { attempt: AttemptIdentity }
struct AttemptIdentity { scope: ScopeKey, intent: IntentId }
impl BrowserAdmission {
    fn navigate(&mut self, session: &SessionLifetime, prepared: PreparedNavigation,
                cancelled: &AtomicBool) -> Result<NavigationOutcome> {
        // TODO ensure activation/seat/kernel owner match; establish per-scope ingress.
        // Authenticate compositor. Upgrade only the exact /devtools/page/{target}.
        // Reject protocol extensions, TLS and read-ahead. Own exactly one writable FD.
        // Check cancellation before transfer. sendmsg hands off; immediately close
        // local copy, including ambiguous transfer failure; never reconnect/retry.
        // After possible handoff ask actor to revoke, await bounded correlated ack.
        // Any lost/malformed receipt or cancellation ack => Unknown, not RefusedZero.
        unimplemented!("not implemented")
    }
}

// All following types remain private to Niri's external_admission module.
struct AdmissionActor {
    launcher: EnrolledLauncher,
    activation: Option<RegisteredGeneration>,
    seen: BurnedIntents,
    unknown: UnresolvedTargets,
}
struct RegisteredGeneration {
    key: ActivationKey,
    process: ProcessGeneration,
    connection: AuthenticatedGenerationChannel,
    scopes: BoundedScopes,
}
struct AuthenticatedGenerationChannel;
struct BoundedScopes;
struct BurnedIntents;
struct UnresolvedTargets;
struct Scope {
    key: ScopeKey,
    owner: ProcessGeneration,
    ingress: OwnedSeqpacket,
    pending: BoundedWriters,
}
struct OwnedSeqpacket;
struct BoundedWriters;
struct UniquePageWriter;
struct ActorTargetAuthority;
struct ActorTurn;
struct LauncherAttestation;
struct KernelPacket;
enum Revocation { Cancel, PeerClosed, SeatExpired, HumanTakeover, ServiceDead, LauncherLost }
enum PendingState { Ready(UniquePageWriter), Attempted(AttemptIdentity), Retired }
impl AdmissionActor {
    fn enroll_child(&mut self, attested: LauncherAttestation, turn: &mut ActorTurn)
        -> Result<()> {
        // TODO validate pinned launcher's per-message SCM creds/start/pidfd and live
        // control connection, recipe/compositor nonce; child's Hello SCM creds and
        // pidfd/start must match launcher-spawned generation. Revoke old generation
        // and close all channels/sources before installing new generation.
        // seen and unknown belong to this actor, never to replaceable registration.
        unimplemented!("not implemented")
    }
    fn bind_scope(&mut self, peer: &AuthenticatedSeatConnection, turn: &mut ActorTurn)
        -> Result<ScopeKey> {
        // TODO verify IPC SO_PEERCRED captured at accept plus start/pidfd identity.
        // Peer must be live and owned by exact enrolled service generation. Allocate
        // actor-scoped serial and exclusive ingress pair, deliver child end over the
        // authenticated generation channel. Caller cannot claim another peer serial.
        unimplemented!("not implemented")
    }
    fn accept(&mut self, packet: KernelPacket, scope: ScopeKey, turn: &mut ActorTurn)
        -> Result<()> {
        // TODO receive MSG_CMSG_CLOEXEC and bounded one-FD packet, validate credentials.
        // Burn accepted immutable intent; capacity exhaustion refuses new intents.
        // Validate live scope, exact native/page attestation, listener owner/inode,
        // TCP loopback tuple, browser generation, capture geometry and authority.
        // Install writer + expiry under this scope only. Drop every rejected FD.
        unimplemented!("not implemented")
    }
    fn ready(&mut self, attempt: AttemptIdentity, turn: &mut ActorTurn) {
        // TODO on compositor actor: recheck scope/peer/process/activation liveness,
        // unresolved target, coordination/geometry/revision/browser generation.
        // Construct typed Page.navigate only; one send DONTWAIT|NOSIGNAL.
        // EAGAIN/EINTR accepting no bytes may rearm under the same pending attempt,
        // every later turn repeats checks. Mark any actual dispatch Unknown.
        // Never send a partial remainder. Shutdown/drop socket and all timer sources.
        unimplemented!("not implemented")
    }
    fn revoke(&mut self, scope: ScopeKey, reason: Revocation) {
        // TODO actor marks scope invalid first, closes ingress to release all kernel
        // queued SCM_RIGHTS, removes writer/expiry sources and closes received FDs.
        // RefusedZero only for exact intents with no attempted send. Attempted/lost
        // outcomes remain unknown. Duplicate revoke succeeds without restoring scope.
        unimplemented!("not implemented")
    }
}
