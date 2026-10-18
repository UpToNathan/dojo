# 04 — Finish and merge the cua bump, then follow its contract changes

Type: task
Status: claimed
Blocked by: none
Effort: medium

## Brief

Branch `cua-bump` (Rift workspace `~/.rifts/Dojo/cua-bump`) bumps the vendored import from
`88aff7e5` to `afed1fcb` and carries the AT-SPI fixes `e3ad8319a`, `7d8378f16`, `6e050d720`,
`e7af6a61b`. The ledger gate needs the per-record `upstream_revision` tooling and re-pinned reviewed
sources; the reviews are recorded in the ledger migration commit.

After merging: rebuild the driver with `scripts/build-cua-driver.sh` (it replaces
`~/.local/share/dojo/bin/cua-driver`; the installed `dojo-agent` must be rebuilt in the same step or
it refuses the binary), reinstall the helpers, and restart.

Follow-ups the bump surfaced:
- `pid` is no longer required when an `element_token` is given (`a85f1be9c`): check `dojo-agent`
  callers and the tool allowlist still match the contract pins.
- `type_text` now fails when a character cannot be typed (`2c3d2f412`) and returns a partial-result
  error: make sure `dojo-agent` surfaces it instead of reporting success.
- `check_permissions` accepts `prompt` (`1ff756bd7`).
- Parity rows stay unverified; do not mark any verified without evidence.

## Acceptance

Ledger, copy, parity and catalog gates green; `dojo-agent` and the driver pins match; a smoke test of
a desktop tool call through `dojo-voice` works on venus.
