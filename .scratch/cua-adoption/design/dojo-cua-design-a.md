# Candidate A — one session-bound computer capability

## Problem

Dojo already has the right authority spine: `dojo-agent` owns connection sessions, `dojo-computer` owns capture identity/admission, Niri checks native dispatch, and Pi owns the durable run. CUA contributes substantially richer native capture, scene/semantic observations, and a coherent browser engine. The design must import those behaviors without creating a second scheduler, split session owner, or a CDP/D-Bus route around human-priority checks. Keep external protocol and CUA types private behind one Rust resource API.

## Usage (caller's view)

The MCP adapter and any direct Rust caller get the same context-bound resource. They do not coordinate capture, token stores, browser references, or cleanup themselves.

```rust
let computer = session.computer(); // borrowed capability, no caller-supplied session ID
let observation = computer.observe(ObserveRequest::Window { window, include: Include::ImageAndSemantics }).await?;
let result = computer.execute(ActionRequest::Click { observation: observation.id, point: ImagePoint::new(410, 228) }).await?;
// result is HandledUnverified, Paused, Refused(capability/reason), or Unknown; observe again before another dependent action.
```

```rust
let page = computer.observe(ObserveRequest::Browser { window, detail: BrowserDetail::Accessibility }).await?;
let result = computer.execute(ActionRequest::Browser(BrowserAction::Click { observation: page.id, element: page.element("e...") })).await?;
// During initial rollout this returns Refused(MutationAdmissionUnavailable); no CDP effect is sent.
```

```rust
let tree = computer.observe(ObserveRequest::Accessibility { window: Some(window), query, limits }).await?;
// `tree.scope` and `tree.completeness` are explicit; an uncorrelated AT-SPI tree is useful read-only evidence, never an actionable window token.
```

## Shape

### Public types and operations

```rust
pub struct ComputerSession { /* owned inside dojo-agent Session */ }
pub struct Computer<'s> { session: &'s ComputerSession }
pub struct ObservationId(PrivateBytes);
pub struct ElementRef(PrivateBytes);
pub enum Capability { NativeCapture, RenderedWindowGeometry, AtspiRead, BrowserRead, NativePointer, NativeText, AtspiMutation, BrowserMutation }
pub struct Capabilities { supported: Set<Capability>, refused: Map<Capability, Refusal> }
pub enum Refusal { Unsupported, EvidenceUnavailable, AdmissionUnproved, PolicyDenied }
pub enum Outcome<T> { Completed(T), Paused, Refused(Refusal), Unknown(EffectReceipt) }

impl Computer<'_> {
  async fn capabilities(&self) -> Capabilities;
  async fn observe(&self, request: ObserveRequest) -> Result<Observation, ObserveError>;
  async fn execute(&self, action: ActionRequest) -> Result<Outcome<ActionReceipt>, ExecuteError>;
  async fn verify(&self, observation: ObservationId, expectation: Expectation) -> Result<Verification, VerifyError>;
}
```

The service-issued observation ID is opaque and scoped to server session + runtime epoch. It resolves to one immutable observation record containing capture digest/dimensions/mapping, exact Niri `WindowIdentity`, output/scene generation and optional semantic/browser snapshot references. It is a lookup key, not authority by itself. An action consumes the observation at admission and retires sibling observations before effects, matching current capture semantics. `Observation` reports coverage, scope, completeness/truncation, and the actual backend/transform evidence; callers cannot assert those fields.

`ComputerSession` lives in existing `service.rs::Session`; teardown cancels work, releases the native seat, invalidates all observations and asks owned backends to clean up. The shared agent process remains sole owner of `CaptureAuthority` and connection-keyed sessions. Pi remains the only scheduler and durable intent/result/recovery owner. Every action result distinguishes no-effect refusal, native handled-but-unverified, paused, and unknown/partial external effect. There is no automatic retry.

### Private composition and module map

- `dojo-computer` remains the deep module: `session.rs` owns observation records, generations, admission and the single public operation surface; `capture/` owns `CapturePlan`, image mapping and backend selection; `semantic/` owns AT-SPI snapshots/ref resolution; `browser/` owns the whole CUA BrowserEngine and its stores; `admission/` owns future common side-channel admission. No public CUA store/engine types.
- `dojo-agent/service.rs` is the resource/session adapter. It creates `ComputerSession`, passes server-side connection identity, cancellation/deadline and authorization context, and exposes typed operations through MCP. No CUA loop or scheduler is imported.
- `dojo-computer-platform-linux` (or private submodules while crate layout settles) contains Wayland screencopy, AT-SPI D-Bus, and `BrowserPlatform` implementations. Platform adapters return evidence; they do not decide whether it authorizes an action.
- `vendor/niri` adds an on-demand `RenderedScene`/capture-correlated observation path using the render scene helpers. It returns exact window/process identity, rendered bounds, clip/visibility, workspace/output, scene generation and explicit unavailable reason. Stable `WindowLayout` is not promoted as rendered geometry.
- `tools/speech/desktop-extension.mjs` and `pi-runtime.mjs` only register schemas, mark every effect unsafe to replay, sequence actions, persist recovery-required state, preserve images, and close/cancel the resource. They never own observation stores.

Core internal signatures:

```rust
struct RequestContext { session: SessionKey, epoch: RuntimeEpoch, deadline: Deadline, cancel: CancellationToken, grant: Option<GrantId> }
struct RenderedFrame { capture: CapturedPixels, scene: Option<RenderedScene>, frame_key: FrameKey }
trait CaptureBackend { async fn capture(&self, plan: &CapturePlan, ctx: &RequestContext) -> Result<CapturedPixels, CaptureFailure>; }
trait SceneSource { async fn capture_scene(&self, output: OutputName, ctx: &RequestContext) -> Result<RenderedScene, SceneFailure>; }
trait SemanticSource { async fn observe(&self, target: Option<&WindowIdentity>, query: Query, ctx: &RequestContext) -> Result<SemanticSnapshot, SemanticFailure>; }
trait BrowserPlatform { /* exact process/endpoint/window evidence only; CUA engine remains private */ }
```

