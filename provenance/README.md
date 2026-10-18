# Pinned references and parity

The CUA baseline is commit `afed1fcbba64acadcc154b8d5d377549ae62bc7e`.
Full parity covers its Driver and Spaces contracts, model workflows, evaluation,
inference, sandbox, hosted products and platform adapters. The source index is
in [.scratch/cua-adoption/full-parity-reference-research.md](../.scratch/cua-adoption/full-parity-reference-research.md).

## Resolution authorities

- Cargo resolves from the corresponding checked-in `Cargo.lock`. Build with
  `cargo check --locked`, `cargo build --locked` and `cargo test --locked`.
  Root registry declarations also use exact versions.
- Nix inputs declare commit SHAs in `devenv.yaml`; `devenv.lock` supplies their
  content hashes and transitive resolutions. The implicit devenv module is now
  an explicit input so entering a shell cannot update it silently.
- Node uses the exact package declarations and `package-lock.json` under
  `tools/speech`. Install with `npm ci` in that directory.
- Other imported package managers retain their source lockfiles. The reference
  inventory includes Zig dependency manifests with content identities.
- The five pre-existing Git subtrees retain exact upstream commits recovered
  from their subtree import history. CUA imports retain their separate source
  ledger and reproducible patches under `vendor/cua`.
- `artifacts/` contains unchanged upstream S1 weight and perception artifact
  locks. `contracts/` contains unchanged Driver 0.8.0 and Spaces 0.7.0 manifests.
  `artifacts/sources.json` records exact source blobs and SHA-256 digests for
  these copies. No model weights are downloaded by these checks.

## Checks

```sh
node --test scripts/reference-pins.test.mjs
node scripts/check-reference-pins.mjs
node scripts/check-reference-pins.mjs --strict
node scripts/check-cua-parity.mjs
node scripts/check-cua-parity.mjs --strict
```

The ordinary reference check detects stale inventories, changed artifact locks
and moving Nix declarations. The strict check also fails on unresolved URLs.
It currently fails. Thousands of links in imported documentation still need
classification and exact source or snapshot identities. A fixed content hash
can pin downloaded bytes even when the origin URL moves; an origin URL alone
cannot. Marketing model aliases and hosted-service versions cannot establish
immutable remote implementations.

The parity check preserves all 115 tools in the pinned contracts. Every row
starts unverified. Existing Dojo integrations need evidence against those exact
interfaces before a row becomes verified. The 23 broader product families in
the research report also remain acceptance requirements. A capability refusal
does not close a row when upstream implements that operation.

After reviewing a pin change, refresh the reference inventory with
`node scripts/check-reference-pins.mjs --write`. This command records the
working files; it does not resolve or approve upstream version changes.
Create the initial parity index with `node scripts/check-cua-parity.mjs --write`.
Later contract changes require an explicit baseline migration; the checker
refuses to silently replace tool identities or existing evidence.

## Zed (GPUI) upstream picks

`vendor/zed` is based on zed-industries/zed `9ab0715969` (the squashed subtree commit). The
`cc94985f` revision in `vendor/zed/ACCESSKIT-ANNOTATION-PROVENANCE.md` was a local checkout
commit that does not exist upstream; `9ab0715969` is the real base. Selected upstream
commits to `crates/gpui`, `gpui_wgpu` and `gpui_platform` are applied on top as patches, not
by merging the whole repository (checked against upstream `56934ec056`, 2026-10-17):

- `679ad7c304` uniform_list layout far down long lists
- `e18de65b75` source-over alpha for translucent layers (the `gpui_wgpu` part)
- `402675e805` wgpu 30 from git `3b3506ea`, fixing Vulkan surface teardown (monitor unplug)
- `e463c5ab74` focus cleanup stalls (the `gpui` and `gpui_platform` parts)
- `7699cb8bb8` touch fling axis

Skipped: hot reload, the Apple, iOS and web backends, and `4f6d97b9de` (frame pacing, which is
profiler and telemetry churn across crates Dojo does not use).

## Follow-up discussion

After the parity and pinning pass, discuss durable execution approvals and
notification management. CUA ticket 10 owns permission persistence, authenticated
operator responses, expiry, revocation and restart behavior. Clarification is
separate from permission. Notifications need to deliver those requests without
creating another approval or conversation database.
