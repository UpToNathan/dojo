# Candidate B: retain the CUA core behind a Dojo episode owner

Design only. No repository or installed files changed. Grounded in `/tmp/dojo-cua-grounding.md`, its native/semantic reports, actual pinned CUA source, current Dojo service/capture code, and adoption tickets 04, 06, 07, 16. Source pin is `88aff7e5424e9ed45b93ded85eb4111472ca5190`. The code below is a type sketch, not an implementation or a validated extraction build.

## Usage, written first

The socket and MCP adapters acquire a session handle from the existing shared Rust service. A caller never supplies its own lifecycle/session identity. Both adapters parse a generated Dojo contract, and they call the same session object. The Pi host persists intent/result/recovery and calls one operation at a time.

```rust
use dojo_computer::{Computer, Observe, Execute, ObservationSource, Operation};

// Server-side only. owner is authenticated connection evidence, not tool JSON.
let computer = Computer::open(host_configuration)?;
let mut session = computer.attach(connection_owner)?;

// Delivered PNG coordinates remain the public coordinate contract.
let screen = session.observe(Observe::screen("DP-1", Some(window_id))).await?;
let receipt = session.execute(Execute::against(&screen.id,
    Operation::Click { point: ImagePoint::new(240.0, 75.0), button: Primary }
)).await?;
// receipt can say handled, paused, refused, partial, unknown. It does not assert
// the user's task succeeded. Pi must observe again after an admitted effect.

// A single session owns native, accessible-object, and browser reference state.
let tree = session.observe(Observe::accessible(window_id)).await?;
let page = session.observe(Observe::browser(window_id)).await?;
let receipt = session.execute(Execute::against(&page.id,
    Operation::BrowserFill { element: page.element("search")?, text: "dojo".into() }
)).await?;
// Initially this returns CapabilityRefused(ExternalMutationAdmissionUnproved).
// It never falls through to a coordinate click or uncontrolled primary seat.

// Browser preparation is explicit and unsafe to replay. It is also capability
// gated until its lifecycle and effects satisfy the relevant admission proof.
let prepared = session.execute(Execute::explicit(
    Operation::PrepareBrowser { profile: IsolatedNew, authorization: launch_grant }
)).await?;
session.close().await?;
```

`page.element(...)` above is convenience over the delivered element list, never a network query, ordinal rewalk, or authority grant. A caller can also use the opaque element reference returned in the observation. The caller sees route semantics such as accessible set-value versus physical typing. It does not see CUA public session strings, ToolRegistry, CDP connections, SnapshotStore, AT-SPI ObjectRef, approval broker, or Wayland resources.

## Problem

Dojo already owns stronger capture/session/process identity and guarded native delivery than CUA desktop snapshots. CUA has a much richer browser engine, coherent semantic stores, policy/consent contracts, and reusable Rust infrastructure. Rewriting every dependency of that engine would discard useful behavior and create a broad port. Importing it unchanged would create another lifecycle authority and permit effects through CDP/D-Bus without Niri dispatch checks. This candidate retains the complete core source dependency and its coherent contracts but confines resource ownership and tool dispatch to a private Dojo embedded episode. Pi remains the sole scheduler, task persistence, transcript, model transport, and retry decision owner.

## Shape

### Data structures and signatures