A native `observe` resolves named output/target, obtains a capture-correlated scene and pixels, normalizes explicit format/stride/transform, applies bounded crop/resampling, and publishes only if frame/identity evidence remains valid. Initial geometry capability supports only compositor states with proven scene/capture correlation; animations, overview, transition, popup/clip cases lacking evidence return `EvidenceUnavailable`. Grim may remain a backend selected only by explicit migration config and must emit the same contract.

AT-SPI observation imports the bounded traversal and retained object references, but Dojo supplies exact window/process-generation correlation. If correlation fails, return a tree marked unscoped/non-actionable (or a process-scoped read-only result); never resolve action tokens through PID-first or bare-row fallback. Token lookup includes session, epoch, exact process start time/window, observation and retained object identity. Teardown prevents late publication.

Browser reuse imports the complete engine as a private unit: CDP transport, binding, semantic snapshots, refs, frames/OOPIFs, mutation/reconnect gates, grants, preparation, downloads and cleanup. `BrowserPlatform` proves process tree, endpoint ownership, tab target, URL and native-window cardinality. Begin with Dojo-owned isolated Chromium. Existing profiles and their grants stay refused until consent and foreground rules are adapted. Browser/AT-SPI reads are exposed after lifecycle and target proof; *all* external mutations (including setup, navigation, dialogs, uploads/downloads and set-value) return `AdmissionUnproved` until ticket 16 defines and race-proves an enforceable admission point. Approval/grant does not imply priority. Do not hold human input behind a side-channel lock.

### Reuse ledger at this boundary

- Import unchanged where dependency/build verification confirms: extracted MIT `browser/cdp_ws.rs`, `image_utils.rs`, `element_frame.rs`, `window_inspection.rs`; pure `walk_budget.rs` and `element_query.rs` behavior/fixtures; complete `core/browser/` as a coherent initial source snapshot, with a recorded patch ledger rather than a fragmented rewrite. Preserve upstream tests and notices.
- Import with narrow patches: `platform-linux/wayland/ext_screencopy.rs` (named output, cancellable bounded dispatch, explicit formats/stride/transform, RAII); AT-SPI native traversal/hit logic (remove X11/Hyprland/title discovery and effect entrypoints from read interface; add Dojo identity/context); `snapshot_store.rs`/`element_token.rs` (remove globals/row fallback, inject session+epoch+exact identity); browser engine integrations (replace CUA session, dispatch, policy and platform hooks with explicit `RequestContext`/Dojo adapters; preserve grant semantics where compatible).
- Keep CUA `protocol::ToolResult`, global dispatch runtime, CUA session globals, Python loops, agent scheduler, policy authority, and service lifecycle out. Translate results to Dojo contract. Do not silently call a module “unchanged” after patching it.

This is a deep interface: callers ask for an observation, execute against its ID, and receive explicit effect state. It hides target binding, budgets, stores, stale-reference checks, platform protocols and cleanup. It exposes only capability/refusal and evidence completeness needed for honest decisions. It avoids a public stage-by-stage capture/snapshot/browser API (information leakage and temporal decomposition).

## Tradeoffs accepted

- We accept staged `AdmissionUnproved` refusals for semantic/browser mutations in exchange for preserving human priority until the external dispatch race is solved.
- We accept partial semantic usefulness without an actionable window binding in exchange for honest read-only evidence; tokens are never upgraded by title or PID ambiguity.
- We accept explicit unavailability during unsupported Niri render transitions in exchange for geometry tied to pixels rather than stable-layout guesses.
- We accept an integrated private browser dependency closure and patch ledger in exchange for retaining the tested coherent engine rather than rebuilding a shallow local CDP facade.
- We accept an explicitly selected grim migration backend temporarily in exchange for parity comparison without two advertised interpretations or implicit fallbacks.

## Alternatives considered

1. Put each CUA facility behind public `Capture`, `Atspi`, and `Browser` services. Rejected: callers would sequence stores, lifecycle, target checks and admission themselves; the interface leaks internals and creates split owners.
2. Copy only CDP websocket, AT-SPI query helpers, and native capture into a new thin implementation. Rejected: it exposes transport/targeting to callers and discards the coherent browser lifecycle and retained semantic references that make upstream richer.
3. Import the CUA runtime/dispatcher as the single computer service. Rejected: its session/policy/tool dispatch and agent-oriented assumptions conflict with Dojo session authority and Pi-only scheduling; adapting those at the boundary yields a smaller caller contract.

## Open questions and risks

- Can Niri return capture-correlated scene data in one operation, or must the compositor itself own capture and metadata correlation?
- Which Niri states can the first exact scene capability prove, and which should remain refused until animated render geometry is represented?
- Which applications provide a stable AT-SPI frame mapping to Niri identity without relying on X11 discovery?
- What exact admission point can ticket 16 enforce for CDP/D-Bus while keeping human input immediate, and which operations remain impossible to admit safely?
- Can whole-browser import and pinned dependencies build cleanly in the workspace without pulling unwanted platform policy or licensing dependencies?

## Next implementation step

Extract the caller-facing `ObservationId`/`Outcome`/capability contract and one `ComputerSession` operation surface into the existing `dojo-computer` owner, then prove session/epoch binding, consume-on-admission, teardown invalidation and explicit capability refusals before importing any external-effect backend.
