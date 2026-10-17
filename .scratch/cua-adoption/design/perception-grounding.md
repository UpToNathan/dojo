# Capture-bound visual perception: current architecture

## Overview

Dojo already has the important capture-side primitive for safe, read-only visual analysis: a session-bound capture ID resolves to immutable PNG bytes, digest, dimensions, target, and expiry without consuming the capture. What is missing is a route that gives those bytes to a perception worker while preserving the capture session as the authority. The pinned CUA implementation supplies a concrete Rust worker, contract, process client, and Linux containment model, but it is not wired into Dojo and its production model/runtime artifacts are not present in the pinned checkout.

The intended operation is `parse_visual_regions(capture_id, options)`: analyze one existing capture and return regions in its delivered-image pixel space, plus provenance that maps those pixels to the captured frame's action coordinates. It creates no durable approval, grant, or mutation authority. Regions are explicitly noninteractive; parsing alone cannot authorize an action. The source exploration in the two grounding reports is descriptive and does not establish that production inference has run successfully on this machine.

## Owners and interfaces

- **Capture authority:** `CaptureAuthority` in [`crates/dojo-computer/src/lib.rs`](../../../crates/dojo-computer/src/lib.rs) owns the `CaptureService`; each `CaptureSession` owns a private `CaptureBinding`. `CaptureSession` can publish, resolve scope/frame, admit actions, retire captures, and decode model text. It currently has no public resolver that hands immutable image data to a caller outside `dojo-computer`.
- **Immutable capture store:** `CaptureService::read_for_perception` and `PerceptionCapture` in [`crates/dojo-cua-primitives/src/capture_registry.rs`](../../../crates/dojo-cua-primitives/src/capture_registry.rs) check the binding and return a non-consuming read. The PNG is held as `Arc<[u8]>`; the registry's TTL/eviction/retirement can invalidate later lookups but cannot revoke an `Arc` already held by a worker.
- **Service and MCP transport:** `Session` in [`src/bin/dojo-agent/service.rs`](../../../src/bin/dojo-agent/service.rs) owns the live capture session and serializes requests. `Message` and `DesktopAdapter` in [`src/bin/dojo-agent.rs`](../../../src/bin/dojo-agent.rs) define the Unix/MCP boundary. There is a `desktop_decode_model` lookup route, but no perception parse message or tool. `desktop_observe` returns the original PNG to Pi as image content.
- **Pi tool dispatch:** [`tools/speech/desktop-extension.mjs`](../../../tools/speech/desktop-extension.mjs) registers discovered MCP tools as sequential Pi tools. Replay safety comes from the explicit `desktopReplay` allowlist, not MCP annotations. A new metadata-only read needs an explicit safe classification. Its request cancellation must reach Rust and the worker; the current service's disconnect reader can set cancellation while the serialized request is executing.
- **Worker candidate:** at pinned upstream revision `88aff7e5424e9ed45b93ded85eb4111472ca5190`, `cua-driver-contract/src/visual.rs` defines the capture-bound region contract; `cua-driver-core/src/perception_tools.rs` implements capture lookup and normalization; `perception_client.rs` plus `perception_client/containment/` owns bounded process IPC and isolation; `cua-perception/src/{lib,manifest,runtime,preprocess,postprocess,main}.rs` implements protocol, artifact validation, ONNX inference, geometry conversion, and executable. These paths are relative to `~/.cache/dojo/upstream/cua/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/rust/`.

## Current integration flow

1. Pi discovers the `dojo-agent` MCP server, pins the discovered tool schema fingerprint, and calls `desktop_observe`. Rust validates scope, captures, verifies the frame did not change during capture, publishes PNG under the session's private binding, and returns capture metadata and the same image.
2. Publication validates PNG dimensions/limits, hashes the exact encoded bytes, stores them with monotonic expiry, and returns capture ID, digest, dimensions, frame, and coordinate scale/coverage metadata. The capture registry's perception read checks ID, target, expiry, and binding under its lock; it does not consume or renew the entry.
3. The closest existing downstream route is `desktop_decode_model`. It resolves capture identity through `CaptureSession`, builds an unchanged-image model view, and returns a read-only parse result. It is not OCR/icon inference. Action admission follows a separate path, checks prediction digest/parser identity and current frame/window/coordinates, and consumes via `admit_action`.
4. The upstream perception host follows the same capture-bound shape: resolve binding and ID, read retained PNG, send framed JSON to a contained worker, then validate the response against the host's capture provenance. The worker runs icon detection and OCR, returns regions in original image pixels with `interactive: false`, and echoes capture/digest/dimensions. Host normalization applies filters and maps pixel coordinates through capture provenance. Dojo has no equivalent route today.

```mermaid
flowchart LR
  Pi --> MCP[dojo-agent MCP]
  MCP --> Session[Session / CaptureSession]
  Session -->|observe and publish| Store[CaptureService + registry]
  Pi -->|capture_id + options (missing route)| MCP
  Session -. missing session-owned byte resolver .-> Worker[contained perception worker]
  Store -->|bound non-consuming read| Session
  Worker -->|regions in image pixels, noninteractive| Normalize[host provenance + validation]
  Normalize --> Pi
```

