# 06 — CUA AT-SPI snapshots and token-bound semantic actions

Type: task
Status: needs-triage
Blocked by: 01, 02, 03
Mutation enablement blocked by: 16
Effort: large

## Brief

Import a coherent AT-SPI implementation from `platform-linux/src/atspi/` and core snapshot/query/token/walk-budget machinery. Replace CUA platform discovery, popup/window matching and global session assumptions with Dojo identity/session adapters. Exact geometry from 04 gates pixel mapping, but read-only tree work can begin earlier.

Provide bounded semantic observations with snapshot ID, scoped element tokens, completeness/truncation, role/state/actions/rectangles and privacy filtering. Bind accessible objects to exact Niri windows using evidence, not title-only matching. Resolve/invoke live objects against identity and token freshness; every mutation passes Dojo policy and ticket 16's proved shared admission. Until that gate passes, ship read-only/refused mutation. `set_value` is not physical typing. Ambiguity, unavailable accessibility or denied permission is an explicit refusal/unknown; no silent pixel/global-pointer fallback.

## Acceptance

- Fixture application tests and upstream logic tests for changed object paths, duplicate names, disabled/password controls and incomplete trees.
- Cross-session/stale tokens and process reuse rejected; D-Bus hangs bounded and cancellable.
- Negative existence claims require complete trusted observations.
- Human ownership blocks semantic effects as well as pointer input; document atomicity limitations.
- Scope and cleanup verified with multiple windows/agents and desktop session teardown.

## Comments
