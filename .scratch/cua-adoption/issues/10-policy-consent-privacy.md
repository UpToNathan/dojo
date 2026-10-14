# 10 — Consolidate CUA-derived policy, consent, budgets and privacy

Type: task
Status: needs-triage
Blocked by: 01, 02, 03
Effort: medium/large

## Brief

Baseline authorization already ships in 02. Audit and reuse CUA policy/authorization/session grants/consent/health machinery and Python budget/operator/privacy callback behavior. Replace process globals and CUA paths with Dojo-owned state. Separate app/window/route permissions, browser-profile grants, clipboard/file access, destructive actions and human takeover. Enforce in Rust execution, not only prompts or model-visible tool lists.

Tie approval to exact intent/target, capability/session/generation, bounded lifetime and revocation. Model changes or schema changes cannot broaden an existing grant. Persist budgets with Pi run ownership; cancellable Rust operations obey time/size/step bounds. Privacy processing must happen before durable results, memory and recordings, not only telemetry export. CUA PII callback is a TODO/pass-through stub: implement filtering rather than claiming a ported redactor. Separate request-image projection, capture TTL, durable artifacts and memory-summary policies; derived request trimming never edits canonical entries. Test restart and zoom. Known sensitive scopes require denial/explicit consent; unknown visual PII cannot be guaranteed redacted. Treat model/desktop content as untrusted; never allow it to alter policy by instruction.

Add Pi-owned pending-human clarification/approval documents and authenticated UI responses, stable IDs, cancellation/expiry and deduplication across restart. Clarification never broadens grants; resumed mutations require fresh target observation. Reuse upstream human-tool request/response concepts, not its polling runtime.

## Acceptance

- Denial/expiry/revocation and cross-session tests across pointer, CDP, AT-SPI, clipboard and file routes.
- Consent cannot be forged by a tool argument; grants are visible and revocable in Dojo.
- Secret/password/clipboard/token values absent from default trace and memory fixtures.
- Limits survive restart and do not introduce an independent agent budget database.
- CUA policy features not adopted are explicitly documented rather than quietly omitted.

## Comments

### Approval discussion outcome

The user chose the existing Pi persistence for pending human requests, responses and execution results. Do not introduce another approval database or persistence layer. Persistent OS configuration and recovery remain owned by Nix, devenv, Home Manager and Btrfs. Approval UI must describe the intended real-world outcome, affected resources, consequences and recovery; source diffs are optional technical detail. Installed OS changes still require approval of the concrete proposal. Clarification and option selection do not implicitly authorize arbitrary execution. Approval scope and enforcement remain implementation work; the discussion did not establish a generic reusable grant.
