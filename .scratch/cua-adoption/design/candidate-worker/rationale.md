# Contained process worker

## Problem

Dojo owns retained capture identity and consuming action admission. CUA supplies a complete Rust ONNX worker and contained process client, but its default host cancellation, mutable artifact paths, and Linux loader allowlist do not satisfy the current Dojo/Nix boundary. The service also processes requests synchronously, so inline inference would delay every client. This design adds a bounded image read under the existing session owner and runs one isolated child for that read. Durable approvals and dependent mutations remain outside this design.

## Usage

[Caller usage](usage.md) is the contract. `CaptureSession::parse_visual_regions` accepts capture ID, model-neutral options, and server-owned read context. Host setup provides the immutable backend. Callers never resolve PNGs, coordinate worker stages, manage artifacts, or create grants.

## Shape

[Type sketch](sketch.rs) defines the ownership. A private `ReadImageTicket` binds immutable bytes, digest, dimensions, captured transform, binding, expiry, cancellation, and held-byte accounting. The worker backend hides containment, framing, model loading, exact artifact/response checks, and child cleanup behind one session method, per boundary-discipline. Results are noninteractive pixel regions and provenance. Their types cannot encode action authorization.

The initial mode is cold: one health handshake, one parse, bounded exit, then reap. Workers share no process, model cache, writable directory, ORT globals, or mutable session state. A host capacity gate returns Busy rather than create another work queue, per separate-before-serializing-shared-state. Pi continues to choose and durably order operations.

The service dispatches the owned future off its serialized loop and handles completion by connection/request identity. Cancellation comes directly from the independent connection reader. Completion checks the live session and exact unconsumed capture under authority before publishing. TTL is never renewed. Details, source closure, and verifiable units are in [integration notes](integration.md).

## Synthesis decision

Pending comparison with the other architecture candidate. This candidate chooses a contained process because cancellation can terminate native inference and because ORT/model faults stay outside the desktop process.

## Tradeoffs accepted

- We accept cold model initialization for deterministic lifecycle and separate native runtime state.
- We accept explicit Busy/capture-expired refusals instead of background queues, retries, or TTL extension.
- We accept IPC encoding and contained-child memory for isolation, with explicit held-byte and frame budgets.
- We accept exact Nix closure packaging work instead of broad `/nix/store` access.

## Alternatives considered

In-process ORT hides IPC but exposes the desktop process to native faults and cannot forcibly stop an accepted inference call. Its otherwise small interface does not hide the required shutdown guarantees.

A reusable warm worker also offers a deep interface, but adds idle shutdown, shared cancellation epochs, model generations, and reuse policy. It is deferred until cold real-model behavior and measured costs justify that complexity.

A stock CUA host/tool import adds its capture binding, globals, tool lifecycle, and cancellation defaults. Its caller must reconcile two authorities, so it loses interface depth.

## Open questions and risks

The exact baseline worker now completes real ONNX inference with the verified model/runtime files when the development loader receives the exact GCC library path. Production must resolve those dependencies from its immutable closure without ambient configuration. Can real inference plus startup/reap finish within the remaining capture TTL on this hardware? Which worker resource ceilings satisfy actual model loading while respecting the host budget? These questions require executable evidence rather than this sketch.

## Next implementation step

Build the session-bound non-consuming read/completion gate and cancellable cold-process adapter against deterministic Rust protocol/containment fixtures, before exposing a production inference capability.
