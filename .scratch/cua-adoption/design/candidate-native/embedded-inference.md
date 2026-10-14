# Embedded Rust perception candidate

## Usage (caller's view)

`dojo-agent` keeps the capture session as the authority. A read-only request parses one retained capture; it never accepts image bytes or a path from Pi, consumes the capture, or enables an action.

```rust
let result = session.parse_visual_regions(
    &capture_id,
    VisualOptions { kinds: RegionKinds::ALL, min_confidence: 0.25, max_regions: 64 },
    request_cancel.child_token(),
).await?;
// result is bound to the capture and contains image-pixel regions with interactive=false.
```

Pi's registered `desktop_parse_visual_regions` tool is a thin adapter to that call. The extension classifies it as a read-only replay-safe tool. Pi still owns scheduling, model calls, durable task state, and result storage.

A caller that tries to use regions to act must go through the existing independent admission path. This parser returns no grant, element token, or mutation authority.

Durable approval and mutation admission remain deferred to the separate admission design (issue 16). A human response or future grant cannot make these regions actionable by itself.

## Problem

Dojo already stores immutable PNGs behind session-bound capture IDs and can read them without consuming them. The missing capability is model inference on those pixels. An embedded runtime avoids a worker executable and IPC lifecycle, but runs ONNX code inside the long-lived desktop service. It must still honor request cancellation, capture expiry, bounded resource use, and trusted Nix artifact identity. The pinned CUA revision is `88aff7e5424e9ed45b93ded85eb4111472ca5190`; source exploration found no model weights or ONNX runtime library in the checkout, so this sketch does not claim production inference is currently runnable.

## Shape

### Data first

```rust
// Public domain request; no transport or runtime types escape.
pub struct VisualOptions {
    pub kinds: RegionKinds,
    pub min_confidence: f32, // validated finite and in 0..=1 at MCP boundary
    pub max_regions: NonZeroUsize, // capped by service policy
}

pub struct VisualParse {
    pub capture: CaptureIdentity, // id + digest + delivered dimensions
    pub parser: TrustedParserIdentity, // generated from immutable artifact lock
    pub regions: Vec<VisualRegion>, // delivered-image pixels; always noninteractive
    pub coordinate_map: CaptureCoordinateMap, // server-owned provenance
}

struct InferenceBundle {
    identity: TrustedParserIdentity,
    runtime: ort::Environment,
    detector: ort::Session,
    text_detector: ort::Session,
    recognizer: ort::Session,
    artifacts: VerifiedBundle, // read-only Nix closure, opened and hashed once
}
```

`CaptureIdentity` and coordinate mapping reuse Dojo capture domain types. Worker/model identity is built at packaging time from the trusted manifest digest, each artifact digest, ONNX Runtime version/library digest, and extension version. The service does not trust identity echoed by the model. Model files and runtime are immutable, read-only paths in the exact Nix closure; no user-selected model path, download-on-demand, Python, or mutable cache is consulted at runtime. Deployment fails closed if the packaged closure's generated identity or runtime checks differ.

### Single public operation

```rust
impl CaptureSession {
    /// Resolve under this session's private binding, infer, then recheck expiry.
    pub async fn parse_visual_regions(
        &self,
        id: &CaptureId,
        options: VisualOptions,
        cancel: CancellationToken,
    ) -> Result<VisualParse, PerceptionError>; // not implemented
}

impl PerceptionEngine {
    /// One bounded parse; implementation must support interrupting an in-flight
    /// ONNX run and joining it before returning Cancelled/Deadline.
    async fn parse(
        &self,
        capture: PerceptionCapture,
        options: VisualOptions,
        deadline: Instant,
        cancel: CancellationToken,
    ) -> Result<RawRegions, EngineError>; // not implemented
}
```

`CaptureSession::parse_visual_regions` computes `deadline = min(request deadline, capture expiry, service maximum)`, resolves `read_for_perception` using its private binding, and clones only the immutable PNG `Arc`. It never renews TTL or seat/session leases. It invokes the single process-owned inference bundle through a bounded semaphore (one active parse initially; a small fixed queue). The inference module decodes PNG, performs the pinned detector/OCR transforms and decoders, and returns raw image-pixel regions. The owner validates bounds, finite scores, region count, digest, and dimensions, maps provenance from the capture, then checks cancellation and capture liveness/expiry again before publishing. If expiry or cancellation wins, no result escapes. A held `Arc` cannot be revoked, so the in-flight deadline is mandatory even after registry retirement.

