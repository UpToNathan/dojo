# 03 — Capture-bound observations and actions

Type: task
Status: claimed
Blocked by: 01, 02
Effort: medium

## Brief

Import tested `capture_registry.rs`, image helpers and coverage/mapping helpers. Upstream registry is inert foundation code, not existing wired production admission. Its PID/window-or-primary target needs a wrapper or patch for Dojo named outputs/crops, generation and process identity. Never invent missing PID/geometry.

Move authoritative capture publication/lookup/admission into the shared Rust session owner so MCP and direct clients agree. Screenshots return opaque capture ID, digest, timestamps, target identity, actual dimensions, mapping and coverage. New coordinate actions accept delivered-image pixels plus capture ID; Rust resolves mapping and revalidates identity, bounds, geometry, permission and ownership. Reject consumed/expired/stale/cross-session/cross-generation IDs. Define when a paused or refused attempt consumes its capture; uncertain effects never restore it. Preserve current screenshot budgets, pre/post geometry checks and no focus/clipboard mutation.

Single-use admission defaults to one observation-dependent action. Explicitly handle provider multi-action lists without quietly reusing one capture. Include fresh-state requirement and simple post-action observation support before richer verification exists.

## Acceptance

- Upstream tests plus crop/scale/multiple-output/rounding and adversarial ID tests.
- Live nested screenshot-to-click with no model-side coordinate arithmetic.
- Geometry change, human takeover, runtime restart and wrong session all fail closed.
- Context/screenshot do not create or renew input seats; capture-only service resources expire.
- Existing screenshot interface preserved by translation, with new preferred contract documented.

## Comments

Implementation evidence, 2026-10-12: server-owned captures and image-coordinate
move/click/scroll are implemented. Session/process/geometry/ownership and bounds
are validated; the updated Niri wrapper checks the mapped target at delivery.
Live Rust/Pi acceptance proves intended target clicks, cross-session and replay
refusal, geometry/human takeover invalidation, occlusion refusal and disconnect
cleanup. Full multi-output/transform/expiry/crash matrix and the remaining 02
privacy policy gate are still open. See [result](../result.md).
