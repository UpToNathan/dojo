> Browser scope superseded: Dojo uses anomalyco/browser-control at
> `b4e4238742fa73daabfb664f1108113e9577849d`. The browser milestones below
> describe the historical CUA assessment, not active implementation requirements.
> See [replacement spec](../browser-control/spec.md).

# CUA adoption: Rust desktop execution, pi-durable orchestration

Status: implementation in progress under the user's authorization to port all useful components. Capability and rollout gates below remain required.

## Objective

Adopt CUA components that improve Dojo's assistant on this existing Niri desktop.
The [relevance register](../../docs/research/cua-dojo-relevance.md) records the
current selection, existing integrations and conditional candidates. Evaluate
completion by chosen Dojo task capabilities, including their native ownership,
verification and recovery behavior. Copied code and whole-repository parity do
not define the product's completion checklist.

Driver execution, browser semantics, native accessibility and deterministic
verification are directly relevant. Existing launcher icon caching, image request
projection, supported model-output parsers and task traces have bounded uses.
Setup, terminal parsing, perception and specialist inference require corresponding
Dojo feature needs before further adoption.

VM/QEMU/container/cloud sandbox products, Fleet, Spaces/remote infrastructure,
whole-desktop capture across monitors, and extra scheduler/provider/history or
durable approval owners are excluded. Every consumed source, dependency and
artifact retains an immutable reference. Pi owns tasks and history; pi-ai owns
provider transport and credentials; desktop execution remains Rust.

The older inventory sections below record earlier investigation. Items outside
this objective are reference material, not mandatory implementation tickets.

## Fixed commitments and change policy

1. Desktop execution, observation stores, action parsing, input translation, policy enforcement, verification and benchmark execution are Rust. No deployed Python interpreter, LiteLLM, Pydantic or Torch dependency is introduced by this effort.
2. pi-durable remains the only agent scheduler and conversation/task persistence owner. Its existing Node/JavaScript host is an intentional exception to an all-Rust application: retaining pi-durable does not mean rewriting pi-durable in Rust. Thin JavaScript registrations and provider integration remain where Pi requires them; reusable computer-use semantics move to Rust.
3. pi-ai continues to own model transport, credentials, subscription compatibility and streaming. No second Python model loop, direct-provider agent daemon or parallel conversation database.
4. Human priority and independent agent seats remain fundamental. All mutating routes—including CDP, AT-SPI, text insertion and window management—must pass Dojo authorization and coordination. A non-pointer route cannot bypass human ownership simply because it avoids Niri input events.
5. No exactly-once claim for external UI effects. Tool intent, action receipts and result storage do not remove the crash interval between effect and receipt. Never replay ambiguous effects automatically.
6. Import behavior by default unless it conflicts with these commitments, licensing, privacy or a documented correctness constraint. Record every retained divergence with upstream behavior, reason, tests and reconsideration condition. Do not quietly keep a weaker Dojo capability because it already exists.
7. Update affected local specs when behavior changes. Preserve input safety requirements, but do not treat current unsupported features or old tool names as immutable product decisions.
8. Maximize reuse of coherent implementations, tests, fixtures and formats—not copied line count. Prefer whole MIT crates or cohesive modules to fragment-by-fragment reimplementation. Remove replaced local logic after cutover.

## Current implementation: facts that shape migration