```rust
// Public Dojo crate. Public IDs are opaque values, never caller chosen owners.
pub struct Computer { state: Arc<ServiceState> }
pub struct Session { episode: Episode }
pub struct Observe { source: ObservationSource, options: ObservationOptions }
pub enum ObservationSource {
    Screen { output: OutputName, window: Option<WindowId>, region: Option<Rect> },
    Accessible { window: WindowId },
    Browser { window: WindowId },
}
pub struct Observation {
    pub id: ObservationId,
    pub source: SourceDescription,
    pub image: Option<Image>,
    pub elements: Vec<Element>,
    pub coverage: Coverage,
    pub actionability: Actionability,
}
pub struct Execute { observation: Option<ObservationId>, intent: IntentId,
                     operation: Operation }
pub enum Operation {
    Click { point: ImagePoint, button: Button },
    Scroll { point: ImagePoint, delta: ScrollDelta },
    PhysicalType { text: SecretText },
    AccessibleAction { element: ElementRef, action: AccessibleActionName },
    AccessibleSetValue { element: ElementRef, value: SecretText },
    BrowserFill { element: ElementRef, text: SecretText },
    BrowserNavigate { target: BrowserTargetRef, url: Url },
    // Contract also includes drag/key/dialog/file/download/window families;
    // unsupported families produce typed capability refusal.
    PrepareBrowser { profile: ProfileSelection, authorization: GrantRef },
}
pub enum Effect { None, Handled, Partial, Unknown }
pub struct Receipt { intent: IntentId, route: Route, effect: Effect,
                     outcome: Outcome, fresh_observation_required: bool }

impl Computer {
    pub fn open(configuration: HostConfiguration) -> Result<Self> { todo!() }
    pub(crate) fn attach(&self, owner: ConnectionOwner) -> Result<Session> { todo!() }
}
impl Session {
    pub async fn observe(&mut self, request: Observe) -> Result<Observation> { todo!() }
    pub async fn execute(&mut self, request: Execute) -> Result<Receipt> { todo!() }
    pub async fn verify(&mut self, request: Verify) -> Result<Verification> { todo!() }
    pub async fn close(&mut self) -> Result<CleanupReport> { todo!() }
}

// Private ownership, typed separately from a display label or public ref.
struct EpisodeKey { service_epoch: ServiceEpoch, generation: NonZeroU64 }
struct Episode {
    key: EpisodeKey,
    alive: Arc<EpisodeLiveness>,
    cancellation: CancellationToken,
    observations: ObservationStore,
    native: NativeSeat,
    imported: CuaEpisode,
    admission: AdmissionGate,
}
struct CuaEpisode {
    // The only owner of this episode's CUA engine and stores.
    embedded: cua_private::EmbeddedEpisode,
}
struct ObservationRecord {
    owner: EpisodeKey,
    target: Option<ExactWindowIdentity>,
    evidence: ObservationEvidence,
    payload: RetainedPayload,
    validity: Validity,
}
enum RetainedPayload {
    Pixels(CaptureRecord),
    Accessible { snapshot: NativeSnapshotKey, retained: AtspiPayload },
    Browser { snapshot: BrowserSnapshotKey },
}
struct SceneStamp { compositor_epoch: CompositorEpoch, capture_sequence: u64 }
struct ExactWindowIdentity {
    compositor_epoch: CompositorEpoch, niri_id: WindowId,
    mapped_generation: u64, pid: Pid, process_start: ProcessStart,
    client_generation: ClientGeneration,
}
struct CapturedScene {
    stamp: SceneStamp,
    output: OutputFrame,
    pixels: DeliveredPixels,
    windows: Vec<RenderedWindowEvidence>,
}
struct RenderedWindowEvidence {
    identity: ExactWindowIdentity, workspace: WorkspaceId,
    bounds: Rect, clip: Region, visibility: Visibility,
    coverage: SurfaceCoverage, admission_geometry: Actionability,
}
struct ExternalPermit { /* private, non-Clone; constructed only by proved gate */ }
impl AdmissionGate {
    async fn external(&self, target: &ExactWindowIdentity, intent: IntentId,
                      route: ExternalRoute) -> Result<ExternalPermit> { todo!() }
}
```

These are selected signatures. Shape exploration must settle the full contract inventory before implementation; the omitted operation families are not a justification to omit useful CUA behavior. There is no public `raw_cdp`, `invoke_cua_tool`, `set_session_id`, `resolve_store`, or protocol-result escape hatch.

### Ownership and compatibility adapter