The dotted edge is the architectural seam. The underlying registry already has `read_for_perception` and the capture exposes `png_bytes()`, but the binding and authority map are private to `dojo-computer`. A safe adapter needs to resolve through the owning `CaptureSession` and transfer immutable bytes plus digest/dimensions/provenance; it must not accept arbitrary caller image/path or a caller-supplied binding. This describes the ownership boundary, not a selected API design.

## Inputs, outputs, and invariants

The public request is a retained `capture_id` and model-neutral filters such as kinds, minimum confidence, and maximum regions. The worker receives encoded image bytes and dimensions; it computes the digest over exact encoded bytes. Dojo's capture path supplies PNG. The output binds back to capture ID, digest and dimensions; regions use original delivered-image pixels, and host provenance carries the affine image-to-action coordinate mapping. Half-pixel centers are preserved. Worker resizing is internal and must be inverted before output.

The perception read is non-consuming, does not extend capture TTL or session/seat leases, and never grants action authority. A held `Arc` remains readable if the registry entry expires or is retired, so lookup expiry alone cannot stop already-started inference; the caller needs a deadline/cancellation policy and an explicit completion freshness rule. Action admission remains a distinct consuming capability. Neither regions nor model provenance create element tokens, browser references, or capture authority.

## Worker and deployment constraints

The worker has cold and warm lifecycles. Cold requests include launch, artifact validation, initialization, health, parse, and process exit in a 30-second default deadline; timeout/cancellation drops and kills/reaps the owned worker. Warm mode serializes parses and has idle shutdown. But `shutdown_now` cancellation epoch is observed by warm parsing only: cold parsing uses explicit token/deadline, so a session-end hook alone does not cancel active cold work. The current upstream host tool also creates a fresh default cancellation token rather than wiring the caller's request cancellation. A Dojo adapter must connect real request/session cancellation before claiming cold cancellation parity.

The worker is isolated from desktop capture/action authority. Linux containment includes Landlock, seccomp, resource/process limits, private writable temp space, and cleared environment. Its system loader allowlist omits `/nix/store`; a Nix-linked executable/runtime therefore needs exact dependency read and interpreter execute access or packaging that satisfies the same boundary. Broad store access changes the containment model.

Artifact validation hashes manifest-listed files, then reopens paths for loading; ancestor symlink checks and immutable verified handles are absent, so a path can change between hash and use. Manifest bytes are hashed for identity, but the constructor has no expected manifest hash. Adoption needs a trusted manifest source and immutable staging (or another reviewed design) to close this TOCTOU gap.

The artifact lock contains reviewed digests, including an 80.9 MB OmniParser detector, two OCR models, dictionary/config, and Linux x64 ONNX Runtime 1.26.0. Those lock entries do not prove installation: no model weights or ONNX runtime library were found in the pinned upstream checkout, and the OmniParser detector has no download URL and must be supplied as the exact reviewed conversion. Only Linux x64 has a reviewed runtime lock entry. Production inference is Rust; upstream assembly/export/verification/quality tooling uses Python, so a no-production-Python build still needs equivalent acquisition and verification in Rust or Nix. No production Python process or Pi-side scheduler is required or established by the worker design.

The real backend returns OCR text and generic icon-class boxes. All regions are noninteractive; it does not infer button meaning, groups, parent links, or reading order. Fixture mode accepts only a fixed 1x1 PNG and is not evidence of real inference. The self-test uses synthetic input; a known-answer real-model fixture and live screenshot are still required to establish production behavior.

## Where to continue

- Dojo capture ownership, publication, lookup, action admission, and retirement: [`crates/dojo-computer/src/lib.rs`](../../../crates/dojo-computer/src/lib.rs) and [`crates/dojo-cua-primitives/src/capture_registry.rs`](../../../crates/dojo-cua-primitives/src/capture_registry.rs).
- Service cancellation, connection lifecycle, and serialized dispatch: [`src/bin/dojo-agent/service.rs`](../../../src/bin/dojo-agent/service.rs); MCP schema/transport: [`src/bin/dojo-agent.rs`](../../../src/bin/dojo-agent.rs).
- Pi discovery, replay classification, and image retention: [`tools/speech/desktop-extension.mjs`](../../../tools/speech/desktop-extension.mjs), [`tools/speech/image-retention.mjs`](../../../tools/speech/image-retention.mjs), and [`tools/speech/computer-model-bridge.mjs`](../../../tools/speech/computer-model-bridge.mjs).
- Upstream candidate contract/tool/client/runtime/artifacts: `cua-driver-contract/src/visual.rs`, `cua-driver-core/src/perception_tools.rs`, `cua-driver-core/src/perception_client.rs`, `cua-perception/src/`, `cua-perception/scripts/artifacts.lock.json`, and `cua-perception/models/MODEL_LEDGER.md` at the pinned upstream path given above.

Source basis: `.scratch/cua-adoption/design/perception-ground-capture.md` and `perception-ground-worker.md`; source paths and symbols are cited inline. Those reports record source inspection only; they did not run model acquisition, worker launch, or inference.