- [`src/bin/dojo-agent.rs`](../../src/bin/dojo-agent.rs) owns MCP discovery, context, screenshot, action, release, launch and feedback tools. The action enum currently contains focus, move, click, key, scroll and reset; actions use logical output coordinates and evdev codes.
- [`src/bin/dojo-agent/service.rs`](../../src/bin/dojo-agent/service.rs) owns the shared, lazy Unix socket input process and independent client sessions. It serializes actions and releases seats on disconnect/idle/error. Screenshots currently execute in the MCP adapter, outside this session owner.
- [`src/bin/dojo-agent/screenshot.rs`](../../src/bin/dojo-agent/screenshot.rs) captures through grim, supports output/region/positioned-window/scale, checks geometry before and after, and returns image-to-logical mapping. It has no capture-bound later action interface.
- [`src/agent_input.rs`](../../src/agent_input.rs) and `vendor/niri` own coordinated independent seats, arrival waits, cancellation and human takeover. The current observation spec deliberately excludes tiled-window capture until geometry is available; this roadmap explicitly schedules that compositor change.
- [`tools/speech/pi-runtime.mjs`](../../tools/speech/pi-runtime.mjs) already opens pi-durable 1.1.0, discovers desktop tools through an MCP subprocess, preserves image content, stores durable submissions and runs memory summaries. Its `stop()` aborts the Pi conversation but does not explicitly close/release the desktop session. Desktop registrations currently rely on Pi's default unsafe replay policy rather than declaring per-tool policy.
- [`docs/dojo-memory.md`](../../docs/dojo-memory.md) describes the current memory algorithm, durable database, provider identity and credentials. Older input/tool specs still describe pi-durable as future work. Reconcile those documents during migration rather than using their historical wording as the current architecture.

The working tree contains other active changes. Implementation tickets must re-read current files and their callers, isolate their diffs, and not overwrite unrelated work.

## Reuse ledger and legal gate

Ticket 01 establishes a machine-readable ledger. Every imported or translated unit records upstream revision and paths, license/notices, copied/modified/translated status, transitive dependencies, retained tests, local patches, parity evidence and owner. Hash unchanged source; describe every patch. Preserve MIT attribution for translations as well as literal copies. Pin fixtures and dependency lockfiles separately.

### Adopt directly or with limited integration edits

- Whole `libs/cua-driver/rust/crates/cua-cursor-motion` and `libs/cua/crates/cua-spaces-transcript`. Previous standalone tests passed; product integration remains work.
- Driver core `capture_registry.rs`, `image_utils.rs`, `element_frame.rs`, `window_inspection.rs`, `browser/cdp_ws.rs`. Previous standalone extraction passed 48 upstream tests. Only these tested files and the two crates are proven extractions; further entries below need dependency/build verification.
- Audit and extract further small Rust logic: `walk_budget.rs`, `text_sanitize.rs`, `key_pacing.rs`, `element_token.rs`, `action_target.rs`, relevant contract inputs/outputs and error types. Some depend on CUA result/session types; do not claim one-file independence without a build.
- `libs/cua/crates/cua-agent-setup`, `cua-icon-cache`, `cua-spaces-contract`: adapt names, paths, skill embedding and supplied tool definitions. Prefer Driver's own typed contract for desktop tools; reuse the Spaces generator only where it adds needed metadata.

### Adopt coherent Rust implementations through Dojo adapters

- Driver core `browser/`: CDP, binding, refs, semantic observation, mutations, downloads, preparation, consent/grants and lifecycle.
- `platform-linux/src/atspi/` plus core snapshot/query/token/budget machinery.
- Core verification (`expectation.rs` and contract `verification.rs`), recording/action history, bounded batch execution, policy/authorization, health reporting and consent where their behavior fits Dojo.
- `platform-linux/src/wayland/ext_screencopy.rs`: replace first-output selection and potentially unbounded roundtrips before replacing grim.
- `libs/cua/crates/cua-vmm`: excluded from this native-host scope; retained only as provenance for earlier source investigation.
- `cua-agents`: extract useful coding-agent detection/event normalization only after auditing its spacesd/protobuf dependencies; Pi remains the run owner.

### Translate Python behavior to Rust

The [source-level port inventory](python-port-inventory.md) specifies exact functions, parser families, fixture sources, known upstream defects and Pi provider constraints. It splits ticket 08 into shippable sub-milestones instead of requiring all native providers before the first Rust profile.

- `libs/python/agent/cua_agent/loops/` and `types.py`: action formats, provider conventions, coordinate normalization, grounding composition and completion/refusal handling. Port parsers and transformations, not `ComputerAgent` as a second runtime.
- `cua_agent/callbacks/`: image retention, budget controls, operator validation, trajectory serialization and privacy processing, mapped to Rust helpers and Pi hooks/tasks. Python callbacks are reference behavior, not deployed dependencies.
- `libs/cua-bench`: task/result/trace structures, bounded task lifecycle and deterministic evaluators. Build a Rust runner using Pi and Dojo adapters; inventory datasets and evaluator assumptions individually.
- `libs/cua-s1/python`: port schema/planner/driver policy (candidate construction, confidence/submit gates, output interpretation and snapshot-bound execution) in ticket 08 after semantic observations. This is useful without model weights. Ticket 14 owns Rust tensor inference after a model-format/backend equivalence spike. Training is not a requirement for shipping inference.

