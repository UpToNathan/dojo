# 02 — Typed Rust computer module and durable Pi session spine

Type: task
Status: claimed
Blocked by: 01
Effort: medium

## Brief

Read current `src/bin/dojo-agent.rs`, service/screenshot modules, their callers, `src/agent_input.rs` and `tools/speech/pi-runtime.mjs`. Extract typed contracts and a deep Rust computer module. Keep MCP/direct socket adapters on one session authority. Broaden session ownership to captures/snapshots/refs without forcing screenshot/context calls to create input seats. Preserve no-service/no-seat observation behavior where feasible; explicitly specify any service-start change and bounded idle shutdown.

Import CUA contract/error/capability machinery where its dependency closure permits. Generate tool schemas from Rust types, retain versioned legacy translation, and define target identity, observation refs, outcomes and partial-effect receipts. Establish baseline authorization and privacy before screenshot release: deterministic secret/password-field/error filtering before Pi result/memory storage, explicit scoped capture consent, known-sensitive-scope denial, and honest visual-redaction limitations. Test durable results, memory mirrors/summaries and zoom restoration; CUA PII callback is a pass-through stub, not reusable redaction. Coordinate all future mutating routes, not just Niri pointer input.

Make Pi desktop registrations a named extension with explicit replay policies. Stop/abort/close must cancel/release the desktop session promptly. Add bounded MCP request deadlines and late-response handling. Persist recovery-required state for interrupted/unknown effects and require fresh observation before continued observation-dependent mutation. Respect prepared-request tool catalogs and old persisted tool schemas.

## Acceptance

- Contract round-trip and adapter conformance tests; independent sessions cannot impersonate one another.
- Unsafe desktop effects never rerun on Pi restart; safe reads return fresh metadata.
- Tests for stop during blocking action, lost MCP process, service crash and close with held input.
- Baseline structured receipts identify not-started/handled/paused/partial/unknown, without exactly-once claims.
- Existing tools continue working; no duplicate preferred catalog in Pi.

## Comments

Implementation evidence, 2026-10-12: `dojo-computer` supplies typed scopes,
observations and coordinate actions. One shared Rust service owns captures and
seats. Pi's named extension has sequential execution, explicit unsafe replay,
bounded MCP promises and durable fresh-observation recovery. Actual Pi-to-Rust
acceptance passes in a nested fixture with a scripted provider. Full baseline
privacy/authorization, complete typed receipts/contract parity and versioned
persisted-tool migrations remain incomplete. See [result](../result.md).
