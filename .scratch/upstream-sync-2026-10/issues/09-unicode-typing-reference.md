# 09 — Unicode typing for the agent seat (reference only)

Type: research
Status: needs-triage
Blocked by: none
Effort: medium

## Brief

cua fixed Unicode typing on X11/virtual-keyboard paths (`f7e177fab`, `8be7052a0`, `7e6db51b0`,
`6d82b3b1b`, `2c3d2f412`): each missing character gets its own spare keycode, long text is batched,
and the driver waits for the client before restoring the keymap. Dojo's `type_text` is ASCII-only and
goes through its own Niri seat. When Unicode typing is wanted, use these commits as the reference
rather than redesigning it; Niri's Smithay virtual keyboard rework (issue 01) is the natural moment.

## Acceptance

Decision to adopt or defer; if adopted, tests with non-ASCII input through the agent seat.