`Computer` is the service's session registry and authority. `Session` owns all retained observations, its input seat, browser preparation/grants, cleanup, and one private imported episode. CUA browser and snapshot stores remain different representations. They are backend storage owned by this episode; `ObservationStore` holds validity and backing keys, not a second copy of browser/semantic payloads. If a reference fails at either layer, execution refuses.

Keep the complete CUA core and contract as a private dependency reachable only from `dojo-computer::cua_private`. A dedicated embedded API in the vendor core constructs exactly the allowed tools/engine under an opaque Dojo context. It preserves existing policy, consent, ToolResult, typed contract, browser state and refusal behavior internally. Its JSON adapter adds policy conversion, session binding, budget/cancellation and receipt normalization, so it is more than a forwarding wrapper.

CUA lifecycle functions in embedded mode delegate to `EpisodeLiveness`, owned by Dojo. They cannot revive a closed episode, elect an idle timeout, infer ownership from a public label, or create a new session implicitly. CUA cleanup hooks are episode-local callbacks executed by Dojo close. Broker and protected-resource ownership state belong to this episode. No separate daemon/runtime registry, core reaper, recording scheduler, batch orchestrator, script engine, or replay loop is instantiated. Browser mutation/reconnect gates and CDP event demultiplexing remain internal transport coordination, with no next-action policy or task persistence.

Current CUA globals have explicit integration hazards. `snapshot_store::runtime_scope_key()` falls back to `legacy`; `tool::current_dispatch_runtime_scope()` also allows construction scope; `session` supports begin/revive/touch/idle eviction and public labels. Retaining these defaults behind a wrapper is rejected. Embedded dispatch must demand the explicit episode context. Async children receive it explicitly or through a task-local scope installed for their whole future. A worker cannot inherit authority from whichever task currently happens to run. Publication requires live episode validation under the same store mutation guard as teardown.

Use one preferred Dojo catalog derived from typed contracts. Keep useful CUA fields, constraints, refusal details, capability metadata, and explicit route semantics. Translate session fields out, extend targets with named outputs/scene identity, and replace weak bare PID/window authority with opaque observed refs. Legacy MCP tools translate into this same API and are retired at cutover. They do not instantiate another path to screenshots or actions. Per boundary-discipline and minimize-reader-load, wire types stay private and pure domain results are the public interface.

### Native capture and authoritative scene geometry

Retain and fix CUA's standard `ext_screencopy.rs` client. First prove the Wayland connection and selected Niri IPC socket address the same compositor, using peer credentials plus compositor epoch/process-start identity. `--socket` can select nested Niri while `WAYLAND_DISPLAY` still points to the host; matching output names is insufficient. Reject a mismatch before capture publication. Match `wl_output.name` to the requested output, handle removal/session stop, use poll/read/dispatch with a single deadline and cancellation FD, own FD/mmap/pool/buffer with RAII, validate allocation/stride/format limits, and honor frame transform. Decode into displayed orientation before crop/resampling. Delivered dimensions define independent x/y mapping. Grim is an explicitly selected migration backend and cannot silently rescue a stale/native denied capture.

Add a Niri scene metadata extension associated with the same standard capture session/frame, rather than infer geometry from three IPC reads. A possible protocol request creates a one-shot scene observer on an `ext_image_copy_capture_session_v1` belonging to the same Wayland client, before its single frame request. Niri attaches `SceneStamp` and immutable render evidence when it constructs the actual `ScreenCapture` render elements; the ready response identifies that stamp. Pixels and metadata refer to that single render. If capture fails, output changes, observer association is unavailable, or metadata lacks a complete matching stamp, publication refuses. A session association prototype must first prove Smithay exposes sufficient identity to link its pending frame and the extension observer; this design does not claim that API already exists.

