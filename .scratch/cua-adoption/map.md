# CUA adoption map

## Notes

The current target is full parity at CUA commit
`88aff7e5424e9ed45b93ded85eb4111472ca5190`, with immutable pins for every
referenced source, dependency and artifact. Tickets 14–15 are required tracks.
The [full parity index](full-parity-reference-research.md) inventories the pinned
29 Driver and 86 Spaces tools plus inference, evaluation and hosted products.
Pin coverage is tracked in `provenance/reference-inventory.json`; unresolved
references remain failures in the strict check.

After this pass, discuss durable approvals and Dojo notification management.
Ticket 10 remains the approval implementation owner. Clarification responses
must never count as permission grants.

User direction: take as much CUA as useful, translate Python to Rust, and replace weaker Dojo behavior—not just wrap it. Preserve Rust desktop execution, pi-durable scheduling/persistence and pi-ai provider ownership. [Full spec](spec.md). [Prior source/extraction report](../../docs/research/cua-system-portability.md). [Exact Python port inventory](python-port-inventory.md).

Implementation now spans the capture/input/session spine, native geometry/capture, read-only accessibility/browser ownership, model policy ports, verification, offline recording/evaluation and setup/cache slices. The latest nested capture/input/Pi proof is `/tmp/dojo-capture-live.tFegd3`, on the inactive `dojo-tests` workspace. Operator-opt-in navigation has an actor/broker implementation and launcher enrollment but no real-browser route proof. Full ticket acceptance remains gated; import or standalone success does not enable unrelated capabilities.

## Decisions so far

- Reuse-first: whole MIT Rust crates/coherent modules plus tests/fixtures; bounded Rust translations for useful Python behavior.
- Replace the model-facing evdev-only interface with capture/snapshot-bound actions, text/hotkeys/drag, semantic observations, full browser engine, deterministic verification and structured outcomes.
- Keep one Rust session authority shared by MCP/direct socket; Pi is the only durable run owner. The existing Node Pi host remains deliberately, not as an excuse to put desktop logic in JavaScript.
- Capture registry integration is new work even upstream; its tested standalone source is not proof of wired action freshness/permission.
- All mutating routes must respect human priority. CDP/AT-SPI cannot be input-gate bypasses.
- FSL/AGPL/model/dataset artifacts and non-Linux/cloud products have explicit gates, not automatic import or automatic rejection.

## Work packages

| Ticket | Deliverable | Depends on | Effort |
| --- | --- | --- | --- |
| [01](issues/01-import-ledger.md) | Pinned source, licenses, provenance and reproducible extraction | — | Small/medium |
| [02](issues/02-session-contract-pi.md) | Typed Rust module, session spine and Pi recovery/cancellation | 01 | Medium |
| [03](issues/03-capture-bound-actions.md) | Authoritative captures and image-coordinate actions | 01, 02 | Medium |
| [04](issues/04-geometry-native-capture.md) | Tiled geometry and native Rust capture | 02, 03 | Large |
| [05](issues/05-rich-input-motion.md) | Rich input and CUA motion under Niri safety | 02, 03 | Large |
| [06](issues/06-atspi-snapshots.md) | AT-SPI snapshots and semantic tokens/actions | 01, 02, 03 | Large |
| [07](issues/07-browser-engine.md) | Full browser engine and Niri identity adapter | 01, 02, 03, 04 | Large |
| [08](issues/08-python-models-to-rust.md) | Rust model/grounding ports; Pi-native provider feasibility | 01, 02, 03 | Medium/exploratory |
| [09](issues/09-verification-batches.md) | Deterministic verification and bounded batches | 02, 03 | Medium |
| [10](issues/10-policy-consent-privacy.md) | Consolidated authorization/consent/budgets/privacy | 01, 02, 03 | Medium/large |
| [11](issues/11-recording-rust-bench.md) | Recording formats and Rust benchmark/evaluator ports | 02, 03, 09, 10 | Large |
| [12](issues/12-setup-transcripts-health.md) | Config editor, transcript/icon reuse and diagnostics | 01, 02 | Medium |
| [13](issues/13-cutover-parity.md) | Default cutover, parity and duplicate removal | 04–12, 16 | Medium/large |
| [14](issues/14-rust-specialist-inference.md) | S1/perception Rust inference and artifact proof | 01, 06, 08, 10, 11 | Exploratory/large |
| [15](issues/15-sandbox-remote-products.md) | QEMU/OCI and separately gated remote-product reuse | 01, 02, 10, 11 | Exploratory/large |
| [16](issues/16-side-channel-admission.md) | Enforceable side-channel human-priority admission | 02 | Exploratory/large |

## Execution order

Critical spine: **01 → 02 → 03**. First release demonstrates a Pi screenshot-grounded click with server-owned mapping, stale/cross-session rejection and interruption safety.

