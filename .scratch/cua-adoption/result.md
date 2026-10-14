# CUA adoption implementation evidence

Source is pinned at `88aff7e5424e9ed45b93ded85eb4111472ca5190`. Rust adaptations, thin Pi integration and nested capture/input slices now work. The complete roadmap remains unfinished, and no installed desktop/runtime was replaced.

## Read-only discovery and evaluator batch

The MCP server now exposes `list_windows`, `get_screen_size` and `list_apps` with input schemas derived from the pinned Driver manifest. Pi negotiates exactly MCP `2025-06-18` and refuses a protocol downgrade. Discovery uses no agent seat. The launcher and service share `src/application_catalogue.rs`.

The nested acceptance run at `/tmp/dojo-capture-live.ne4VcG` passed the pinned schemas, annotations, protocol and discovery checks together with capture/input/Pi recovery in 29.71 seconds. The three tools remain partial parity. Window listing still uses cached layout, application discovery covers window-owning processes, and application identity/launch fields need upstream semantic alignment.

Native bounds verification now requests compositor geometry without PNG readback. Its regression first failed when the mock compositor refused pixels, then passed with `WindowPlacements`. The Niri headless test passed all eight transforms at scale 1.25. An earlier loaded run correctly refused an action after capture geometry changed; its evidence remains at `/tmp/dojo-capture-live.VaoqqD`. These runs did not replace installed Niri.

The recording crate adds ten typed general metric families and 25 actual CLI fixtures. Its suite passed 29 tests. The optional pinned CPython comparison matched 24 of 25 cases, with one explicit missing-key/null divergence. The three fuzzy cases now match hash-pinned RapidFuzz 3.14.6 exactly after correcting the Rust floating-point operation order. Root library passed 47 tests with one ignored; speech tests passed 92; the shell's non-Tokio tray regression passed.

## Perception artifact acquisition

The public perception 0.2.1 Linux archive was acquired and checked against SHA256 `b4d76b626df9ce1a48b8036f7313412525295d0cb818d82ecbe3d9cca5fd1d08`. The detector, both OCR models, dictionary and ONNX Runtime match the existing pinned artifact lock. `scripts/check-perception-artifacts.mjs` independently verifies the archive and six extracted files.

The release was built from `6aa37d0751d094bfe8fdc2df331ed9eae934c832`, which differs from our baseline. Its executable is not accepted as our worker. Worker source will be built from the baseline, with the release used only for verified artifact bytes. The bundle is machine-local cache, not tracked source or production inference proof. Durable approvals remain reserved for discussion.

## Latest capture, rich input and Pi evidence

`/tmp/dojo-capture-live.M0zbYv/result.json` records passing capture/input gates. The isolated nested scenario proves target input delivery, cross-session/replay refusal, stale geometry refusal, human takeover, occlusion safety, held-input cleanup on disconnect, physical ASCII text, bounded key hold, in-window drag, fresh execute-observe, capture-correlated bounds verification and release during both human takeover and same-window geometry changes. `/tmp/dojo-capture-live.M0zbYv/pi-model-result.json` adds a real Rust/MCP textual UI-TARS pipeline: Pi observes, Rust decodes the retained PNG, Pi admits exactly one action with parser/raw/digest metadata, and the receipt supplies a fresh image. Decoder attempts now consume the same durable tool budget. Model AskHuman outcomes become durable clarification tools. `/tmp/dojo-capture-live.M0zbYv/pi-result.json` proves Pi's actual durable scheduler called the real MCP service, retained images and blocked repeated effects before dispatch.

The service owns opaque one-use captures and exact process/compositor/window evidence. Niri checks the actual pointer/keyboard target throughout guarded rich input and uses CUA motion easing. Consumed captures, sibling observations, disconnects, target changes and takeover invalidate later effects. Unknown transport outcomes do not trigger automatic replay.