Niri shares final render-placement helpers with scene evidence. Stable tiled columns get the actual scroll/column/tile/render offsets and physical rounding. Floating windows use render offsets. Surface/client identity and clips come from compositor scene construction. Never reproduce column layout in the Rust agent. Stable `WindowLayout` remains the low-frequency IPC layout description. An initial truthful scene backend can mark overview, workspace transitions, interactive moves, opening/resize animations or unknown transforms unavailable for window-scoped actions while output observations remain useful. Animated parity later needs final draw transforms and clips, including bob/opening/monitor passes, rather than a second approximate algorithm.

ScreenCapture output can differ from physical framebuffer policy. The contract reports visible-composited capture policy, cursor inclusion, decorations/popup clipping and occlusion coverage honestly. A window crop is not an isolated unobscured application screenshot. Niri still rechecks current hit-tested client/target/coordination at native effect dispatch.

### AT-SPI

Retain native tree traversal, queries, budgets, object retention and action semantics. Replace platform discovery with a Niri-backed `AccessibleWindowEvidence` adapter. It proves exact process/bus owner, retained frame object and native-window correlation. An application-scoped or ambiguous tree can be returned with honest coverage and non-actionable refs. Multiple windows in one PID cannot be disambiguated by title or first-window selection.

Patch `SnapshotStore` identity to include EpisodeKey and exact process/window generation for publication and resolution, with unforgeable internal ownership context. Remove bare row action aliases, process-global restart-sensitive tokens as authority, and `SideIndex` PID-without-window fallback. Public Dojo refs resolve to the retained native payload directly; no ordinal rewalk. Preserve original indices after query projection. Add real lifecycle cancellation to D-Bus reads and mutation calls, and reject late publication. Negative existence verification requires trusted complete scoped coverage.

Direct D-Bus mutations, including helpers called by browser setup/consent, require ExternalPermit in the innermost effect executor. Merely leaving MCP action names unregistered is insufficient. No X11 click/type fallback survives the Niri port.

### Browser

Retain the entire browser directory, including engine, store, CDP pool/transport, semantic/frame/OOPIF logic, binding, preparation, downloads, grants, reconnect and mutation gates. Use Dojo `BrowserPlatform` for exact native/client/process identity, render geometry/cardinality, process tree, trusted executable and loopback socket ownership. Explicit trusted Nix store wrapper/symlink handling replaces upstream's blanket canonical-path redirect refusal. Arbitrary writable redirect chains remain refused.

Retain CUA consent/origin/file authorization through the episode context; consent is distinct from current human priority. Do not require callers to preconstruct an engine, provide grants, attach a tab, and manage cleanup across public calls. First support read-only observation of proved endpoints. Explicit driver-owned preparation and later existing-profile attachment have their own authorization/effect capabilities; preparation never runs during observation. Existing-profile foreground effects remain refused when they violate independent-seat/human-focus guarantees.

Audit every raw CDP send in engine/tools/preparation/download/consent paths. The private transport API accepts classified read requests or mutation requests carrying ExternalPermit. Unknown commands refuse. Browser observation domain enable/subscription/attachment commands need explicit audited observation classifications; arbitrary Runtime.evaluate is never classified read-only merely because its caller labels it observation. Browser ref frame/document and endpoint generation checks remain intact. The imported mutation lock serializes agents but grants no human coordination authority.

### Capability gates

At first `NativeObserve`, `AccessibleObserve`, and `BrowserObserve` can be advertised when their own identity/coverage proof passes. Native effects retain Dojo's compositor checks. `AccessibleMutation`, `BrowserMutation`, browser setup effects, and every related helper return `ExternalMutationAdmissionUnproved` until ticket 16 succeeds for their route. A config switch cannot construct ExternalPermit.

The protocol proof must specify the admission linearization point relative to immediate human input, queue revocation, exact client/window/process generation, and honest already-admitted in-flight/unknown outcomes. A revision query followed by CDP/D-Bus dispatch fails this gate. Human events never wait on a D-Bus/CDP call. After an admitted or ambiguous effect, consume all related observation refs and require fresh observation; never restore consumed refs or replay through another route.

