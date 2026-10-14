# Maintained CUA imports and Rust adaptations

The [whole-source copy and fork record](copy-first.md) describes the full Driver and sandbox SDK imports, the thin Pi resource connection, and their executable checks. Source integrity and runtime parity remain separate gates.

Dojo imports CUA source at revision `88aff7e5424e9ed45b93ded85eb4111472ca5190` and translates useful MIT behavior into Rust. Pi owns scheduling, canonical conversations and credentials. Imported modules, successful builds and offline fixtures each establish a limited fact; the capability gates below describe runtime availability.

## Reproduce the audit

Run from the Dojo root:

```sh
node vendor/cua/verify-ledger.mjs
node --test vendor/cua/ledger-tools.test.mjs vendor/cua/verify-import.test.mjs
node vendor/cua/verify-ledger.mjs --source /tmp/cua-portability-research
devenv shell -- node vendor/cua/audit-dependencies.mjs --write
node vendor/cua/refresh-ledger.mjs --source /tmp/cua-portability-research --write
node vendor/cua/verify-ledger.mjs --source /tmp/cua-portability-research
```

The upstream checkout is an audit input. Normal builds never depend on that temporary path. Acquire a pinned checkout explicitly when needed:

```sh
git clone --no-checkout https://github.com/trycua/cua /tmp/cua-import-source
git -C /tmp/cua-import-source checkout --detach 88aff7e5424e9ed45b93ded85eb4111472ca5190
node vendor/cua/verify-ledger.mjs --source /tmp/cua-import-source
```

`refresh-ledger.mjs` defaults to a read-only report. `--write` updates hashes only for owned local translations/glue and already declared source modifications. Every existing `copied` entry must still match its recorded hash and the pinned git object. A changed copied file aborts before any write. Review and explicitly declare such a modification before refreshing. Source objects come from `git show REV:path`, so dirty upstream working trees cannot redefine the baseline.

The refresh tool generates canonical unified diffs under `ledger-patches/`. `verify-ledger.mjs --source` independently regenerates each diff, applies it to the original source with `git apply` in a disposable directory and compares the resulting bytes with maintained source. A forged diff fails even if its checksum was updated. The verifier also checks translated source objects and all 72 inventoried upstream package manifests and governing notice hashes.

All 14 refresh/verifier tests pass. They demonstrate copied-byte refusal, dirty-source isolation, idempotence, exact patch reproduction, forged-patch detection and local-byte mismatch detection. No installed files, user profiles, live desktop or runtime state are changed by these tools.

## Audit files

| File | What it proves |
| --- | --- |
| `ledger.json` | Per-file integrity, immutable copies, explicit modifications, translated source objects, owners and scoped runtime evidence |
| `ledger-patches/` | Exact changes from pinned source to declared adapted files |
| `inventory.json` | 72 upstream Cargo package declarations and current adopted/translated slice status; inventories do not authorize a product or artifact |
| `dependency-closure.json` | Locked Linux Cargo metadata for 15 manifests, resolved features/graphs, separate normal/build membership, license declarations and available notice/copyright evidence |
| Unit `provenance.json`, archived sources and README files | Behavior correspondence, retained tests, local changes and remaining integration gates |

The ledger's current `files.length` is the maintained-file count. Older references to 84 or 298 files describe earlier snapshots. Run the verifier to obtain the current count rather than reusing those historical numbers. Local glue hashes establish integrity and do not claim upstream byte parity. Source and dependency manifests exclude generated targets and registry caches.

Dependency audit IDs use repository-relative labels for local packages. Cargo.lock hashes pin the actual resolutions. Upstream SDK/Driver lock snapshots under `provenance/` remain evidence of upstream metadata; they are not the active extracted locks.

## Adopted slices and capability gates