The ONNX backend is allowed only if its `RunOptions` termination can interrupt every detector and recognizer call and the caller can join the inference task within the service deadline. A timed-out caller may not detach a runaway task while releasing its memory budget. If this property cannot be demonstrated on the pinned Linux/Nix runtime, this candidate fails and must not ship; there is no soft-timeout fallback.

### Module map

```text
crates/dojo-computer/src/lib.rs
  CaptureSession::parse_visual_regions   // owns binding, expiry and final freshness check
crates/dojo-computer-perception/src/lib.rs
  PerceptionEngine                       // narrow typed parse surface
crates/dojo-computer-perception/src/artifacts.rs
  generated immutable identity + closure validation (no mutable fetch/cache)
crates/dojo-computer-perception/src/runtime.rs
  process-owned ONNX sessions, cancelable runs, fixed concurrency/memory bounds
crates/dojo-computer-perception/src/preprocess.rs
  pinned image/tensor transforms and inverse geometry
crates/dojo-computer-perception/src/postprocess.rs
  detector/OCR decode, CTC, NMS, canonical noninteractive regions
src/bin/dojo-agent/service.rs
  request cancellation/deadline propagation; no scheduler or background loop
src/bin/dojo-agent.rs
  MCP boundary parsing and schema only
```

The interface hides ONNX session management, transforms, artifact verification, and output normalization. Callers provide only capture ID, bounded filters, and cancellation. The owner and runtime remain exposed as separate modules because capture authority and model execution have different owners; neither may access the other's mutable state. Imports stay private except the `PerceptionEngine` construction used by `CaptureAuthority`.

### Red-flag screen

- No second scheduler, shared writable cache, public ONNX or transport types, caller-coordinated stages, or alternate inference path.
- The model bundle has one owner and immutable artifact identity. Capture storage remains owned by Dojo's existing `CaptureService`.
- The risk is a shallow wrapper if cancellation is only a Tokio timeout around blocking ONNX. The required terminate-and-join contract prevents that shape from being accepted as complete.
- Result normalization is one host boundary; worker output cannot define its own trusted identity or action authority.

## Candidate status

This is the embedded candidate only; it is not a synthesis decision or an implementation approval. It is structurally distinct from the process-worker candidate: ONNX sessions live in the `dojo-agent` address space, with no subprocess protocol or process containment boundary.

## Tradeoffs accepted

- We accept sharing the desktop service's address space with native inference code in exchange for no worker launch, IPC framing, or child lifecycle.
- We accept loading the pinned model closure at service startup in exchange for stable in-process sessions and no uncontrolled runtime acquisition.
- We accept an initial serialized inference queue in exchange for a simple memory bound and predictable cancellation ownership.
- We accept refusing the capability until ONNX cancellation and Nix closure identity are proven in exchange for never returning after a fake deadline while inference continues unseen.
- We accept that OCR text and generic icon classes are not semantic meaning; regions remain noninteractive.

## Alternatives considered

- **Contained process worker (stronger current candidate):** keeps inference crashes, allocator leaks, and ONNX code outside the service and can be killed/reaped at a deadline. It exposes process configuration and artifact staging to the host adapter, while hiding tensor/runtime complexity. It currently has a concrete upstream IPC and Linux-containment implementation, but has an unverified Nix loader allowlist, a manifest hash/use TOCTOU gap, and missing production weights/runtime in the checkout. It is more viable for hard cancellation if those packaging boundaries are repaired.
- **Detached blocking task with caller timeout:** hides inference behind a small call but cannot stop CPU/memory use or guarantee capture-expiry completion. It loses because its apparent deadline is false and a timed-out parse can overlap later requests.
- **Pi/provider vision parse:** exposes provider-specific image retention, model schemas, and parsing behavior to the caller and makes output normalization less reproducible. It loses because this capability is Rust-owned local inference; Pi remains scheduler and provider owner for agent tasks, not a second inference runtime.

## Open questions and risks

- Can the selected ONNX Runtime API terminate all in-flight detector and OCR runs promptly and safely across the intended Nix build, with the run options' lifetime valid during cancellation?
- Can the exact reviewed OmniParser conversion and OCR artifacts be sourced and pinned in a reproducible Nix closure, with licensing and redistribution terms recorded? The current report found the detector bytes absent and no download URL.
- What measured peak memory and latency should set the one-parse memory/queue budget on supported hardware?
- Should a parse that completes exactly at capture expiry return `Expired`? This design says yes, using a final monotonic-time check.

## Next implementation step

Build a no-product feasibility spike for the pinned ONNX Runtime that runs a deliberately long inference, terminates it through the chosen cancellation API, joins it within a measured deadline, and verifies the Nix-linked runtime closure before implementing the production engine.
