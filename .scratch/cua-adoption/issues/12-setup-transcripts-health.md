# 12 — Agent setup, transcript/icon reuse and health diagnostics

Type: task
Status: needs-triage
Blocked by: 01, 02
Effort: medium; independent sub-slices

## Brief

Adapt `cua-agent-setup` to Dojo commands/state/skill embedding, preserving JSONC/TOML edits, backups, detection and reversible writes. Respect existing project/user configurations and avoid changing global providers or credentials. All installation is explicit.

Import `cua-spaces-transcript` unchanged where possible, then integrate terminal/agent observations only where useful; Claude terminal-layout assumptions are not universal. Reuse `cua-icon-cache` with Dojo paths and actual Linux icon fetcher. Audit `cua-agents` event handling for useful extraction without spacesd becoming a run owner. Pi-native events remain the source for Dojo's own runs.

Reuse health/capability reporting so users can distinguish unavailable geometry, missing accessibility, unsupported client seat delivery, absent debugging endpoint and pending consent. Advertised tool/capability set reflects live supported behavior, not every upstream schema.

## Acceptance

- Config edits preserve comments/format, backup and reversal; sandbox-home tests cover existing content.
- Upstream transcript tests retained; malformed terminal streams bounded.
- Icon concurrent-fetch/cache-limit/missing-result tests and no idle fetch loops.
- Missing host services produce actionable capability/refusal results without side effects.
- No extra scheduler, silent config installation or sensitive transcript recording.

## Comments