## Exact source import closure and patches

The broad private dependency is intentional. This is a source-level closure, not a build claim.

1. Preserve the whole `libs/cua-driver/rust/crates/cua-driver-core/src/**` and `cua-driver-core/Cargo.toml`, including its inline tests, `browser/mock_cdp.rs`, `browser/v2_tests.rs` and script prelude fixture. Preserve the whole `libs/cua-driver/rust/crates/cua-driver-contract/src/**` and its Cargo manifest, contract fixtures/generator resources referenced by its tests. Retain MIT notices and a manifest of source hashes. A source manifest includes every file under these two crate roots, so no hand-maintained helper dependency list masquerades as a closure.
2. Preserve `libs/cua-driver/rust/Skills/cua-driver/{BROWSER,EMBEDDING,LINUX,MACOS,README,RECORDING,RUNTIME,SKILL,VISUAL,WORKFLOW,WINDOWS}.md` at the relative locations used by `mcp_skills.rs` include_str. They are build/test dependencies, not automatically advertised Dojo skills. Resolve other fixture path references by a manifest scan and pin them; generator binaries and non-embedded runtime modules are not shipped entrypoints.
3. Extract precisely `platform-linux/src/atspi/{mod.rs,native.rs,snapshot.rs,native/hit.rs,native/hit_test.rs}`, `platform-linux/src/snapshot_queries.rs`, `platform-linux/src/at_point_policy.rs`, `platform-linux/src/pointer_shape_map.rs`, and `platform-linux/src/wayland/ext_screencopy.rs`, retaining their inline fixtures/tests. For AT-SPI hit support, retain role sampling only if used, using imported core cursor_shape. Delete unneeded cursor-emulation consumers rather than import global input/overlay ownership.
4. Replace source references to `crate::{x11,input,wayland::{hyprland,sway_ipc,shell_helper},browser_platform}` in the extracted AT-SPI implementation with the narrow Niri correlation/geometry adapter or explicit refusal. Do not import these whole platform modules. Preserve useful pure geometry/trust logic and retain/translate its tests. Rewrite X11 DISPLAY-specific tests to adapter fixtures. These are documented integration patches, not allegedly unchanged imports.
5. Adapt manifest workspace inheritance to a bounded local vendor workspace or explicit fields. Core dependency set is its full pinned manifest: contract, tokio, serde/serde_json, anyhow, thiserror, tracing, async-trait, serde_yaml_ng, image and pinned compatible zune-core, tokio-tungstenite 0.24, futures-util, uuid, base64, sha2, url, chacha20poly1305, hkdf, zeroize, coset, ciborium, time, getrandom, fs2, hmac, jsonschema 0.46, tempfile and Unix libc. Disable default yaml/rego/script unless the selected Dojo policy needs them; serde_yaml_ng is unconditional upstream. Contract adds schemars 1.2.1 and uniffi. Native extraction adds atspi and its D-Bus dependencies, Wayland client/protocol bindings, SHM/mmap support and PNG/image helpers matching the source manifests. Cargo.lock gives the final exact transitive dependency closure.
6. Add vendor `EmbeddedEpisode` and context injection. Patch `tool.rs`, `session.rs`, `snapshot_store.rs`, `element_token.rs`, authorization/consent construction and browser lifecycle hooks as described above. Add strict embedded registration, exclude legacy page/script/batch/replay/daemon/session tools, disable implicit revive/touch/reaper paths, and make lifecycle globals non-authoritative in embedded mode. Retain tests for non-embedded upstream behavior where it remains compiled, add embedded cross-session/teardown tests, and document each changed assertion.
7. Patch CDP sends and AT-SPI effect calls to require the shared typed admission capability and explicit request classification. Patch cancellation/late result suppression across all retained async work. These changes are mandatory before any effect capability is advertised, including preparation.
8. Preserve all remaining broad core source as private build dependency initially. Used modules should be listed separately in the provenance ledger. Dormant source does not constitute adopted runtime parity. If the dependency/patch cost outweighs reuse, remove dormant modules in a later source extraction wave supported by actual Cargo compilation and dependency scan, not a guessed list.