### Separate product and license tracks—not silently discarded

Spaces apps, spacesd/relay, streaming/media, Teleport, Keyvault and related infrastructure are FSL, not unrestricted MIT extractions. Ticket 01 records which terms permit the intended use/distribution. Any proposed reuse stays separately gated; do not vendor FSL source into the core pending that decision. Protocol/client MIT code can be reusable independently of a server's license. AGPL detector bundles and model/dataset artifacts require their own decisions.

VM, hosted, remote, macOS and Windows product implementations are outside the
current completion criteria. Their source and license records remain evidence of
earlier investigation; they do not create a requirement to deploy those products.

## Target module structure

Proposed paths, not existing modules:

- `crates/dojo-computer-contract`: typed observations/actions/results, capabilities, errors, schema generation and import provenance references. Domain types do not depend on GPUI, MCP or provider SDKs.
- `crates/dojo-computer`: deep module behind `observe`, `execute`, `verify` and session teardown operations. Owns capture/snapshot/ref stores, validation, coordination admission, routes, partial-effect reporting and bounded lifecycle. It hides CUA internals instead of exposing its globals to callers.
- `crates/dojo-computer-models`: pure Rust decoders/normalizers and grounding transforms, consuming explicit model output plus observation metadata. It does not send model requests or own credentials.
- `crates/dojo-computer-bench`: Rust task runner/evaluator/trace interchange once core observations and actions exist.
- Optional `crates/dojo-computer-inference` and `crates/dojo-computer-sandbox`: model inference and VM lifecycle with separate dependency/artifact gates.
- `vendor/cua/`: bounded imported source slices and provenance, not a build of the entire monorepo. Keep standalone unchanged crates where possible. Exact layout is settled in ticket 01 to avoid duplicate packages and nested-workspace metadata problems.
- `dojo-agent`: remains the shipped binary and shared process owner. MCP and direct socket are adapters to the same typed Rust module. Keep its existing startup/idle behavior, but broaden its session owner to include observations and refs.
- `tools/speech/pi-runtime.mjs`: named desktop extension, explicit replay policy, cancellation/recovery handling, budgets and durable model registrations. No Rust domain logic duplicated here.

Real seams have at least production and test adapters: Niri identity/input versus a deterministic fixture desktop; CDP loopback/fake server versus a real browser; AT-SPI fixture application versus real desktop apps. Inject time and cancellation for bounded operations internally. Do not add a trait for each function or expose every CUA helper as a public method.

### Observation and action invariants

An observation identifies its session, runtime generation, source/target, acquisition time, geometry generation, image digest/dimensions/transform, completeness/truncation and optional semantic snapshot. IDs are opaque references, not authorization grants. The server resolves identity from its session; callers cannot impersonate another session by providing an identifier.

CUA's capture registry is a foundation, explicitly not wired into upstream production dispatch. Its `CaptureTarget` only distinguishes a PID/window pair and primary desktop. Dojo needs a wrapper or documented patch for named outputs, output-relative crops, compositor generation and stronger process identity. Do not cram every display into an indistinguishable `PrimaryDesktop` or fake a PID when Niri cannot supply one. The same authoritative session must publish and consume captures; keeping a registry only inside the disposable MCP subprocess would break direct clients and restart invalidation.

An action identifies its exact target and route, its capture/snapshot reference where applicable, its delivery semantics and its action intent ID. The Rust module validates permission, current identity/geometry, target bounds, token ownership/freshness, capability and coordination before effects. Niri still rechecks coordination at delivery. Multi-step input validates at each effect and releases held input on interruption. Side-channel mutations are gated by ticket 16's enforceable shared admission protocol, not enabled after a best-effort ownership query. Specify admission versus human takeover ordering, revocation, in-flight completion and unknown-effect semantics without delaying human input. Until that protocol is implemented and race-tested, AT-SPI/CDP are read-only or capability-refused for mutation. If a remote application cannot support cancellation/serialization guarantees sufficient for Dojo's human-priority contract, retain that mutation refusal rather than weakening the commitment.

