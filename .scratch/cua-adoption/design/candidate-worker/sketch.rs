//! DESIGN ONLY. All bodies are intentionally not implemented.
//! Names not already in the repository describe proposed domain types.

use std::sync::Arc;
use std::time::{Duration, Instant};

// Public typed contract. No CUA IPC/storage/native runtime types are reexported.
pub struct VisualParseRequest {
    pub capture_id: CaptureId,
    pub options: VisualOptions,
}
pub struct VisualOptions {
    kinds: RegionKinds,
    minimum_confidence: UnitConfidence,
    maximum_regions: PositiveRegionLimit,
}
pub struct VisualParseOutput {
    pub capture_id: CaptureId,
    pub screenshot: ScreenshotIdentity,
    pub provenance: CaptureProvenance,
    pub parser: ParserIdentity,
    pub regions: Vec<NonInteractiveRegion>,
    pub coverage: PerceptionCoverage,
    pub usage: PerceptionUsage,
    // Schema derives constant false fields, not mutable boolean authority.
    pub authorized: NoAuthorization,
    pub capture_consumed: NotConsumed,
}
pub struct NonInteractiveRegion {
    pub kind: VisualKind,
    pub bounds: ImagePixelRect,
    pub confidence: UnitConfidence,
    pub label: Option<BoundedText>,
    pub interactive: NonInteractive,
}
pub enum VisualKind { Text, Icon }
pub struct ScreenshotIdentity {
    digest: EncodedPngSha256,
    width: NonZeroImageWidth,
    height: NonZeroImageHeight,
}
pub struct CaptureProvenance {
    source: DojoCaptureSource, // named output, optional exact window identity
    acquired_at: ObservationTime,
    image_to_logical: AffineImageMapping,
    coverage: CapturedSurfaceCoverage,
}
pub struct ParserIdentity {
    worker_source: SourceRevision,
    worker_binary: Sha256,
    manifest: Sha256,
    artifact_subjects: ModelArtifactDigests,
    onnx_runtime: RuntimeIdentity,
    dependency_closure: ImmutableClosureIdentity,
}
pub enum VisualParseFailure {
    CaptureUnknown,
    CaptureExpired,
    CaptureRetired,
    SessionClosed,
    DeniedDisclosure,
    NotInstalled,
    UnsupportedTarget,
    UnsupportedContainment,
    ArtifactInvalid,
    ArtifactNotImmutable,
    ResourceLimit,
    Busy,
    Cancelled,
    DeadlineExceeded,
    WorkerCrash,
    ProtocolInvalid,
}

// Application owns backend configuration. Opening is inert: no warm worker.
pub struct PerceptionBackend {
    bundle: TrustedWorkerBundle,
    limits: WorkerLimits,
    capacity: WorkerCapacity,
}
impl PerceptionBackend {
    pub fn open(bundle: TrustedWorkerBundle, limits: WorkerLimits)
        -> Result<Self, VisualParseFailure> {
        todo!("not implemented: verify installation/closure without launching")
    }
}

impl CaptureSession {
    pub async fn parse_visual_regions(
        &self,
        backend: &PerceptionBackend,
        request: VisualParseRequest,
        context: ReadRequestContext,
    ) -> Result<VisualParseOutput, VisualParseFailure> {
        todo!("not implemented: session-bound immutable read, child parse, completion gate")
    }

    // Private seam. No public arbitrary-image or caller-binding runner exists.
    fn begin_perception_read(
        &self,
        capture_id: CaptureId,
        context: &ReadRequestContext,
    ) -> Result<ReadImageTicket, VisualParseFailure> {
        todo!("not implemented: policy, binding/TTL lookup and memory reservation")
    }
    fn complete_perception_read(
        &self,
        ticket: ReadImageTicket,
        worker: ValidatedWorkerOutput,
        context: &ReadRequestContext,
    ) -> Result<VisualParseOutput, VisualParseFailure> {
        todo!("not implemented: atomic live-session/current-binding/capture completion check")
    }
}

// Public opaque context: fields and constructor are private to the trusted
// server integration; its adapter exposes session.read_context(request).
// CaptureSession derives its binding internally, never from tool JSON.
// The server alone constructs this context. Connection cancellation and request
// cancellation are live signals, not a freshly allocated default worker token.
pub struct ReadRequestContext {
    session: LiveSessionReadGuard,
    request: ReadRequestId,
    cancellation: SessionAndRequestCancellation,
    deadline: Instant,
    disclosure: ReadDisclosureContext,
}
struct ReadImageTicket {
    binding: PrivateCaptureBinding,
    capture_id: CaptureId,
    image: Arc<[u8]>,
    identity: ScreenshotIdentity,
    provenance: CaptureProvenance,
    valid_until: CaptureClockExpiry,
    deadline: Instant,
    reservation: HeldImageAndWireBudget,
}

// One child per request. This is IO/process ownership, not a task scheduler.
struct WorkerCapacity { available: AtomicCapacity }
struct WorkerPermit { release_on_drop: CapacityReservation }
struct OwnedWorker {
    child: ContainedProcess,
    group: VerifiedOwnedProcessGroup,
    protocol: PrivateFramedProtocol,
    private_work_directory: PrivateWorkDirectory,
    permit: WorkerPermit,
}
impl OwnedWorker {
    async fn parse_once(
        bundle: &TrustedWorkerBundle,
        ticket: &ReadImageTicket,
        cancellation: &SessionAndRequestCancellation,
        limits: &WorkerLimits,
        permit: WorkerPermit,
    ) -> Result<ValidatedWorkerOutput, VisualParseFailure> {
        todo!("not implemented: cold launch, health, exactly one parse, bounded exit/reap")
    }
    async fn cancel_and_reap(&mut self, deadline: Instant) -> CleanupOutcome {
        todo!("not implemented: kill owned group, wait/reap, retain charge until settled")
    }
}

// Only a trusted host packaging path constructs this type. Model/tool JSON
// cannot manufacture trusted files or broaden Landlock read/execute rights.
pub struct TrustedWorkerBundle {
    identity: ParserIdentity,
    executable: ImmutableExecutable,
    manifest: ImmutableManifest,
    artifacts: ImmutableArtifactSet,
    dependencies: ExactReadAndExecuteClosure,
}
impl TrustedWorkerBundle {
    pub fn from_installation(
        installation: HostInstallation,
        expected: PinnedArtifactAndBuildIdentity,
    ) -> Result<Self, VisualParseFailure> {
        todo!("not implemented: canonical immutable Nix members, expected hashes, exact closure")
    }
}
pub struct WorkerLimits {
    maximum_request: Duration,
    cleanup_reserve: Duration,
    maximum_processes: PositiveProcessLimit,
    maximum_held_image_and_wire_bytes: Bytes,
    image_and_frame: PinnedWorkerProtocolLimits,
    process: PinnedContainmentResourceLimits,
}