Only `dojo-computer` depends on the private vendor core. Contract conversion is internal. A dependency check fails if another workspace crate directly imports the vendor core, and API checks fail if Dojo publicly reexports its types. License/private-source choice does not make crate visibility a security boundary; the effect capability and embedded constructor checks carry the safety guarantee.

## Module map

```text
src/bin/dojo-agent/{mcp,socket,service}
  parse contract, authenticate connection, own service startup/connection loss
  calls Computer::attach and Session::{observe,execute,verify,close}
crates/dojo-computer-contract
  typed Dojo contract, schema/capability/route/result metadata
crates/dojo-computer
  lib.rs: small public Computer/Session API
  session.rs: sole episode ownership, lifecycle, observation validity, cancellation
  cua_private.rs: private embedded import/context/result conversion
  niri.rs: exact identity/scene/native input adapter
  accessibility.rs: retained native payload and Niri correlation adapter
  capture.rs: delivered image mapping/limits and selected backend
  admission.rs: native checks and initially-refusing external gate
vendor/cua/.../cua-driver-core
  coherent imported implementations plus embedded integration patches
vendor/cua/.../cua-driver-contract
  upstream format/schema behavior behind private conversion
vendor/niri
  final render evidence, correlated scene extension, guarded native dispatch
Pi desktop extension
  generated preferred catalog, unsafe replay classification, durable recovery
```

Module depth is high. Four session operations hide output capture, all retained ref namespaces, browser bindings/grants/preparation, input lifecycle, capability/policy checks, cancellation, cleanup and partial/unknown semantics. Callers select what they want to observe or do. They never coordinate separate resource managers. Per separate-before-serializing-shared-state, sessions own distinct browser/store instances; immutable backend connections can be shared only when their identity/cancellation semantics permit it.

## Synthesis decision

Pending parent comparison with the other structural candidate. Candidate B favors keeping coherent CUA core and contract context, with a private embedded lifecycle patch, over rebuilding each helper dependency as a local thin port. Its viability depends on proving the embedded constructor and innermost effect gate prevent every unscoped/reviving/mutating bypass. If that proof demands pervasive ambient-global exceptions, reject B rather than preserve those exceptions.

## Tradeoffs accepted

- We accept a broader initial Cargo dependency and dormant source set in exchange for retaining coherent engine/policy/consent/contract behavior and upstream tests.
- We accept explicit integration patches to CUA lifecycle and stores in exchange for one authoritative Dojo episode and strict shutdown invalidation.
- We accept a small custom Niri observation protocol in exchange for capture-correlated render geometry; standard capture alone lacks that contract.
- We accept truthful read-only semantic/browser capabilities before ticket 16 in exchange for protecting human priority without speculative timing guarantees.
- We accept distinct private backend stores in exchange for preserving their tested identity semantics. A single Dojo owner unifies validity and lifecycle.

## Alternatives considered

A selective module import with locally rebuilt policy/lifecycle context has a smaller dependency set and can enforce explicit ownership cleanly. Its public API can be equally deep. It risks replacing substantial CUA consent/contract machinery and losing upstream assumptions as each helper gets translated; the synthesis should favor it if broad embedded patching proves harder to audit.

A stock CUA daemon behind Dojo exposes shallow integration and creates a second session/resource authority. Passing a session label to its tools cannot bind it to the Dojo connection or protect semantic dispatch. It is rejected.

Separate capture/accessibility/browser public managers each hide their own protocol, but require callers to coordinate lifecycle, observation invalidation, permission and ref consumption. They duplicate identity rules and create several viable mutation paths. They are rejected for split ownership and caller load.