The default coordinate-action interface uses pixels of the delivered image plus its capture ID. Mapping stays inside Rust. Direct logical-coordinate control is retained only as an explicitly lower-level targeted interface, not an implicit fallback when capture admission fails.

A single-use capture authorizes at most one admitted coordinate action, not an indefinitely reusable action script. For an upstream multi-action model response, default to executing the first observation-dependent action then observing again; batched effects require an explicit audited contract. Define consume-on-admission/failure/paused semantics with tests. Never restore consumed IDs after an uncertain effect. A pixel capture cannot prove that UI content stayed unchanged within the same geometry; fresh observations and postconditions remain necessary.

A result distinguishes validation failure, human pause, capability refusal, delivery/handling, partial or unknown effect and verified postcondition. `handled` is not equivalent to success. Preserve route-specific semantics: accessible `set_value`, CDP fill and physical typing are different operations, not interchangeable silent fallbacks.

### pi-durable owns the run

One path for every action: committed Pi tool intent → Rust admission/execution → structured result → committed Pi tool result → optional fresh observation/verification → next generation. No inner autonomous CUA agent loop.

Explicitly register desktop effects as replay-unsafe and enforce sequential ordering within an agent's desktop tool round, even where MCP annotations describe them as idempotent. A shared service's serial arrival order is not a substitute for preserving model-issued order. Screenshot failure after delivered input reports an unknown effect, never a reason to repeat input. Read-only observations can be safe to rerun, but return newly timestamped IDs; old IDs remain invalid after restart. Pin the tool catalog per prepared model request and version registrations; persisted arguments to an old tool must not be interpreted under a silently changed schema.

Persist a Dojo recovery-required document or equivalent Pi-visible state after unknown/interrupted effects. Require a new observation before another observation-dependent mutation; neither runtime resume nor model reasoning may reuse the old capture. Abort, stop, close and client loss must reach Rust cancellation/release, including blocking IPC, ongoing drag and verification deadlines. No indefinite MCP promises.

Tool-using vision models fit current Pi interfaces first. Native computer-use provider response blocks may not fit pi-ai's current message/content union or credential endpoint. Ticket 08 must prove transcript storage, streaming, native call IDs/results, safety acknowledgments, OAuth/API-key distinctions, replay and restart before enabling them. Add a pinned Pi adapter extension/upstream patch if needed; do not disguise native computer calls as generic tools and claim wire equivalence. If a provider cannot be integrated without an independent scheduler, leave it gated and keep its Rust decoder usable through a supported tool-based model.

## Behavior migration: replace rather than layer

### Observations

Adopt capture IDs, semantic snapshot IDs, element tokens, exact target identity, bounded tree walks, honest coverage/completeness and transform ownership. Add positioned tiled-window geometry in Niri instead of permanently refusing tiled capture. Retain visible-composited/occluded screenshot semantics unless a new isolated-surface mode is explicitly supported and named. No guessed window geometry or matching by title alone.

Replace grim with native Rust capture after output selection, bounded event dispatch, format/stride/transform/color handling and parity tests pass. Keep grim as a temporary selectable fallback during rollout, not a second interpretation of screenshot coordinates.

### Desktop input

Replace model-visible evdev-only interactions with CUA-derived typed click, move, double-click, drag, scrolling-at-point, key/hotkey, text and semantic actions. Expose capability refusals honestly until each route exists. Keep raw evdev for diagnostics/compatibility.

Text needs Unicode/layout/composition and terminal-safety decisions. No hidden clipboard replacement. Clipboard read/write/paste are separate explicit privacy-sensitive effects with approval and retention policy. Clipboard restoration is not guaranteed when a human changes it concurrently; do not promise transactional restoration.

Add in-window drags before cross-window drag-and-drop. The latter needs pointer-grab/data-device policy changes and a new compositor acceptance contract. Window frame/layout actions similarly require a separately admitted policy. Preserve independent human cursor and focus; CUA's real-primary-pointer/foreground defaults are an intentional divergence.

