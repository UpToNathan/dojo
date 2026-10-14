# 17 — Pin every dependency, source and behavior reference

Type: task
Status: claimed
Blocked by: 01
Effort: large

## Brief

The user expanded the target to full parity against immutable references.
Inventory every referenced dependency, copied or forked source, model,
dataset, artifact, protocol and behavior source. Preserve existing resolved
versions. Pin upstream commits and downloaded content separately.

Use existing package locks as resolution authorities. New sources need
provenance when first introduced. Mutable documentation needs an immutable
source blob or a recoverable snapshot. Hosted model aliases cannot pin a
provider's deployment; record this limit and the tested wire contract.

## Acceptance

- Every consumed external input has an immutable identity and reproducible
  resolution or download command.
- Forks record their upstream base and local modifications.
- A moving declaration, changed artifact, missing lock or unclassified new
  source makes the applicable strict check fail.
- All 115 pinned Driver and Spaces tool rows have explicit conformance
  evidence. The 23 broader product families remain visible acceptance work.
- Pin changes are reviewed migrations, with no silent update during shell
  entry, install, fixture generation or parity checks.

## Progress

Pinned 21 root Cargo declarations to their current lock resolutions, all
13 Nix inputs to their original commits, and rbonsai 0.1.5 and mbx 1.22.0
installers to exact versions. Recovered five Git subtree source commits.
Copied exact CUA S1 and perception artifact locks, and both public contract
manifests, with Git blob and SHA-256 verification.

The reference inventory captures 38 lock or Zig dependency files and source
manifests. It currently reports 5,023 unresolved URL occurrences, largely
inside imported source and documentation. This count is an inventory of
occurrences, not a count of distinct runtime dependencies.

The parity index preserves every tool and starts each row unverified.
Existing integrations need exact contract evidence before changing that
status. Strict reference and parity checks currently fail as expected.

Validation passed: three pin parsing tests, 14 CUA ledger tooling tests,
`devenv shell -- cargo check --locked --all-targets`, and the 414-file ledger
proof with 23 reproduced patches and 48 behavior source objects.
The original implicit devenv input updated during an early shell invocation.
Restored its original commit and added it explicitly to `devenv.yaml`.
A second shell invocation preserved that pin.

## Follow-up

After this pass, discuss durable approvals and notification management as
requested. Ticket 10 owns approval implementation. Current clarification
responses do not grant execution permissions.
