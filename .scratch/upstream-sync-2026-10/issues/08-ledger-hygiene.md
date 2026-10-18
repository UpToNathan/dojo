# 08 — Keep the cua ledger green and fix stale provenance notes

Type: task
Status: ready-for-agent
Blocked by: 04
Effort: small

## Brief

`verify-ledger` was already failing before the October bump (21 Dojo-owned local records stale, such as
`Cargo.lock` and `src/lib.rs`) because nothing runs it. Run `refresh-ledger --write` after each such
change, and add the ledger and reference-pin checks to whatever pre-commit or CI hook the repo uses
for similar gates.

Also fix: `docs/licenses/preset-palettes.txt` cites omarchy `4c1b4cb3` while the research doc pins
`61263219` (both valid; make them agree or say why); the README dependency-license tables in
`vendor/cua/README.md` are stale (closure 1288 -> 1307 packages).

## Acceptance

`node vendor/cua/verify-ledger.mjs --source <pinned checkout>` and `node scripts/check-reference-pins.mjs`
pass at HEAD and are run by a hook.