The original intermittent refusal is now reproduced and fixed. A targeted movement toward an already human-owned application silently suppressed agent pointer focus, and the next button check mislabeled that ownership pause as target mismatch. Niri now checks human ownership immediately after the exact destination hit test and before movement or pointer-focus validation. The actor regression failed before the fix (`/tmp/dojo-preowned-red.log`) and all 15 focused tests pass after it (`/tmp/dojo-preowned-green.log`). The latest 18-gate live fixture explicitly claims the application first: its receipt is paused, has zero handled effects and consumes the capture. A fresh observation after leaving the application allows normal input. A hidden nested Winit host stalled animation and produced a bounded transport failure in a separate diagnostic run; the acceptance fixture stays visible. No target checks or human priority rules were relaxed.

Native Rust named-output Wayland capture is the default; `DOJO_CAPTURE_BACKEND=grim` explicitly selects the migration backend. Window captures use Niri `CaptureScene` pixels and geometry from the same render. Transitions without authoritative geometry refuse. This evidence comes from disposable nested clients and a scripted provider, not a live installed replacement or a real remote model.

Relevant checks:

```sh
devenv shell -- cargo build --bin dojo-agent
devenv shell -- cargo build --manifest-path vendor/niri/Cargo.toml --no-default-features
devenv shell -- cargo build --manifest-path tools/agent-input/Cargo.toml --locked
devenv shell -- cargo test --test capture_desktop -- --ignored --nocapture
```

The latest reported focused suites are 31 MCP unit tests, 12 capture-authority tests and 3 motion tests. Root library tests pass 43 cases. The current Node speech/Pi suite passes 91 cases. The shell suite passes 96 cases with one ignored case. The final native input regression suite passes 16 cases, as recorded in `/tmp/dojo-captured-path-final-tests.log`. The focused Pi external-effect suite passes nine cases, including SQLite reopen and blocking recovery after an unknown effect. Root `cargo check --all-targets` also passes with the regenerated lock. Their counts describe the current tested slice and do not resolve the complete capability matrices.

## Model, request projection and human state

`dojo-computer-models` has 25 pure Rust gold/security tests. It includes bounded action/native-block decoders, preprocessing metadata plans, capture-scoped grounding, budget/image-retention policy and S1 schema/planner/driver policy. `computer-model-bridge.mjs` supplies supported Pi tool-based adapters while Rust owns parser/domain behavior. Native formats still require their own wire, durable transcript, endpoint/subscription and credential evidence before claiming provider parity. Tensor inference and model artifacts remain separate gates.

`image-retention.mjs` projects requests with explicit 0/1/N retention, preserving canonical durable entries, call/result relationships and signatures. The default request projection retains eight images. Its behavior corresponds to the pinned Python image-retention callback, with the Python zero-slice defect removed. `human-requests.mjs` stores clarification state through Pi and provides an authenticated UI hook. The reported 24 human/voice tests prove bounded clarification lifecycle behavior; clarification documents grant no execution permission and resumed coordinate effects still require a fresh observation.

## Browser, accessibility and admission

The complete private CUA core/contract minimal-feature build passes. Explicit snapshot/browser owners avoid the ambient runtime and lifecycle paths that could revive closed sessions. Read-only BrowserEngine observations retain safe descriptions and frame-bound refs.

Operator-opt-in browser navigation is now wired through service/MCP. It requires a fresh native capture, exact same-window browser episode, pre-effect retirement of refs/captures and serialized native-actor `Page.navigate` writes. It is disabled by default. Actor/real socket/SCM broker fixtures are focused proof of the writer mechanism. Real Helium/MCP owned-browser acceptance is recorded at /tmp/dojo-owned-browser-live.Pas0iX, including correlated refs/semantic evidence, exact child-generation cleanup and profile removal. Installed runtime, generalized CDP effects and existing-profile grants remain gated.

`desktop_accessibility` supplies bounded read-only AT-SPI application observations with generation/bus checks, truncation and sensitive-value filtering. Bounds and display indices are not action references. AT-SPI mutation remains refused because exact top-level native-window correlation is unavailable.

