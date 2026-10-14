# 09 — Deterministic verification and bounded step execution

Type: task
Status: needs-triage
Blocked by: 02, 03
Effort: medium; semantic predicates depend on 06/07

## Brief

Reuse core `expectation.rs`, contract `verification.rs` and bounded batch machinery. Implement Dojo observation adapter, stable samples, deadlines, cancellation and true/false/unknown outcomes. Avoid re-entering public authorization recursively. Negative element predicates require complete trusted trees. Begin with window/geometry predicates; enable semantic/browser predicates after their observations ship.

Steps execute through the same action authority. Validate permission, target identity, capabilities, token freshness and coordination at each effect. Define explicit snapshot/capture consumption across steps rather than treating a multi-step script as one reusable screenshot grant. Stop on pause/refusal/staleness, report completed prefix plus ambiguous step, and never replay the batch after uncertain interruption.

## Acceptance

- Upstream predicate fixtures plus deterministic clock/observation sequences for stable/unstable/unknown states.
- Deadlines and hung providers bounded; no idle polling outside an active verify operation.
- Handled-but-no-UI-change is not verified success.
- Human takeover between steps stops subsequent effects and releases held input.
- Pi receives structured evidence and fresh-observation requirements, not a fabricated success boolean.

## Comments