After 03, parallel streams are native geometry/capture (04), richer input (05), AT-SPI (06), Rust parsers/provider research and later S1 pure policy (08), window verification (09) and policy consolidation (10). Browser engine (07) depends on geometry proof. Ticket 16 can begin after 02 and blocks mutation enablement in 06/07; read-only functionality can land first. Setup/transcript/icon slices (12) can run after 02. Recording/bench (11) follows verifier/privacy baselines. Semantic predicates in 09 and semantic benchmark tasks in 11 require the corresponding 06/07 capabilities even though basic implementations can start earlier.

Release capability slices individually. 13 is the final parity/default milestone, not a requirement to withhold every improvement until all tickets finish. Native provider formats that lack Pi/endpoint support remain explicitly gated and do not block supported tool-based models. 14–15 expand maximum reuse after feasibility/product/legal decisions.

## Fog / gates to resolve

- Exact transitive dependencies for larger CUA imports and replacement of process-global hooks/session owners.
- Native computer-use message/stream/result representation in pinned pi-ai/pi-durable and subscription endpoint availability.
- Capture admission consumption on pause/failure; process/output identity when Niri PID evidence is missing.
- Enforceable admission/takeover ordering for side-channel semantic/CDP effects (16); mutation remains refused until this release gate passes.
- Unicode text/composition and in-window versus cross-window drag support without global injection/clipboard surprises.
- Native capture formats/deadlines and Niri tiled geometry under transforms/occlusion.
- Rust inference/export/tokenizer/LoRA equivalence and separate artifact terms.
- FSL product use/distribution decision and whether remote sharing/secret transfer is actually desired.
- No calendar estimates until integration spikes establish real effort.

## Implementation evidence, 2026-10-12

- 01: 13 ledger/verifier tests pass; the stable-lock audit covers 15 manifests and explicitly separates active normal/build packages from wider metadata. Consolidated per-file ledger, immutable-copy refresh, reproducible pinned-source patches and executable dependency audit cover the current adapters and root/compositor glue. Read the current ledger count instead of historical 84/298-file counts. Third-party declared licenses, missing notices and review requirements are explicit; no blanket approval is recorded.
- 02–05, 09: the shared Rust service, one-use captures, native Wayland capture, same-render Niri scene and guarded rich input have nested evidence. The latest passing gates include text, bounded hold, drag, fresh observations, same-render bounds verification and takeover/geometry interruption. Pi drives the real MCP service, preserves images and blocks replay. Complete authorization/privacy and client/output/crash matrices remain open.
- 06–07, 16: accessibility/browser reads have explicit scoped owners. Operator-opt-in navigation uses native actor serialization and an authenticated broker, defaults disabled and lacks real-browser route/installed proof. AT-SPI effects remain refused without exact top-level correlation. General semantic mutation admission is not proven by the narrow writer fixtures.
- 08: 25 pure Rust model/policy cases pass; supported tool-based Pi bridges, immutable image request projection and durable human clarification state exist. Native provider wire/subscription support, durable permission grants and tensor/model artifact gates remain separate.
- 11: 23 Rust recording/processor/grounding tests plus 21 unchanged transcript conformance tests pass. An isolated CLI fixture set produces success/failure/unknown rewards. The thin Pi exporter reads canonical entries. Live benchmark tasks, external artifacts/application metrics and model baselines remain open.
- 12: setup plan/apply/reversal and bounded Linux icon-cache adapters are built; explicit installation and live launcher/UI behavior need their own evidence.
- 13–15: broader parity/default cutover, specialist inference, disposable desktops and separately licensed products remain open. No ticket is marked resolved by this map.

See [current implementation evidence](result.md) and [audit commands and scope](../../vendor/cua/README.md).

## Resumed portable units

- Ticket 09: `dojo-computer-verification::steps` and the installed Pi `computer-steps.mjs` facade provide fresh-observation single-step transitions, bounded prefixes, durable intent gaps and cancellation without automatic replay. Exact browser DOM identity/coverage adapters support title/URL and full-coverage absence; production browser acquisition remains its owner's separate integration work. Evidence is 24 Rust and six Node tests (five real Pi/Rust-codec cases) with fixture desktop responses.
- Ticket 11: `dojo-computer-recording::usage` ports callback accumulation as typed bounded integer totals. Missing costs stay explicit; no model budget owner or telemetry sink is added. The recording suite now has 23 passing tests.
- Audit: refresh discovers new step/root test glue, verification provenance and declared private-core additions. It refuses classifying any existing pinned upstream object as a new local addition. The source-stable ledger covers 484 maintained files, 28 exact patches and 54 behavior-source objects. All 509 existing Niri lock package identities and checksums are preserved; the launch attester adds `sha2 0.10.9`. The inventory excludes generated worktree copies and still has 4951 unresolved references; full contract parity still has zero verified rows. Successful implementation tests do not close either gate.

- Browser launch admission: five fixed-recipe child-launch tests and eleven native admission tests pass. Native sockets authenticate process identity using `SO_PEERCRED`/`SO_PEERPIDFD`; launcher enrollment verifies the child pidfd, private control socket, ancestry, recipe and compositor generation. Writers refuse foreign native peers and dead process/control generations. Service launch integration, per-session ingress and cancellation, queued resource cleanup and the actual Chromium route remain open.