`desktop_verify` evaluates bounded window predicates and stable observations. Missing/unproved semantic evidence remains unknown. Capture-correlated rendered bounds now participate in verification. Handled input alone is not verified task success.

## Recording, offline evaluation and setup

`dojo-computer-recording` passes 23 Rust tests. It redacts before retaining bounded event projections, validates opt-in image artifacts, exports ATIF-v1.8, analyzes effect/receipt gaps and bounds transcript input through the unchanged MIT renderer. Typed usage aggregation adds duplicate/overflow refusal, cached-token subset checks and explicit missing-cost coverage; Pi remains the budget owner. All 21 retained transcript conformance tests pass.

The offline Rust episode CLI runs setup, answer recording, deterministic grounding evaluation, trace and cleanup in isolated memory. Literal ScreenSpot box/polygon/infeasible cases preserve upstream scoring. The fixture task set returns rewards 1, 0 and unknown. GUI-R1 ports annotated-HTML target filtering and normalized boxes; Aguvis ports seeded augmentation, bounded RGB border shrinking and restricted predefined actions with explicit coordinate spaces. External datasets, full application evaluators, live task setup and real model baselines remain unverified.

The thin `desktop-trace.mjs` exporter reads Pi's canonical entries and sends projections through Rust privacy/validation. It creates no second conversation database or effect-replay loop. Setup adaptations retain format-preserving plan/apply/backups and reversal behavior; application is explicit. The icon cache has bounded per-owner state and a Linux adapter, with actual launcher/UI integration separately tracked.

## Resumed bounded-step and semantic work

The verification crate now passes 24 Rust tests, including nine pure phase/restart/fake-clock and browser-identity/coverage cases. `computer-steps.mjs` is installed through the existing Pi runtime and guarded desktop executor. Five actual Pi/Rust-codec integration tests plus a pure reference-normalization test prove fresh observation per effect, SQLite reopen without uncertain replay, recovery from a persisted in-flight intent and stop during a pending effect. Desktop transport in these tests is a deterministic fixture. The protocol stores only bounded current intent metadata, counters and completed prefixes; it creates no action loop. Handled delivery and reaching the step limit remain distinct from task verification.

Browser DOM evidence requires exact process/window generation binding. Negative element absence additionally requires a complete, current, unfiltered walk with no missing frames. Ref-only or incomplete observations remain unknown where coverage cannot establish the predicate. Unbound native AT-SPI evidence remains unavailable. Production owned-browser acceptance is cited separately above; these pure crate tests do not establish installed runtime acceptance.

The callback audit in the recording README accounts for trajectory/logging usage, image retention, budgets, prompt instructions, operator validation, PII, telemetry and raster annotations. External telemetry sinks and application-specific metrics were not introduced. The final stable refresh verifies 414 maintained files: 223 unchanged copies, 23 explicit modifications and 168 locally owned adaptations/glue files. All 23 unified patches reproduce exact maintained bytes from pinned git objects, and 48 behavior source objects validate. The optional host policy and owned-browser additions are included; copied modifications still require an explicit reviewed classification.

## Import and dependency audit

The authoritative maintained-file count is `vendor/cua/ledger.json`'s current `files.length`. Historical 84/298-file counts are earlier snapshots. The refresh tool cannot bless changed copied source; declared modifications receive canonical pinned-source diffs. The verifier independently applies those diffs and checks exact resulting bytes, translated source objects and all 72 upstream manifest/governing-license hashes.

```sh
node --test vendor/cua/ledger-tools.test.mjs vendor/cua/verify-import.test.mjs
node vendor/cua/verify-ledger.mjs --source /tmp/cua-portability-research
devenv shell -- node vendor/cua/audit-dependencies.mjs --write
node vendor/cua/refresh-ledger.mjs --source /tmp/cua-portability-research --write
node vendor/cua/verify-ledger.mjs --source /tmp/cua-portability-research
```