Reuse cursor motion/effects, but reconcile first arrival versus settling, click timing and velocity-preserving redirection. Effects must not delay clicks unexpectedly, mask hit-testing, contaminate observations, or keep idle animation frames running. Update the motion spec when replacing its algorithm; keep its safety acceptance cases.

### Browser and accessibility

Adopt the full MIT browser engine as a coherent module with a Dojo `BrowserPlatform` adapter, not just its WebSocket transport. First support driver-owned isolated Chromium profiles; later support existing profiles with explicit consent. Do not steal a profile, silently relaunch the user's browser, enable debugging on observation, or modify cookies/credentials as an implicit preparation step.

Prove executable/process/window/endpoint identity, including PID reuse and browser subprocesses. Nix commonly uses executable symlinks/wrappers; upstream's strict executable-path selector needs an explicit trusted Nix installation adapter rather than relaxing it to arbitrary symlink execution.

Adopt AT-SPI tree walking, observed-object actions, budgets, truncation and token validation. Match accessible windows to Niri targets with evidence. Permission errors and incomplete walks produce unknown/unavailable, not a claim of absent elements. Semantic actions must observe human priority and not fall back to a global pointer on ambiguity.

### Human clarification and approvals

Inventory CUA `human_tool/` and `adapters/human_adapter.py` request/response behavior. Replace its in-memory completion queue and HTTP polling with a Pi-owned pending-human document/task plus an authenticated Dojo UI response interface. Requests carry stable ID, purpose, bounded payload, expiry and cancellation state; responses deduplicate across restart. Clarification does not grant permission. A granted intent is exact and revocable, and resuming after either kind of human response requires fresh observation/revalidation before mutation. OperatorNormalizer is a syntax normalizer, not an approval mechanism. Tickets 08/10 own this path with UI integration in 12.

### First-release privacy and retention

CUA's `callbacks/pii_anonymization.py` is a TODO/pass-through stub, not a working redactor. Implement baseline Dojo privacy before the first capture release, rather than waiting for 10. Deterministically filter password/secret semantic fields, credential-bearing errors/arguments and trace metadata before durable Pi result/memory storage. Do not claim reliable visual PII detection: captures are explicit, scoped, and may contain unknown sensitive pixels. Deny known sensitive scopes by default; broader/sensitive capture requires informed consent with declared model transmission/storage behavior. Scope a screenshot to the needed target/region and reject capture when the requested privacy guarantee cannot be met.

Separate request-context image limits, Rust capture-store TTL, durable artifact retention and memory-summary policy. Request trimming operates on a derived request, preserving call/result pairing and signatures; it never edits canonical Pi entries or the binary memory tree. Durable artifact deletion/redaction requires a separately defined policy covering provider-visible content, stored entries, memory mirrors, summaries and `zoom()` restoration. Existing Pi image persistence is intentional but cannot be mistaken for an ephemeral capture. Privacy tests cover each storage/retrieval path, not only exported recordings.

### Verification, batching and history

Use CUA's deterministic verification predicates and bounded stable sampling before model-based judging. Preserve true/false/unknown, particularly absence predicates on incomplete trees. Verification is not a hidden second model loop.

Reuse bounded run-steps machinery where possible, but revalidate each step and halt on pause, stale refs or partial effects. Record completed prefix and ambiguous step; do not replay the batch on recovery. Static pause/wait durations are bounded and cancellation-aware, not a background polling service.

Reuse action/trajectory/recording formats with route, identity, capture/snapshot refs, timings, error and verification evidence. Replace process-global hooks with session-scoped injected adapters where necessary. Recording is opt-in, bounded and privacy-filtered; known secrets, clipboard contents, password semantic values and subscription tokens must not enter traces, telemetry or memory. Unknown visual sensitivity is governed by the explicit capture-consent/storage policy, not an unsupported blanket redaction guarantee. Pi remains the canonical conversation archive; Rust trace data references it rather than becoming another agent history.

## Python-to-Rust port method

For each slice: inventory dependencies → extract representative upstream fixtures → define observable behavior → implement a Rust decoder/transform → differential comparison → integrate through the shared action module → delete duplicate Dojo logic.

