# 15 — Disposable desktops and separately licensed remote-product reuse

Type: research
Status: needs-triage
Blocked by: 01, 02, 10, 11
Effort: exploratory/large

## Brief

Evaluate MIT `cua-vmm` QEMU/OCI lifecycle as a Rust module behind Dojo/Pi execution adapters. Add owned images, nested desktop setup, filesystem/process access, teardown, cancellation and resource/network policies where required. VM lifecycle is distinct from local desktop actions; sandbox capabilities must not silently expand host authority. Pi owns task scheduling and credentials; sandbox module owns only its external lifecycle receipts/resources.

Separately inventory MIT SDK/protocol/client pieces useful for remote machines. For FSL Spaces/streaming/relay/Teleport/Keyvault, record exact source/dependency/use terms and a product/distribution decision before code incorporation. Do not rewrite around legal terms to pretend a translated derivative is unrestricted. Lume is macOS-only; remote macOS use is a host integration, not Linux implementation parity.

## Acceptance

- Isolated Rust lifecycle proof with restart reconciliation and teardown that never kills unowned processes.
- Permissions/resource/network/image policies and fixture tests; artifact provenance and licensing.
- Remote authorization and human-priority semantics specified before exposing remote mutations.
- Explicit go/no-go per product slice; no cloud account, paid infrastructure or secret transfer assumed.
- No mandatory spacesd/FSL server or second runtime added to local Dojo's critical path.

## Comments
