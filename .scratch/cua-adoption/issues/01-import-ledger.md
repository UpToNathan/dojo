# 01 — Establish pinned CUA import ledger and reproducible extraction

Type: task
Status: claimed
Blocked by: none
Effort: small for proven modules; medium for broader audit

## Brief

Follow [the adoption spec](../spec.md). Pin CUA `88aff7e5424e9ed45b93ded85eb4111472ca5190`. Decide bounded `vendor/cua` layout and record source paths, licenses/notices, copied/modified/translated state, dependency closure, hashes, tests and patch ownership in a machine-readable ledger. Avoid importing an entire upstream workspace merely to inherit its manifest. Reproduce the motion, transcript and five Driver module extractions from the prior report. Keep downloads and generated builds outside tracked source; normal builds must work offline from maintained inputs.

Inventory remaining core/contract/browser/AT-SPI/verification/recording dependencies. Mark unbuilt extractions honestly. Record FSL/AGPL/model/dataset candidates separately; technical inventory may proceed, but incorporation requiring a use/distribution decision is blocked until that decision is explicit. Audit licenses per package, not just root MIT text. Preserve translated-code attribution too.

## Acceptance

- Reproducible pinned source acquisition and byte checks; upstream extraction tests pass through devenv.
- License notices retained, lockfiles recorded, imported code distinguishable from local integration.
- Build order/dependency graph for larger imports; workspace conflicts and MSRV verified against Dojo.
- No runtime, installed desktop, global config or active user state changed.
- Ledger includes non-adopted slices and reasons, allowing later maximum-reuse work.

## Comments

Implementation evidence, 2026-10-12: bounded packages, machine-readable ledger,
license notices, byte checks, offline extraction tests and package inventory are
in `vendor/cua`. Root integration now builds with image 0.25.10 / zune-core 0.5.1;
standalone imports also pass. Full transitive artifact/notices audit and larger
imports remain incomplete. See [result](../result.md).
