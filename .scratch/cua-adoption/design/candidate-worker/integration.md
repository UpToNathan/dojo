# Integration and evidence

Design only; no executable changes in this candidate.

## Modules and import closure

| Owner | Responsibility |
| --- | --- |
| `dojo-computer::CaptureSession` | Resolve capture, disclosure policy, private binding, TTL and held-byte reservation; atomically validate completion. |
| Private `dojo-computer-perception` backend | Trusted artifact installation, capacity admission, child ownership, strict protocol, containment, output normalization. |
| Connection-owned service session | Construct opaque read context; dispatch asynchronous read and receive keyed completion; cancel immediately from reader EOF/stop. |
| Pinned `cua-perception` worker | Entire Rust ONNX detector/OCR pipeline, preprocessing, postprocessing, manifest/runtime checks and framed worker protocol. |
| Pi tool adapter | Recognize pinned `parse_visual_regions` as a metadata read; preserve capture marker and perform durable task scheduling. |

Retain the complete 46-file worker closure now copied at `vendor/cua/driver/crates/cua-perception`, from CUA revision `88aff7e5424e9ed45b93ded85eb4111472ca5190`: Cargo manifest, `src` modules, artifact metadata/notices, scripts and existing Rust tests/fixture corpus. Production executes Rust only. Retain pinned ORT/ort-sys `2.0.0-rc.10`; artifact acquisition scripts are build/provenance tooling, not production runtime dependencies.

Reuse the coherent process implementation from pinned `cua-driver-core/src/perception_client.rs` and `perception_client/{containment.rs,containment/linux.rs,containment/unix.rs}` behind the private backend. Preserve framed reads/writes, process-group ownership, parent-death handling, seccomp/Landlock denial, environment clearing and cleanup guards. Extract the pure region filtering and normalization from `perception_tools.rs` and contract `visual.rs` without its ToolRegistry, global artifact discovery, capture owner or scheduler. Keep upstream tests attached to retained helpers and add Dojo boundary cases.

Required host patches are explicit request cancellation/deadline instead of a default token; trusted installation instead of arbitrary executable/manifest paths; exact Linux loader closure instead of broad system trees; strict required echoes instead of accepting omitted screenshot/capture identity; Dojo captured source/transform instead of declaring all outputs primary; constant noninteractive/no-authorization results. Do not expose a raw worker runner. The worker protocol has no cancellation command: termination belongs to the host. If protocol changes become necessary, record them as patches to the pinned whole worker, with new binary identity.

## Lifecycle

1. The serialized service loop admits a read using its existing connection and session. The capture authority resolves the exact retained PNG and provenance without consuming it. Reserve retained-image and base64/frame memory, then acquire a worker slot without waiting; refusal is `Busy` or a typed resource error.
2. Move the read future off the global service loop. Its context contains connection/request cancellation and a server deadline; it does not carry agent tasks or durable ordering. Other clients, session cleanup and explicit actions retain their existing semantics. A cancellation event must reach this request directly from the connection reader, not await the next serialized dispatch.
3. Compute the effective budget from the smaller of server deadline and remaining capture lifetime, using the registry's own clock. Reserve time for launch, health, parse and kill/reap. Start one child with private working directory and stdio only. Enforce the pinned 16 MiB frame, 8 MiB image, 8192 dimension, 32 Mi-pixel and 128 MiB decode bounds before sending. Larger retained Dojo captures are refused, never silently resized.
4. Accept only the expected protocol/request ID, capture ID, PNG digest/dimensions and exact worker/model/runtime/manifest identities. Validate bounded finite boxes/confidences/text. Options filter output after inference; they do not promise less computation. Preserve fractional pixel centers and use the capture's stored affine mapping.
5. Close stdin, await exit, and reap. Cancellation/deadline kills the owned process group and awaits bounded cleanup; retain capacity and memory charges until ownership is settled. Drop is a fallback kill, not proof of reaping. A late reply cannot publish.
6. A completion event keyed by connection/session/request must reacquire the original capture authority and confirm the same private binding and registered capture remains live, unconsumed, unretired and unexpired, and the request is uncancelled. Removal, action consumption or expiry during inference suppresses publication. TTL is never renewed. Holding `Arc<[u8]>` alone cannot prove capture validity. This is a read of stored pixels; changed live geometry is relevant to later action admission, not an excuse to relabel the stored screenshot.

No result grants authority, prepares a browser, creates an element token, or admits a mutation. Durable approval design is deferred.

## Trusted package and loader

The verified artifact staging cache now contains real model/runtime bytes at `/home/nathan/.cache/dojo/artifacts/cua-perception/0.2.1/verified`. That mutable cache is evidence and build input, not a trusted production installation. `provenance/artifacts/cua-perception-release.lock.json` and `scripts/check-perception-artifacts.mjs` pin the release archive and exact six subjects. Use a Nix fixed-output derivation for the archive SHA256 `b4d76b626df9ce1a48b8036f7313412525295d0cb818d82ecbe3d9cca5fd1d08`, validate/extract only those subjects, and build the executable from the exact pinned baseline. Release executable/source commit differ and are not accepted as the worker baseline.

Create a package-owned immutable manifest, binary identity and exact transitive runtime dependency closure, including the dynamic interpreter and required libstdc++. Grant executable/interpreter execute rights and data/library read rights only for reviewed immutable members; no blanket `/nix/store` or ambient user paths. Clear ambient loader variables. Resolve dependencies through package-owned deterministic loader configuration or an explicit loader invocation. If changing the pinned ORT bytes is required, record a new reviewed digest rather than claiming the original subject. Preserve artifact licenses/notices, including the detector's AGPL obligations, in the package/provenance closure.

The real baseline Rust worker now parses the pinned 960×600 native-controls fixture; `/tmp/dojo-perception-real-inference.json` records actual text/boxes with `interactive:false`. It initially failed loading libstdc++, then succeeded with an exact development GCC library path. This proves model execution, not production loader packaging or session containment. No production Python is needed.

## Verifiable implementation units

1. Session read ticket and completion gate: same bytes twice without consumption; cross-session, retired, consumed, expired and disconnect refusals; expiry racing completion under injected registry clock.
2. Off-loop service completion: a pending fixture worker must not prevent another client read or session cleanup; direct disconnect cancellation prevents send/publication.
3. Cold protocol/ownership adapter with Rust fixture processes: malformed/missing echoes, excessive frames, hang/crash, cancellation during health/inference/exit, bounded kill/reap and capacity release; no sockets or inherited desktop authority.
4. Immutable package: verifier checks all six artifacts and baseline binary source; contained child loads real ORT with cleared environment and exact loader closure. Unknown or mutable path fails before launch.
5. Real known-answer corpus and live retained capture: validate text/boxes and original PNG identity, no duplicate image block or capture-marker clearing, no grants, and TTL/deadline completion. Existing fixture-only worker tests and source compilation are separately reported.

## Red-flag review

One capture owner and one deep read method; worker stages stay private. No caller-provided image, path, PID or authority; no parallel capture registry, warm process pool, hidden queue, runtime downloads, model interpreter, or inferred action permission. Cold startup cost is an accepted tradeoff, not an unmeasured performance claim.
