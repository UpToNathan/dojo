# Cross-judge: browser admission candidates

Scores are 0–3 (0 = absent/contradictory, 1 = major gap, 2 = credible with material open work, 3 = strong and grounded). These assess design coverage, not implementation evidence. Both candidates are sketches; neither proves descriptor exclusivity, real Chromium topology, or runnable acceptance today.

| Criterion | Candidate A | Candidate B | Judge notes |
| --- | ---: | ---: | --- |
| Explicit unforgeable operator enrollment and fixed launch recipe | 1 | 3 | A requires explicit enrollment, but its trusted artifact identity is expressly unresolved and does not yet pin argv, environment, inherited FDs, or install closure. B makes the operator select a live attester generation and a fixed recipe covering those inputs. This changes the trust subject to a launcher, which must be made explicit to the operator. |
| Kernel process-generation/peer/session binding and queued-FD cancellation at final actor send | 2 | 3 | Both bind kernel credentials, process generation, peer/session, and order actor revocation against the final write. B is more concrete about `SCM_CREDENTIALS`, queued `SCM_RIGHTS`, ancillary-data cleanup, separate cancellation control, and serialized actor events. Neither can claim kernel proof that a writable FD has no clone. A's async `navigate`/`&mut self` shape needs care so waiting for broker/cancel acknowledgments cannot stall Niri's actor. |
| Live browser topology/page identity/exclusive writer ownership and no replay/unknown persistence | 2 | 3 | Both preserve exact page target, native/browser/listener identity distinctions, one transferred FD, no retry, and Pi-owned durable unknown state. B carries the unsupported split-topology refusal, inode/tuple validation, intent burn/capacity behavior, and limits of actor ordering more explicitly. Both correctly treat exclusive-writer/no-clone as a trusted broker obligation requiring route proof. |
| Smallest deep interface preserving lazy service/Pi scheduler ownership | 2 | 2 | A avoids a new process but moves child supervision and more lifecycle responsibility into Niri. B keeps lazy service and browser/session ownership where they are, but adds a separately trusted attester daemon and another protocol/lifecycle. Both preserve Pi as the only durable scheduler and avoid Python or a second durable store. Neither is a clear complexity winner. |
| Nonblocking Niri actor/resource cleanup with runnable red/green integration | 2 | 3 | A specifies actor-ordered removal of sources/FDs and a detailed inactive-route acceptance plan, but its async service interface risks accidental actor blocking. B's actor API is synchronous and its integration plan names a proposed executable acceptance target and concrete observable scenarios. That target does not yet exist, so this is runnable-plan quality, not a passing integration. |
| Feasibility without invented approval/recovery semantics or installed changes | 2 | 2 | Both leave approval and unknown settlement unresolved, retain Pi durability, and constrain work to source plus inactive `dojo-tests`. A leaves enrollment identity unresolved; B's additional daemon, pinned running launcher, and immutable Nix recipe increase implementation and deployment work. Neither authorizes installing or restarting Niri. |
| **Total** | **11/18** | **16/18** | |

## Verdict

Use **Candidate B as the base**. It closes the central trust gap more concretely: operator-selected live launcher generation plus a recipe that fixes executable, argv, environment, policy, dependencies, and inherited descriptors. It also gives the clearest account of the actor's enforceable cancellation ordering and of queued versus received FD cleanup. This is a design preference, not approval to deploy a new daemon or alter installed configuration.

### Graft from A

- Keep A's integration ownership map and its broad red/green scenario inventory, especially lazy activation idempotence, exact Chromium-to-MCP route evidence, and the rule that source tests/socket fixtures do not substitute for the real route.
- Keep A's concise statement of Niri's final-writer role and preserve existing service browser/session/controller ownership.

### Retain from B

- Fixed launch recipe and explicit trust-subject change; no path/hash-only or same-UID enrollment.
- Dedicated child slot per compositor generation; per-scope ingress and separate cancellation control; invalidate-before-close ordering, with acknowledgment only after actor cleanup.
- Explicit Linux credential/ancillary-data checks and the caveat that peer credentials on a socketpair identify its creator; authenticate child messages separately.
- Keep attempted or ambiguous writes unknown, preserve consumed intents/unresolved targets across registration replacement, and refuse capacity exhaustion rather than evicting replay protection.

### Reject or resolve before implementation

- Reject any interpretation of either proposal that treats an uncloneable/non-cloneable FD as a kernel-enforced guarantee. Make no-clone/no-buffered-write a broker implementation obligation and prove it in the real route.
- Do not copy A's unresolved artifact-only enrollment or direct enrollment phrasing into implementation. B's launcher is itself an expanded trust subject; require explicit operator enrollment and verify the complete fixed recipe against the current Nix/devenv/Home Manager model before selecting implementation details.
- Do not implement B's proposed attester binary until a source-only prototype proves its added process and fixed recipe are feasible without a durable store, Python runtime, host installation, or changed Pi ownership.
- Keep Niri's actor event loop nonblocking. Broker preparation and acknowledgment waits belong outside it; the actor handles bounded message receipt and one nonblocking final send. Make queued-FD teardown and accepted-send uncertainty observable in the inactive fixture.
- Neither proposal establishes a settlement authority or approval protocol. Add no recovery-clear API or inferred consent. Registration, fresh observation, and restart must not clear uncertainty.
- Validate real Chromium/native-window/browser-root/listener identities and the actual Nix launch recipe before claiming the current exact-PID topology works. Unsupported split identities remain fail-closed.

## Delivery judgment

The design work can proceed as source preparation and inactive nested fixture work. Do not install configuration, replace/restart Niri, or imply that either proposed acceptance runner has been executed. B is the stronger starting point, with implementation feasibility of its launcher boundary as the first gate.
