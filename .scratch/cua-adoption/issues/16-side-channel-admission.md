# 16 — Enforce human-priority admission for CDP and AT-SPI mutations

Type: prototype
Status: needs-triage
Blocked by: 02
Effort: exploratory/large; safety release blocker

## Brief

Niri coordinated input rechecks ownership when delivering native events. A separate ownership query followed by a CDP or D-Bus mutation has a race and is not an equivalent guarantee. Prove a shared admission protocol before enabling semantic/browser mutations. Read-only 06/07 functionality can ship first.

Specify exact target/client identity, revision/generation, intent, grant, admission/in-flight state, revocation and outcome. Prototype ordering between external dispatch and immediate human takeover. Never defer human input to protect an agent operation. Decide how pending effects are canceled, how already-delivered effects are reported, and which external requests cannot be safely admitted. Do not promise rollback/cancellation of an operation already accepted by an application. Fail closed where the application's route cannot meet the chosen guarantee.

Use deterministic coordination plus controllable CDP/AT-SPI fixture adapters to pause at every check/dispatch/effect/result boundary. Then demonstrate the selected protocol with actual nested human input. Record guarantees and unavoidable in-flight semantics explicitly before implementing the production gate. Permission approval is not a coordination lease.

## Acceptance

- No mutation starts after a human takeover that precedes its enforceable admission point; cancellation/revocation prevents queued follow-on effects.
- Human input remains immediate; no lock or external timeout blocks human events.
- Already-admitted noncancelable operations produce explicit in-flight/unknown outcomes and require fresh observation; no automatic retry.
- Cross-window/client identity and PID/generation reuse cannot reuse admission.
- Every mutating CDP/AT-SPI route uses this gate, including set-value/navigation/download-triggering actions.
- Until proven, capabilities advertise read-only/refused mutation—not best-effort human safety.

## Comments

Review round 1 identified the ownership-query/side-effect race as a P1 gap. This ticket makes the shared admission proof a release blocker, rather than relying on a timing disclaimer.

2026-10-12 continuation: the fixed-recipe launch prototype, kernel-bound native peers and opt-in launcher child enrollment are implemented in source. Enrollment verifies the selected attester, child parent/generation, immutable recipe and compositor generation, transferred pidfd and private control endpoint. Final navigation dispatch checks that the enrolled child still owns its native peer and both process/control lifetimes remain live. This does not close acceptance: per-session ingress and cancellation, queued descriptor/source retirement, lazy service integration and the actual Chromium route remain required. No approval or uncertainty-clear semantics were added, and installed Niri was not changed.
