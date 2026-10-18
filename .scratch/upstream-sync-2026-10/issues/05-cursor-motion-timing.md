# 05 — Decide whether to adopt cua's new cursor timing defaults

Type: grilling
Status: needs-triage
Blocked by: none
Effort: small

## Brief

cua `a70d776b4` makes the agent cursor default to a "steady" timing with a 600 ms floor and adds
`relaxed`, `calm`, `deliberate`. Dojo's translated `vendor/cua/crates/cua-cursor-motion` has three
timings, is at 0.34.0 and has golden fixtures. Decide whether the floor suits Dojo's agent cursor
(Dojo draws it through Niri), then either port the change with new fixtures or record why not.

## Acceptance

A decision recorded here; if adopted, fixtures and the translated crate updated together.
