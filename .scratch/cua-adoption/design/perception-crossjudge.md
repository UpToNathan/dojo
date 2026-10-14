# Perception architecture cross-judge

## Decision

Use **candidate-worker** as the base. It fits the actual Rust worker already copied and built, keeps native ONNX execution outside the long-lived desktop process, and makes child termination/reaping an enforceable request boundary. Its larger integration surface is real work, but it aligns with the current serialized Dojo service and has an implementable cancellation mechanism.

Candidate-native has a simpler public call and a good capture-owner shape. Its critical guarantee—interrupt every detector/OCR call and join before returning on deadline—remains an unproven property of the pinned ONNX Runtime. Without that, Tokio timeout would only hide a still-running inference task and release caller control while CPU/memory stay occupied. Reject it as the base unless a focused runtime spike proves cancellation and join within the service deadline.

## Scores

Scores are 1 (weak) through 5 (strong). These judge the designs as written, accounting for the current evidence: exact model/runtime artifacts have been acquired and match their baseline hashes, and the copied baseline Rust worker has successfully run real inference. The run is recorded at `/tmp/dojo-perception-real-inference.json`; it required an exact GCC development `libstdc++` loader path. That proves inference, not the final immutable Nix loader closure. Neither candidate is integrated into the service.

| Criterion | Native | Worker | Reason |
|---|---:|---:|---|
| Capture authority and freshness | 4 | 5 | Both keep the session as capture owner and require final freshness checks. Worker makes the private ticket carry binding, expiry, provenance, cancellation, and held-byte reservation across the child boundary, then gates publication against live session/capture state. Native describes a final expiry check, but is less explicit about retirement/consumption races. |
| Cancellation and cleanup bounds | 1 | 4 | Native explicitly depends on `RunOptions` interrupting every ORT call and joining; this has not been proven. Worker can kill/reap its owned process group and retain resource charges until cleanup settles. The actual service cancellation/completion wiring is still design work, so this is not a 5. |
| Interface depth / misuse resistance | 4 | 5 | Native offers a narrow session method with bounded options and no public ONNX types. Worker gives callers the same narrow operation while also making request context opaque and preventing callers from supplying paths, bytes, bindings, PIDs, or worker stages. It clearly separates capture ownership from process ownership. |
| Immutable Nix artifact reproducibility | 3 | 4 | Native proposes a generated identity and read-only exact Nix closure, but has no concrete closure construction and remains gated on ORT cancellation. Worker names a fixed-output archive, six exact subjects, baseline build, immutable manifest, and exact dependency rights. The successful real run still needed a development `libstdc++` path, so production loader closure and runtime dependencies remain to be packaged and verified. |
| Feasibility with actual Rust worker | 2 | 5 | Native would need to port/rework the worker's preprocessing/postprocessing/runtime into an in-process backend and resolve its cancellation contract. Worker retains the complete Rust executable/client/containment sources; the exact artifacts now exist and the baseline has passed real inference. Session service integration and packaging remain, but the inference path is demonstrated. |

## Keep and graft

- Keep the worker candidate's cold one-process-per-request lifecycle, strict bounded protocol, private work directory, process-group kill/reap, and permit/accounting held through cleanup. Do not add a warm pool or background queue before cold integration and measurements establish a need.
- Keep its `ReadImageTicket` / completion-gate ownership model: session-derived binding, exact retained bytes and digest, capture transform/expiry, cancellation/deadline, and memory reservation travel together. Recheck live session, binding, and unconsumed/unexpired capture before returning any result; an `Arc` surviving registry expiry is not evidence of continued authority.
- Graft the native candidate's especially clear caller-facing contract: one typed `CaptureSession::parse_visual_regions` call, model-neutral bounded filters, noninteractive result types, and no native runtime types exposed to Pi or service callers. The worker candidate already follows this direction; retain it as a boundary rule, not a second backend.
- Keep the common coordinate/provenance rules: source-image pixel boxes, inverse internal resize geometry, captured image-to-logical mapping, strict capture/digest/dimension identity, and no action authority. Parsing must not create grants, element tokens, or mutations; durable approval discussion remains deferred.
- Keep the no-production-Python constraint. Python may remain in build/provenance tooling only if needed; the deployed parser is the Rust worker, and Pi remains the durable task scheduler.

## Reject or defer

- Reject native inference as the production path for now: sharing ONNX with the service reduces isolation and cannot provide a hard timeout until call termination and join are demonstrated on the pinned runtime and Nix build.
- Reject a detached blocking task with a caller-side timeout. It can outlive capture expiry, cancellation, or the memory reservation while still consuming resources.
- Reject a stock CUA tool import that brings its own capture binding, scheduler, global artifact discovery, or default cancellation token. Adapt its worker/client/containment implementation behind Dojo's capture authority instead.
- Defer warm workers, cache reuse, retries, automatic replacement captures, and any change to TTL. They add shared lifecycle state and are unnecessary to establish the first bounded read path.
- Do not treat current model availability or `/tmp/dojo-perception-real-inference.json` as deployment proof. Artifact hashes and real inference are now established; exact immutable Nix loader dependencies, especially the required `libstdc++`, still need package-level validation.

## Remaining integration proof

The worker design's service loop plan is important: inference must move off the globally serialized request loop, with disconnect/stop cancellation delivered directly from the connection reader. Completion is routed back by connection/request identity. Prove that a slow worker does not block another session or cleanup; cancellation during health, inference, and exit always kills and reaps within the reserved cleanup budget; expiry/retirement/consumption races suppress late results; and memory/process permits are released only after child ownership settles. Then verify the contained child loads the real pinned models/runtime with cleared ambient loader variables and only its exact immutable Nix dependency closure.

Sources: [`candidate-native/embedded-inference.md`](candidate-native/embedded-inference.md); [`candidate-worker/rationale.md`](candidate-worker/rationale.md), [`integration.md`](candidate-worker/integration.md), [`sketch.rs`](candidate-worker/sketch.rs), and [`usage.md`](candidate-worker/usage.md); architecture baseline [`perception-grounding.md`](perception-grounding.md). The current run and artifact facts are supplied by the parent task; worker/service integration remains unimplemented.