Normal tests and shipped runtime require no Python. If a one-time isolated fixture-generation step uses upstream Python, keep it out of deployed dependencies and CI's ordinary Rust path; where the user/environment disallows Python entirely, derive fixtures from checked-in upstream tests and documented examples. Generated reference outputs record source revision and generation method. Never execute model-generated Python or shell actions while producing fixtures.

Use Rust enums/serde for action unions, finite/range validation, explicit coordinate spaces, bounded image/history retention and classified errors. Bind every processed model image to a versioned model-view record: capture ID/digest, parser/profile version, preprocessing parameters, model-view dimensions and model-view-to-delivered-image transform. Compose that transform with capture-to-desktop mapping in Rust. Grounding outputs must name this record; never infer the target from the last historical image. Test non-square images, fractional scaling, rounding, boundaries and stale results. Support upstream formats through parser families rather than flattening everything into an untyped JSON catch-all. Port the entire useful behavior of each selected format, including refusal/done/ask-human cases; an unsupported action is an explicit capability result.

Start with generic tool-based vision plus UI-TARS/Qwen-style grounding parsers, then native Anthropic/OpenAI/Gemini formats if Pi can represent their turns. Text-action predictions must become Pi ToolCalls inside a provider adapter before answer classification; the installed Pi `afterResponse` hook cannot transform output. Installed OpenAI Responses conversion has no native `computer_call` path, whereas Anthropic already carries ordinary tool-use/image-result blocks but still needs native declarations and beta integration. Grounding requests are owned Pi tasks/registered models, never provider requests issued by a Rust parser. Budgets and operator/human checks attach to the same durable run. Retired upstream Python means we own fixes; source parity is a baseline, not a reason to reproduce unsafe execution or obsolete model assumptions. Specifically do not copy automatic pending-safety acknowledgments after execution, normalization that discards targeted typing/drag fields, or description-only grounding caches that retain old coordinates. Define image-retention zero semantics and preserve native call/result pairing/signatures in request projections without mutating stored originals.

## Phases, dependencies and release gates

Tickets are individual files under [issues/](issues/); [map.md](map.md) summarizes the graph. Ticket 16 is a safety prerequisite for semantic/browser mutation, although their read-only imports can begin earlier. Later tickets are scoped plans, not all AFK-ready implementation tasks.

### A. Establish the import and session spine — 01–03

01 records provenance/licensing and reproduces extractions. 02 establishes typed contract, shared session ownership, tool versioning and Pi cancellation/recovery. 03 adds authoritative capture storage and capture-bound actions. This is the first useful release: current Pi performs a screenshot-grounded click without doing coordinate math itself, stale/cross-session IDs fail, and stop/restart cannot replay it.

### B. Close core desktop gaps — 04–05

04 adds exact tiled/floating geometry and native Rust capture. 05 adds richer desktop actions and CUA-derived movement behind the existing Niri safety gate. Ship action families separately as capabilities; unsupported drags/text never become advertised by schema alone.

### C. Semantic control and full browser reuse — 06–07

06 implements snapshot-bound AT-SPI; 07 imports browser engine with isolated-profile then consented-existing-profile milestones. Ticket 16's shared admission proof gates their mutation capabilities. These can proceed in parallel after the typed/session spine; geometry and identity proof gate coordinate browser actions.

### D. Rust model behavior ports, verification and policy — 08–10

08 ports model action/grounding semantics, S1's model-independent schema/planner/execution policy once 06 is available, and proves each Pi provider integration. 09 imports deterministic verification and bounded step execution. 10 consolidates authorization/consent/budgets/privacy. Baseline safety/authorization is part of 02, not postponed until 10. Baseline fresh-state verification is part of 03; 09 adds richer predicates. Baseline trace/receipt metadata begins in 02; 11 expands it into exportable recordings.

### E. Productization and evaluation — 11–13

11 recording/history and Rust benchmark/evaluator ports. 12 configuration installation, icon cache, transcript/event normalization and health diagnostics. 13 migration/default cutover, parity evidence, duplicate removal and docs/spec reconciliation. No feature goes default solely because its standalone tests pass.

### F. Inference, sandbox and remote parity — 14–15