The rerun against the stable root/compositor locks covers 15 manifests and 928 distinct package/version/source identities. All 14 ledger/verifier tests pass. It separates the root's 552 normal/build packages from its 622-package metadata graph, and Niri's 282 normal/build packages from its 417-package metadata graph. Every third-party package has a declared expression. The local `dojo` package has no declared license. Available license/notice texts and source copyright declarations are hashed; 67 package identities have no local notice file found, including 40 active root packages. Across all maintained manifests, 52 notice exceptions occur in normal/build closures and another 15 only in wider metadata. Niri has 14 normal/build notice exceptions; the root has 40, including 38 registry/git packages. Exact names/versions, available source copyright evidence and review-needed expressions remain explicit. This is an executable evidence audit, not approval of distribution or compatibility.

## Remaining gates

Final focused product evidence adds 30 passing model-bridge/desktop/runtime tests. Eight concurrent summary jobs sharing the foreground model ID remain ordinary summaries while the foreground alone decodes and dispatches. Host-marked request objects determine scope before transport; no global mode is used. The old ID-only bridge fails the same real Pi concurrency fixture. Cleanup is available at a saturated budget and preserves unknown external-effect barriers. Actual Rust catalog step tests validate nested Region, drag, ImagePoint and model provenance references, rejecting invalid input before transport while preserving the ready capture. Four canonical usage export tests include one using Pi's immutable entry API and the actual Rust codec; reported costs are estimates, not billing measurements.

The latest captured pointer scope proof is `/tmp/dojo-capture-live.M0zbYv/result.json`: all 19 live gates pass, including a path across a second application that delivers captured agent-seat events only to the admitted window. Its native regression suite passes 16 tests. Production installed Niri was not replaced by this nested fixture.

Complete authorization/consent/privacy budgets and durable approvals, native provider endpoint/wire parity, broader output/client/crash matrices, exact semantic window correlation, generalized browser/AT-SPI effects, dataset/model artifact terms, application-specific evaluator parity, installed deployment and final duplicate/default cutover remain open. Standalone tests, source imports, scoped actor proofs and the live capture/input fixture do not close those unrelated gates. No ticket is marked resolved by this evidence file.


## Optional host policy

`DOJO_DESKTOP_POLICY` selects a private, bounded YAML policy using CUA's evaluator. It adds restrictions to dispatch and never grants capture authority, browser profile consent or human ownership. The service binds its socket and handshake to the policy digest and refuses a changed file before dispatch. Cleanup remains available. Configuration is host owned; model tool arguments cannot replace the file or evaluator. The direct compatibility screenshot, context, launch and feedback boundaries also enforce this restriction.

`/tmp/dojo-cua-policy-final-green.log` records 31 MCP unit tests plus an actual MCP/service test which denies native actions, isolated browser creation, compatibility screenshot and application launch before compositor transport. The no-default-features, YAML-enabled private-core unit suite passes 974 cases after the sensitive-DOM-label regression. Preserved upstream external integration tests target CUA's ambient public APIs and do not compile against the intentionally private Dojo host facade; this is a separate upstream interface gate, not a claim of full stock-driver parity. Durable human permission approvals remain separate from this operator restriction and from clarification state.


## Final integrated rerun

`/tmp/dojo-cua-final-rust-integrated.log` passes root library 43, shell 96 with one ignored, MCP 31, and one actual policy dispatch integration test. `/tmp/dojo-cua-final-node-integrated.log` passes all 105 tests, combining 91 speech/Pi/product tests with 14 source-audit tests. `/tmp/dojo-cua-final-live-integrated.log` reruns both actual Helium ownership/refs/verification/cleanup and the 19-gate capture/input/Pi scenario successfully against the final sources. `/tmp/dojo-cua-final-ledger-proof.log` independently verifies all 414 maintained files and reproduces all 23 exact pinned-source patches. Browser verification bounds use the same output-relative logical convention as native verification; the offset-output regression is included in the MCP count.