## Red flag screen

- Shallow module: four operations hide substantial implementation and lifecycle policy. No public staged binding/preparation pipeline is required for ordinary operations.
- Information leakage/importable internals: no CUA transport, tool registry or store types cross the public API; dependency/API checks enforce it.
- Temporal decomposition: modules own identity, capture mapping, retained accessibility and authority decisions, rather than exposing load/validate/send stages.
- Pass-through: the embedded adapter removes public ownership claims, injects server evidence, binds cancellation, normalizes results and forbids unsupported effects.
- Split ownership: Dojo owns episode liveness/observation validity; CUA backend stores own their retained payloads under that episode. No CUA timer can revive or retire the session independently.
- Two ways: one preferred generated tool catalog and common typed session route; legacy adapters are temporary translations and removed at parity.
- Hand-synced list: schema generates catalog/replay/route metadata; a classified effect command inventory drives transport admission. Unknown commands refuse rather than silently bypass policy.

## Open questions and risks

Can a small embedded patch remove every ambient lifecycle/legacy-scope fallback without changing most core modules? A compile/import prototype and direct bypass probes resolve this; broad source reuse is conditional on that result.

Can the current Smithay capture session/frame wrappers associate a same-client Niri scene observer with the exact render? Prototype the extension before choosing its wire object. If unsupported, design a capture-plus-scene operation with equivalent single-render evidence rather than correlate separate IPC snapshots by approximate time.

Which multi-window applications expose sufficient AT-SPI identity to prove the frame/window mapping? Unsupported cases remain application-scoped observations until fixture/live evidence proves correlation.

Which CDP/D-Bus effect routes satisfy ticket 16's admission ordering and in-flight semantics? Each capability stays refused until its route has evidence.

## Verifiable implementation units

1. Import/license ledger and standalone broad embedded dependency build. Retain upstream tests, generate source/dependency manifest and schema fixtures, and prove no deployed Python or runtime daemon dependency. Run from devenv; no install.
2. Add embedded episode context, strict allowed registration and episode-local cleanup. Prove identical public labels on separate connections cannot share browser/snapshot resources; stop during publication cannot publish; stale service-epoch refs fail. Probe direct imported lifecycle/dispatch APIs for revive and unscoped fallback.
3. Fix native client in isolation using deterministic protocol fixtures for host-versus-nested compositor peer identity, named output, removal, hung dispatch, every supported format/transform, budgets, RAII and cancellation. Compare actual returned dimensions/mapping with grim in a nested desktop.
4. Prototype and implement scene correlation with tiled/floating render evidence. Prove every accepted window capture has the same stamp as pixels; mismatches/transitions/epoch restart fail closed. Exercise scales/rotation/clip/popup/occlusion without human focus/pointer effects.
5. Import AT-SPI retained payloads and queries with the Niri adapter; expose read-only observations. Test duplicate names, projected indices, stale/cross-session/process reuse, multi-window ambiguity, D-Bus hangs and teardown.
6. Import the entire browser engine through embedded context and BrowserPlatform. Exercise retained mock CDP tests plus real isolated fixture pages, frame/OOPIF changes, endpoint/process identity, grants and cleanup. Keep preparation and mutation refused until their admission capability passes.
7. Prototype ticket 16 using controllable fixtures paused at every check/admission/send/effect/result boundary, then nested human takeover. Only successful route proofs permit construction of ExternalPermit. Test unknown outcomes and queued follow-on cancellation for every effect family.
8. Cut preferred MCP/direct/Pi callers over together, preserve image/receipt/recovery metadata, then delete legacy screenshot/ref/effect paths and temporary grim after parity. Crash/stop/restart tests prove unsafe replay stays refused and human input remains immediate.

These are proposed verification units, not tests already run. The first implementation step is the broad import/embedded episode prototype, because its ownership proof decides whether this candidate deserves to be the base.
