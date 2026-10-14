# Adoption plan review — round 1

Original brief: maximize CUA reuse, translate useful Python behavior to Rust, update weaker Dojo contracts, preserve Rust desktop execution and pi-durable run ownership. Review was source/document inspection, not execution proof. A parallel Python source inventory was incorporated in [python-port-inventory.md](python-port-inventory.md).

## Findings and responses

1. **P1: best-effort coordination query does not enforce human priority for CDP/AT-SPI effects.** Accepted. Added [16](issues/16-side-channel-admission.md) as mutation enablement gate for 06/07 and final parity dependency. Read-only reuse can proceed; refused mutation is the fallback. Exact admission/revocation/in-flight semantics remain prototype work, not a claimed guarantee.
2. **P1: upstream PII callback is a stub and first-release privacy is vague.** Accepted and verified in source. Spec/02/10 now require baseline deterministic filtering and explicit capture scope/consent/storage behavior before first release, with tests across Pi results, memory, summaries and zoom. No blanket visual-redaction claim. Stub classified as reference only.
3. **P1: textual model formats also need Pi intent integration.** Accepted. 08/inventory specify provider adaptation before Pi answer classification, no effect from response/watch hooks, persisted parser/output/observation metadata and crash tests. Ordinary function profile ships first; every later format has its own bridge gate.
4. **P2: processed model image needs a capture-bound transform.** Accepted. Added versioned model-view record/digest/preprocessing/dimensions/transform and differential edge cases. Source inventory distinguishes actual UI-TARS preprocessing from separately defined smart-resize helpers; do not assume every model uses the same snapping rule.
5. **P2: request retention may corrupt durable memory/transcript.** Accepted. Separate request projection, capture TTL, durable artifacts and memory-summary policy; canonical entries remain intact absent explicit deletion policy. Pairing/signatures/restart/zoom covered.
6. **P2: ask-human has no durable response path.** Accepted. Inventoried upstream human queue/adapter; specified Pi-owned pending-human state, authenticated UI response, IDs/expiry/cancellation/deduplication, separation from permission grants and fresh observation on resume. Do not import polling runtime.

## Unresolved implementation gates

These findings are addressed in the roadmap, not implemented. Shared side-channel admission, reliable scoped privacy behavior, model-provider round-trips and human-response UI require their ticket evidence. Native endpoint support and Rust tensor backend parity remain unverified. No second review round was performed and no integrated safety/provider compatibility claim is made.