## Current continuation

Read-only discovery now includes bounded live Linux process enumeration, CUA launch-command and bundle-ID metadata, StartupWMClass matching and the reserved Linux active flag. Background process fixtures pass. Mtime-based launcher ambiguity, complete per-app window records and exact output/error conformance remain open. The public parity index records four tools as partial, with hashed evidence; no row is yet fully verified.

The real source-built perception worker executes against immutable local Nix artifacts. The latest complete nested acceptance passed in 86.96 seconds, with evidence at `/tmp/dojo-capture-live.JpTLYF`. It includes MCP cancellation, capture retirement, recovery of another session's parse, native typing, holds and drag. The inactive-host timeout was caused by EGL swap interval 1 throttling the nested Winit event loop; the source fork now explicitly selects interval 0 for that non-vsync backend. Neither request deadlines nor human takeover checks were weakened. Both nested desktop fixtures share workspace isolation and verify that the originally focused workspace remains focused.

## Launch enrollment continuation

The fixed-recipe launch boundary passes five tests using a Nix-staged ELF child. It clears ambient environment, authenticates the inherited private channel using kernel credentials, and owns a pidfd for process lifetime. Arbitrary invocation, changed recipes and injected environment entries are refused. This is launch provenance, not permission to perform an effect.

Native IPC now captures `SO_PEERCRED` and `SO_PEERPIDFD` at accept and attaches the opaque process identity to the agent seat. Niri has an explicitly selected launcher/recipe admission mode, mutually exclusive with static broker enrollment. It accepts a selected launcher's child pidfd and private control socket, checks child ancestry and recipe/compositor generations, and binds each navigation writer to a native peer owned by that child. Writer dispatch rechecks process/control lifetime and peer ownership. Existing uncertainty barriers remain intact.

The lazy service launch connection, separate per-session ingress and cancellation, prompt cleanup of queued descriptors/event sources, and real Chromium/MCP route remain unfinished. The new enrollment mode has not been deployed or advertised as full browser parity. Broader Driver/Spaces, platform, inference and artifact gates remain open.

The shared service now includes owned-browser lifetime in its next wake-up calculation. Previously a session with only an owned browser and no other deadline could sleep indefinitely. The lease regression and all 42 MCP binary tests pass.

The service connection's cancellation thread now owns an opaque native retirement handle. EOF or a failed response closes the session's native connection before serialized disconnect handling, even while an external request holds the main loop. Controller release and transport failure close retained aliases. Niri checks socket `POLLRDHUP` as well as process lifetime, so unread protocol bytes cannot keep a closed connection admitted. The blocked-reply socket test, two IPC socket tests and three native peer tests pass. The full live capture/input/perception/Pi run passes in 73.15 seconds at `/tmp/dojo-capture-live.J1FFDc`; installed Niri and the user's active workspace were unchanged. Prompt retirement of transferred writer descriptors and their actor sources remains separate unfinished work.

The pin inventory now excludes `.delta` generated worktree copies while retaining `.scratch`, vendored source and local crates. Four pin-tool tests cover that scope and immutable-reference classification. The inventory has 38 locks, 5711 references and 4951 unresolved references. This scope correction does not pin any previously unresolved input.

Real-browser creation, exact ref ownership, verification, explicit close/process cleanup and idle expiry pass in the inactive workspace at `/tmp/dojo-owned-browser-live.PrLkOV`. The expiry scenario resets native input to retire semantic observations, confirms the profile still exists, then waits for profile deletion without another service request. The earlier failed-action variant closed the MCP service connection and therefore measured disconnect cleanup, not expiry; it was replaced before recording this acceptance claim.

The final capture/input/perception/Pi acceptance passes in 73.18 seconds at `/tmp/dojo-capture-live.tFegd3`. EGL swap interval is now selected after `render_output`, which makes the draw surface current; Smithay `bind()` constructs a framebuffer without guaranteeing that surface is current. The final nested log contains no swap-interval failure warnings. Outer Niri reports `dojo-tests` inactive and the original workspace focused. This elapsed time is an acceptance-run duration, not a performance comparison.