| Slice | Current evidence | Remaining scope |
| --- | --- | --- |
| CUA primitives/capture authority | Unchanged logic plus Dojo one-use session captures | Complete authorization/privacy and rollout matrices |
| Cursor motion/rich input | Niri consumes CUA easing under exact-window authority; nested text/hold/drag/cancellation evidence | Full platform/client parity and installed deployment |
| Native capture and same-render geometry | Named-output Rust capture and Niri capture-correlated scene evidence | Complete output/transform/hotplug matrix |
| Accessibility | Bounded read-only AT-SPI observations with sensitive-value filtering | Exact top-level correlation; mutations still refused |
| Private core/contract/browser | Whole minimal-feature build; owned reads; operator-opt-in guarded navigation adapter | Browser navigation defaults disabled; real browser route and generalized CDP/AT-SPI mutation acceptance unverified |
| Model behavior | Rust action/native-block parsing, preprocessing plans, grounding/budget/retention and S1 pure policy; thin supported Pi bridges | Native wire/subscription endpoint equivalence and tensor inference remain gated |
| Recording/evaluation | 23 Rust tests, canonical Pi usage/trace export, isolated CLI tasks, ATIF, GUI-R1/Aguvis and deterministic grounding metrics | Live recording hook coverage, external dataset artifacts/application evaluators and model baselines |
| Transcript | 21 unchanged conformance tests; bounded offline observation adapter | General terminal/application layout parity |
| Setup/icon cache | Adapted config plan/apply and reversible fixtures; bounded Linux cache | Explicit installation and separate live launcher/UI evidence |
| External admission | Native actor/writer and authenticated-broker fixtures; explicit navigation adapter | No general semantic mutation admission claim or enabled existing-profile effects |

The latest nested capture/input evidence is `/tmp/dojo-capture-live.ghgPdw` with all 19 gates passing. Its `result.json` records passing capture/input gates, including text, bounded hold, in-window drag, fresh execute-observe, same-render bounds verification and takeover/geometry cancellation. `pi-result.json` proves actual Pi/MCP tool calls, retained images and blocked replay. This uses a disposable nested compositor and scripted model. It does not prove a real provider, installed runtime or every adoption ticket. Separate real owned-browser Helium/MCP acceptance is `/tmp/dojo-owned-browser-live.dm9VS1`; generalized browser effects remain gated.

## Dependency license evidence

The executable audit currently resolves 928 distinct package/version/source identities, including 894 registry/git packages. Root normal/build membership includes 552 packages, while root Linux metadata contains 622 packages. Niri normal/build membership includes 282 packages, while its metadata graph contains 417 packages. Metadata-only dev/workspace packages are not described as active root build dependencies.

| Scope | Normal/build packages | Wider metadata packages | Notice files not found in normal/build |
| --- | --- | --- | --- |
| Dojo root | 552 | 622 | 40, including 38 registry/git packages |
| Niri compositor | 282 | 417 | 14 registry/git packages |
| Union of 15 maintained manifests | 799 | 928 | 52, including 50 registry/git packages |

Fifteen additional notice exceptions occur only in the wider metadata scope. Package identities can appear in several scopes, so the rows are not additive. Exact exceptions and available source-copyright evidence remain in the JSON audit.

No registry/git package lacks a declared license expression. The actual unknown declaration is local `dojo@0.1.0`. The audit records 67 package identities without a local license/notice file found, including 40 in the root normal/build closure. It also records any copyright/SPDX lines found in supplied README/source files. A declared expression or a source copyright line does not replace a complete notice file. Exact names, versions, source labels, build membership and evidence hashes are in `third_party_audit` in `dependency-closure.json`.

MPL, GPL, Unlicense and license alternatives needing distribution/use review are listed explicitly. GPL Niri packages are distinguished from MIT CUA slices; upstream FSL products remain excluded from adopted core source. No compatibility, notice-assembly, competing-use, distribution, model-weight or dataset decision is approved by these audits.

The metadata/audit commands use `--locked --offline` after dependencies have been provisioned. An uncached pinned source can be acquired explicitly with locked Cargo metadata or fetch; this audit fetched only the missing pinned Niri development packages. A fresh disconnected machine is not promised a build without the existing registry/git source cache. No generated registry cache is tracked.

## Build layout and licenses

Standalone imports and adaptation crates retain their own manifests/locks. Empty nested workspaces keep the extracted packages independent; they can still be root path dependencies. Do not add them as root workspace members without updating the intended layout and lock evidence.

The root uses Dojo adapters and the private minimal-feature CUA core. Imported ToolRegistry, scheduler, ambient lifecycle, provider credentials and legacy mutation APIs remain uninstantiated by these adapters. Browser and AT-SPI effects require explicit Dojo authority; no fallback to uncontrolled primary input is introduced.

MIT notices remain with the imported/adapted packages and under `licenses/`. Benchmark translations preserve both the Cua AI and TryCua MIT notices. Archived Linux platform sources and helper provenance remain with the private driver slice. Upstream Driver third-party notices are retained as source evidence, not as a claim that every listed component participates in this closure.

FSL Spaces apps/servers, Keyvault, Teleport and media products require their own product/use/distribution decision. Detector runtimes, model weights and dataset artifacts have independent terms. No such artifact is downloaded or licensed merely by recording its inventory entry.
