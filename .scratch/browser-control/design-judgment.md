# Browser Control design judgment

## Decision

Use Candidate A as the base because it is the only candidate that puts admission in the same Niri actor as human input and the actual extension transport write. Candidate B is simpler and has a cleaner extension tombstone story, but its EventStream delivery interval explicitly permits browser commands after takeover; it cannot satisfy the strict admission requirement.

Graft Candidate B's explicit generation retirement and outcome vocabulary onto A's transport gate. A lease retirement must be independent of execute serialization, tombstone the exact generation, reject queued work, and never wait for arbitrary `execute(code)` to finish. Preserve one owner for task intent/history (Pi), tab ownership (upstream registry), and native admission (Niri).

## Rubric

| Criterion | Candidate A | Candidate B | Judgment |
|---|---|---|---|
| Upstream reuse / smallest coherent boundary | Reuses driver and extension but adds a native broker, Niri writer, and substantial fork surface. | Reuses more of the upstream route; generation gate is a compact lifecycle seam. | A is the smallest shape that can meet strict admission. Keep its broker transport-only and avoid adding a parallel driver or task store. |
| Pi cancellation / agent and tab isolation | Early retirement and per-lease/channel/target generations; one physical in-flight command. | Out-of-semaphore cancellation and per-session generations; exact mapping optional. | A has stronger isolation at admission. Graft B's explicit session generation and retirement semantics; keep tab ownership solely in upstream. Neither can stop an already-started Chrome effect. |
| Enforceable Niri human priority / query-send race | Same-actor write gate removes query/send race for the transport admission point. | EventStream/tombstone is asynchronous, so a send can race takeover. | A wins only for ordering the extension command write. It does not prove that a Chrome/page effect begins before human input; describe the guarantee as transport admission and classify later effects in-flight/unknown. If issue 16 requires actual effect-start ordering, neither candidate meets it. |
| Nix / agent deployability | Identifies frozen package, stable extension path, managed commands, Pi runtime extension, and remaining human extension-load uncertainty. | Little concrete deployment plan. | A is the better base, contingent on building the pinned package and proving the real Helium extension path. Keep activation steps explicit; do not claim unattended deployment until verified. |
| Simplicity / reviewability | Large cross-process protocol and Niri change; detailed bounds, no second scheduler/history. | More compact but weakens required guarantee. | A carries unavoidable complexity for strict native ordering. Keep bounded envelopes, one in-flight command, no durable scheduler/approval/history, and no duplicate execute journal. Reject any design that adds a second task authority. |

## Red flags and required corrections

- **Overclaim at the admission boundary:** Niri's successful nonblocking socket write proves only that bytes entered the kernel transport in order with the actor's human-input processing. It does not prove when the extension invokes Chrome or when page effects start. A must not call that “effect start” or claim a human action cancels already admitted work. Keep the accepted in-flight/unknown limitation visible to Pi.
- **Trusted arbitrary code:** `execute(code)` may continue computing, install timers, or cause page work after retirement. Closing routes and rejecting subsequent commands cannot undo those actions. Retire promptly without waiting for that worker; require fresh observation and an explicit task decision before retrying unknown mutations.
- **Protocol correctness is a concentrated risk:** descriptor handoff, WebSocket control frames, command correlation, extension reconnect, and upstream `ExtensionRpc` adaptation need one narrow transport owner and fail-closed behavior. Do not retain an ordinary relay writer or an automatic retry/fallback path.
- **Identity and scope:** there is no established tab-to-Niri-window mapping. Use the conservative aggregate browser-client guard described in A; retire all relevant leases on human activity and fail closed when browser identity is uncertain. Do not infer target identity from title or PID alone.
- **Deployment is not yet demonstrated:** stable extension installation for the live Helium profile may still require a human load/restart. Prove the built Nix package, managed agent commands, and actual extension connection before advertising mutation support.

## Implementation risks / proof gates

1. Demonstrate a real pinned-extension command written by Niri's actor, with human takeover before and after the write; report queued, in-flight, and unknown outcomes accurately.
2. Prove there is no bypass writer, including automatic control frames, recovery, fallback, tab operations, recording, or child CDP routes. Any unconverted path must be disabled in Dojo mode.
3. Verify exact lease, channel, target, and command generations across reconnect and stale cleanup; Pi stop must not wait for the execute semaphore or conversation teardown.
4. Build and install the frozen source through Nix and verify the stable extension path and managed Pi/OpenCode commands on the actual browser profile.
5. Keep the initial contract narrow: human activity in any guarded browser client pauses all leases; already-admitted Chrome/page work may continue. No exact-window guarantee until that mapping is proven.