The general evaluator suite passes 29 tests. The optional CPython 3.14.8 reference harness extracts RapidFuzz's pure reference implementation from its exact upstream-lock archive and verifies its hash and size before executing it. This is development-only; production remains Rust. MIT attribution and the source archive/file identities are retained in the reference artifact lock.

Approval discussion settled persistence ownership. Pi owns pending requests, responses and execution results; OS configuration durability and recovery remain with Nix, devenv, Home Manager and Btrfs. Outcome-oriented approval/options flows and execution enforcement remain work. No extra approval database was added.

## Whole-source copy and Pi transport

The copy-first import now contains the complete Driver workspace and its Wayland helper, 22 MIT Cua packages, three Fleet packages, and support assets from the pinned baseline. `vendor/cua/copy-upstream.mjs` reproduces imports from Git objects and records explicit workspace, lock, and fork adaptations. The [copy record](../../vendor/cua/copy-first.md) owns the current scope and commands.

The copied SDK topology and creation suites pass 19 tests. The original CLI sandbox and MCP skills suites pass 18 tests. All 66 original Driver contract tests pass, and locked SDK builds pass. The actual Rust CLI supplies 138 tools. Pi registers 69 explicitly selected resource contracts, checks their schemas, and calls `list_spaces` through a real MCP process. The combined Pi tests pass 27 tests.

The new integration does not expose CUA's scheduler, providers, approval tools, or generic dispatch. No new approval database or permission grant has been added. It remains opt-in and has not changed installed configuration. Full Niri execution and strict parity remain open.

## Driver socket integration, 2026-10-13

The whole pinned Driver now runs as a private inherited-stdio worker owned by
one connection to Dojo's existing activated desktop service. Pi discovers the
admitted original runtime schemas; no second scheduler or public daemon was
added. A shared native seat library connects the original click/key/text/scroll
and focus handlers to Niri's capture and ownership checks. Worker artifact,
imported implementation, fork patch, locks and local IPC sources are pinned.

Current focused checks pass: 44 dojo-agent tests, 26 original Driver Niri tests,
6 real-child worker transport tests, 24 Pi/MCP tests, 7 copier tests and the tray
runtime regression. Live evidence at `/tmp/dojo-capture-live.3kgZ2g` proves
actual original capture/action calls, literal text transitions, reuse refusal,
excluded-tool refusal and worker cleanup in inactive `dojo-tests`.
`provenance/evidence/cua-driver-service.json` retains the relevant native results
and events. Five Driver parity rows are partial; full parity is not claimed.

The actual host read check at `/tmp/dojo-driver-host-read.ia5Icd` confirms Driver
startup and screen discovery, but the running Niri
`26.04 (7fb1e4ffa7-modified)` rejects WindowPlacements. Updated native compositor
code is verified in the isolated process. Host activation requires installing
that compositor and a new login; issue 13 requires separate backed-up deployment
authorization. Browser/accessibility mutations, Unicode input, multi-output and
clipped-window capture remain parity work.

The user approved deployment on 2026-10-13. The existing installer selected
`~/.local/share/dojo/niri/face129289992a2d` for the next login and retained
`4677bd2c0bdd6fea` as `previous`. Installed bytes match the recorded release
acceptance artifact. UWSM selects the Dojo wrapper. The existing compositor
session was not restarted; host Driver acceptance resumes after the new login.

New-session host acceptance passes at `/tmp/dojo-driver-host-window.F5u7Ag`:
window discovery, screen discovery and exact-window capture with a bound PNG.
The active compositor is the approved updated fork. Fractional tiled dimensions
and per-window output selection were corrected in the Driver; 28 Niri tests and
release input acceptance at `/tmp/dojo-capture-live.a0MsVB` pass. Whole-desktop
multi-output capture remains refused.