14 Rust specialist inference/perception ports and artifact gates. 15 QEMU/OCI
disposable desktops and separately licensed remote products. Both are required
for the full parity target. They cannot substitute for finishing local correctness.

There is no reliable calendar estimate from source inspection alone. Effort bands: small = bounded extraction/manifest work; medium = multi-module Rust port/integration; large = compositor/identity/semantic subsystem; exploratory = backend/license/provider feasibility can change scope. Tickets carry these bands. First three tickets are the critical path; later phases have independent streams, not a compulsory serial monolith.

## Validation and rollout

1. Source parity: byte comparisons for unmodified imports; upstream tests and generated schema/fixture diffs; translated behavior differential cases. Treat upstream assertions as evidence, not an excuse to bypass Dojo integration tests.
2. Rust interface tests: capture lifetime/consume rules; scale/crop/transforms; process/generation reuse; cross-session denial; capability errors; stable verification and unknown semantics; budgets/timeouts; partial effects; held-input cleanup; malicious/malformed model output.
3. MCP/direct/Pi conformance: same typed arguments/results across adapters; images preserved; schema version/replay policy fixed; cancellation delivered; late responses bounded; restart at before-intent, after-intent, during-effect and before-result storage. No default replay of desktop effects.
4. Live nested desktop: floating and tiled windows, multiple outputs and scales, native and primary-seat-only clients, two agents, human takeover during movement/text/drag/semantic action, lock/output removal/disconnect/service crash. Assert human cursor/focus/clipboard isolation, actual delivery and zero idle animation/polling work.
5. Browser: real isolated Chromium with local pages, DOM changes/stale refs, frames/OOPIFs, dialogs, downloads and lifecycle; refuse wrong endpoints, PID reuse, missing geometry and absent consent. Existing-profile tests use a disposable test profile, never the user's authenticated profile by default.
6. Accessibility: known fixture app and representative applications, incomplete trees, duplicate names, changing element paths, password fields, disabled controls and bounded D-Bus timeouts.
7. Benchmark: deterministic task setup/teardown, trace round-trip, evaluator gold cases and failures, fixed models/tasks/seeds where possible; compare current Dojo and new routes on success, false-positive verification, latency and resources. Do not invent target percentages before establishing a baseline.
8. License/privacy: audit direct/transitive dependencies and model/dataset notices; inspect recorded exports and memory for sensitive content; confirm config edits back up and reverse cleanly.

Use documented devenv commands for Rust changes: `devenv shell -- cargo check`, `devenv shell -- cargo test`, focused crate tests, `devenv shell -- cargo build --bin dojo-agent`, and compositor `devenv shell -- cargo build --manifest-path vendor/niri/Cargo.toml --no-default-features`. Run `node --test tools/speech/*.test.mjs` for Pi host changes. Reuse `tools/agent-input/run-nested.sh` scenarios and add Rust-based scenarios for new contracts; older Python development checks are not runtime dependencies, and new acceptance coverage should not require Python. Feature-specific commands go into each ticket as its implementation is finalized.

Roll out per capability behind explicit configuration. Keep old MCP calls working through translating adapters for a documented transition, but register only one preferred surface in Pi to avoid duplicated tool schemas. No automatic fallback from a denied or stale semantic action to pixels. No fallback from independent-seat input to uncontrolled primary pointer injection. Once parity and safety gates pass, switch defaults, remove obsolete local helpers and temporary backends, and update relevant specs. Deployed binary/compositor/config installation is a separate explicit operation using existing backed-up installers; planning and builds do not replace the live desktop.

## Definition of done

Local adoption requires tickets 01–13 and the side-channel mutation gate 16.
Full parity also requires tickets 14–15 and every operation in the pinned
Driver and Spaces contracts, with equivalent Rust implementations and platform
adapters. Capability refusals must remain visible gaps when upstream supports
the operation. Retained divergences require a documented constraint and evidence.
All source, behavior and artifact references must pass immutable provenance checks.
The [full capability and source index](full-parity-reference-research.md) is the
coverage baseline. Passing local tests does not close remote or host-specific rows.

Not done: simply vendoring CUA, exposing schemas without implementations, a standalone cargo test, a parser that executes generated code, or a second hidden agent runtime.
