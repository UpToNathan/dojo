# Full parity references at the pinned CUA revision

Research date: 2026-10-12. Source authority is CUA commit `88aff7e5424e9ed45b93ded85eb4111472ca5190`. The local checkout HEAD matches that commit. Every source link below uses the immutable commit. Git object hashes identify exact trees or blobs, independently of a branch name. This report inventories product families and source paths; it does not claim line-by-line implementation equivalence or live provider/model artifact verification.

Full parity includes the local Driver, model workflows, benchmarks, specialist inference, sandbox SDK, cloud/host services and Spaces products. Earlier scope documents that treat tickets 14 and 15 as optional need an explicit update under the new target. Source imports and passing standalone tests do not close an integrated behavior row. [Current Dojo evidence](result.md) records the existing partial implementation and outstanding gates.

## Capability matrix

| Capability family | Pinned source | Git object | Current Dojo gap and acceptance requirement |
|---|---|---|---|
| Driver contract/MCP/CLI/native SDK | [libs/cua-driver/contract/manifest.json](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/contract/manifest.json) | `fcc4a8a0a25a686d957a092c1da148752e9defda` | Private contract/core imports and Dojo MCP are partial. Public C ABI, UniFFI bindings, daemon and upstream tool contract are not exposed. Enumerate every tool/variant and generated SDK fixture. |
| Capture/window/input/cursor | [libs/cua-driver/rust/crates/cua-driver-core/src](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/rust/crates/cua-driver-core/src) | `8974a4537e53534e6b67e7fab9e624ee826501c1` | Native Niri capture, one-use IDs and bounded rich input work. Unicode/IME/clipboard, full window lifecycle, transforms, client/crash matrices, cross-window drag, video and cursor overlays remain separate acceptance gaps. |
| Native accessibility | [libs/cua-driver/rust/crates/platform-linux/src](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/rust/crates/platform-linux/src) | `0320968684976cff43fec9d4ea1b299c3a0d78f6` | Application-scoped AT-SPI reads exist. Exact top-level correlation and element mutation/ref authority remain absent. Linux backend parity includes X11 and other Wayland compositors if full platform support is required. |
| Browser engine/profile/storage | [libs/cua-driver/rust/crates/cua-driver-core/src/browser](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/rust/crates/cua-driver-core/src/browser) | `eb5815f6cd9912ebe1e8b48967df67b89cb8e1ba` | Owned Helium observation and exact scope work. General CDP writes, existing-profile grants, download/upload, storage operations, embedded Electron and settled outcomes need complete acceptance. |
| Authorization/consent/history/privacy | [libs/cua-driver/rust/crates/cua-driver-core/src/authorization.rs](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/rust/crates/cua-driver-core/src/authorization.rs) | `d8c79dd0b20baaedc56309ccf694d7bd8be66812` | Immutable YAML restriction works. Durable execution approvals, standard/bounded/unrestricted capability manifests, approved profile access, permission-gated history, canonical transcript privacy are unfinished. Preserve Pi as durable owner. |
| Verification/batches/recording | [libs/cua-driver/rust/crates/cua-driver-core/src/batch_tools](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/rust/crates/cua-driver-core/src/batch_tools) | `bfce1af9d65850e7688056a4d79e66c28e37195d` | Bounded Pi steps and verifier exist. Upstream batch query/action/expectation families, recording render/zoom/video, encrypted history, and all externally visible error/results need explicit fixtures. |
| Perception extension | [libs/cua-driver/rust/crates/cua-perception](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/rust/crates/cua-perception) | `cbee4148eadc1452e4faa0b82003d75a84439a8c` | Rust ONNX worker, manifests, artifacts, quality corpus and source bundle are not integrated. Region observations must keep capture identity and grant no independent effect authority. |
| Model profiles and native provider wires | [libs/python/agent/cua_agent/loops](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/python/agent/cua_agent/loops) | `377d981ce8fb3b5a5927fecf3e45d7309868ef3f` | Textual dialects and pure decoders ported. Native Anthropic/OpenAI/Gemini declarations, streams, replay, safety envelopes, credentials and endpoint tests are incomplete; composed grounding and image-byte parity need complete fixtures. |
| Agent callbacks/human/telemetry | [libs/python/agent/cua_agent/callbacks](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/python/agent/cua_agent/callbacks) | `1bfecaf0db352419b5d172af68cd6454c02fab42` | Pi budgets/retention/clarification and export projection exist. Full callback result parity, durable permission distinction, PII policy, telemetry sinks and application logging are gaps. Upstream ComputerAgent scheduler must be translated into Pi workflows. |
| Benchmark/task/evaluator/data | [libs/cua-bench/cua_bench](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/cua_bench) | `df562dc05092466212538c93d0f1a036afdbdcda` | Pure episode/scoring/processors are partial. Task setup, application evaluators, dataset adapters, simulated browser tasks, batch/oracle solvers and training exports need full Rust equivalents and isolated test environments. |
| S1 inference/training/evaluation | [libs/cua-s1](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-s1) | `495a36a9cdee0368f659da6bd9cca7844c3cb73d` | Schema/planner policy exists. Forms/nano/4B model inference, tokenizer/image preprocessing, training/synthetic data, RL and quality evidence are not complete. Rust equivalents required; source alone is not checkpoint parity. |
| Bench S1/RL/driver performance suites | [libs/cua-bench-rl](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench-rl) | `fc5421ab7b972449ed88f914190f9175ac9699ae` | RL environments, reward/rollout APIs, S1 evaluation and driver automated benchmarks are not ported. Benchmark source licenses differ by directory. |
| Sandbox placement/lifecycle/readiness | [libs/cua/crates/cua-sandbox-core](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-sandbox-core) | `484931b9313bf744b957f90a1135f9b61aad04ed` | No integrated parity. Local/cloud/direct sandbox placement, state, expiry, sidecars, service requests/forward/public URLs and daemon-independent readiness need contracts and adapters. |
| QEMU/OCI/gVisor/Lume runtimes | [libs/cua/crates/cua-vmm](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-vmm) | `21fb61770f7f1d47a9e370292cfaba41006c45b4` | No integrated parity. Linux QEMU/container/gVisor providers are portable Rust. Apple Virtualization/Metal Lume needs an Apple host or remote provider, not a Linux implementation of Apple APIs. |
| OCI images/builds/disk/cache | [libs/cua/crates/cua-image](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-image) | `e419956214ad216041f2ff2afeaece5e67140441` | No integrated parity. Image resolution/pull/push/build, rootfs/containerDisk, signed/hash artifacts, disk accounting and cache GC need explicit image digest locks. |
| Fleet/cloud/control plane | [libs/fleet](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/fleet) | `9abe565828d1d995ec3aed3192e1308fecb3168c` | No integrated parity. Pools/templates/claims, credentials/RBAC, WIF, provisioning, remote builds and Terraform are a separate product track. Source cannot reproduce private service deployment or guarantee hosted endpoint capability. |
| SDK/CLI/daemon/auth/host | [libs/cua](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua) | `b7baeeca41de94036024a29c988e3b7716764b87` | Most SDK/CLI services absent. Embedded/daemon API, PKCE/device auth, host enrollment, relay directory, unattended service installation, tunnels and language bindings require independent implementation/capability tests. |
| Spaces daemon/protobuf/files/processes | [libs/cua-spacesd](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-spacesd) | `758236d2ddc4841331cbeaaff661634940a234e4` | No integrated parity. Port3211 gRPC, process/files/window/input services, sessions, provider capability probes, doctor and server are distinct from Dojo MCP. |
| Streaming/viewer/media/presence | [libs/cua/crates/cua-media-protocol](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-media-protocol) | `155a216b38f5e21c688d57f15738266e27ce205e` | No integrated parity. Wire-v2 video/audio, QUIC, codecs, reconnect, proxy viewer, HTML5 viewer and presence must pin protocol/schema, native codec builds and fixtures. |
| Spaces UI/app core/agents/routines | [apps/cua-spaces](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/apps/cua-spaces) | `1d04c0b9620844ff2106a37ebc79d09381f4b1ad` | No integrated parity. Shared Rust view models, Tauri Linux/Windows UI, Swift macOS UI, persistent agents, routines and notifications require product integration. Agent schedules/history stay in Pi. |
| Teleport/keyvault/storage/volumes | [libs/cua/crates/cua-keyvault](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-keyvault) | `840ad03d8db10eae00e532bf210ca7f3f5cc463a` | No integrated parity. Explicit consent, credential export/receive, app layouts, Chromium LevelDB, machine seal, volume FUSE/remote sync and site login are new capabilities, not implied by owned browser. |
| Setup/icons/transcripts/health | [libs/cua/crates/cua-agent-setup](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-agent-setup) | `b1b689d91b46f4b641cf0ae514a61c4f67ab282a` | Setup plan/apply and icon/transcript modules partly integrated. Full agent detection/config formats, package install/uninstall/update, health reports and deployment evidence remain. |
| Legacy packages/samples/fixtures/bindings | [README.md](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/README.md) | `3abb935b5cb35272c250aebfc24b5f16b25dfef3` | Deprecated computer/server/agent/SOM/cuabot are reference behavior and migration tracks, not current deployment defaults. Samples and generated bindings are source/versioned fixtures, not proof every demonstrated behavior is supported. |

## Existing immutable artifact locks worth copying

The upstream S1 lock already pins both repositories and every selected file's byte count and SHA-256. Copying that entire lock with its source provenance avoids resolving a moving model alias. These are upstream declared checksums, not independently downloaded and verified bytes in this investigation.

[libs/cua-s1/ci/weights.lock.json](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-s1/ci/weights.lock.json) has blob `dfb9748b45d850ea6e5b7ada784bf6697ca19431`.

- `Qwen/Qwen3.5-4B` at `851bf6e806efd8d0a36b00ddf55e13ccb7b8cd0a`, role `base`, 14 locked files.
- `cua-ai/cua-s1-4b-0.2` at `16818868b0cc7813808aae4e87b417657046ab79`, role `adapter`, 6 locked files.

[libs/cua-driver/rust/crates/cua-perception/scripts/artifacts.lock.json](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/rust/crates/cua-perception/scripts/artifacts.lock.json) has blob `1a34e7396e9c4b512ed42cdf95fbe9f2888c83fd`. It pins ONNX Runtime `1.26.0`, archive SHA-256 and extracted library SHA-256 separately for each supported target. It pins corresponding model/exporter sources and licenses. Preserve all fields, conversion recipe and quality corpus.

- `https://huggingface.co/microsoft/OmniParser-v2.0` revision `6600256cb0f1b07651e3bc86166196307bad7e2d`; source archive/file SHA-256 `dab3d4351ad00b035db829909a4db98354d5a90f6990e4ac00222a9a95d4bf57`.
- `https://github.com/ultralytics/ultralytics` revision `8f7fe171f0db13e5c2eff55eee3443f8903e5068`; source archive/file SHA-256 `6c289215a5ce593a872111b87e58ffff6949d235c2af720e8f01704375f7bf44`.
- `https://github.com/pytorch/pytorch` revision `ba56102387ef21a3b04b357e5b183d48f0afefc7`; source archive/file SHA-256 `a9623012b046e9cc87c919f178782860bcee9fbf014ad5c24a2655b517d0881b`.
- `https://github.com/pytorch/vision` revision `824e8c8726b65fd9d5abdc9702f81c2b0c4c0dc8`; source archive/file SHA-256 `8e44b7705c901b9af76113393fe675b02ceeb43b1c688db50e0d8ff8a9dcef03`.
- `https://github.com/onnx/onnx` revision `57b9c6a4f6eebb09ae1b34cfb632078518f72832`; source archive/file SHA-256 `a1a9c44e20ea1f9cf0c094e0f82001113b36e0867e04c06966475c46e3c75739`.
- `https://github.com/inisis/OnnxSlim` revision `0.1.68`; source archive/file SHA-256 `ff1ec2bf61ff10b75e33990b2046c446dec522ba7f0a12b2f984547ff43f9138`.

Other S1 model cards name forms, nano and 4B0.1 variants. General `from_pretrained` callers still use model IDs without universally supplying revisions. Pin each actually supported profile's base weights, adapter, tokenizer, processor, chat template and model card; do not infer that the 4B0.2 smoke lock covers every variant. Benchmark datasets need repository revisions, splits/configurations, complete consumed-file hashes, license snapshots and task/evaluator source revisions. Simulated HTML and source fixtures can be pinned as Git blobs.

## Pin requirements across references

| Reference kind | Required pin | Evidence that closes the requirement |
|---|---|---|
| Copied/forked source | Repository, full commit, source object hashes, local file hashes, exact patch | Re-extract upstream bytes and apply patches to reproduce maintained bytes |
| Registry dependency | Exact resolved version, registry identity, checksum/integrity, lockfile | Locked/offline-capable resolution; audit every active workspace and optional feature |
| Git dependency/action/submodule | Full commit and content hash | Resolve and verify exact object; tags/branches remain labels only |
| Nix/toolchain/system package | Lock narHash/revision, compiler and target, closure identity | Rebuild from pinned inputs and record runtime external binary identity |
| OCI/VM/image/firmware | Manifest digest, platform digest, disk/firmware hashes | Pull/fetch verifies digest; aliases such as linux24.04 are not immutable |
| Download/release/install script | Exact release/ref plus SHA-256 of bytes | Verify before execution; latest endpoint is discovery only |
| Model/dataset | Repository commit plus consumed-file hashes and terms | Exact offline bytes, tokenizer/processor parity, reference task outputs |
| API/protocol/schema | Exact source/schema revision, versioned fixtures, tested endpoint/model capability | Round-trip golden envelopes; service alias alone cannot pin remote implementation |
| Documentation/behavior citation | Immutable source blob or dated snapshot with content hash | Document bytes can be recovered; mutable docs URL is origin metadata |
| Generated bindings | Generator/source/toolchain pins and generated file hashes | Regenerate with check mode and compare bytes |
| Host-specific framework | SDK/build/OS version and host target | Tests on corresponding platform, or typed remote provider contract |

A local working tree digest pins Dojo's current implementation, but it cannot substitute for the upstream base commit of a fork. A manifest's broad semver range can coexist with a lock during development, but unattended installer/update behavior must use the exact lock or declared artifact hashes. A provider model marketing alias cannot establish immutable model weights or remote service deployment. Record the exact observed API/model capability and schema evidence separately from the locally pinned client.

## Host-specific and unavailable evidence

Apple Accessibility, ScreenCaptureKit, Virtualization.framework, Metal, macOS Keychain and SwiftUI cannot run natively on Linux. Their data contracts and backend-independent logic can be ported. Platform implementation parity requires a macOS build/test host or a remote macOS provider. Windows UIA/Win32/DXGI/service implementations likewise need a Windows backend and host tests. X11, Sway, GNOME, KDE and Niri are distinct desktop backends; Niri tests alone cannot claim the complete upstream Linux matrix. Cloud relay/Fleet authentication and hosted model behavior require real endpoint contracts and credentials; public source does not prove deployed service parity. The no-production-Python constraint requires Rust replacements for Python-only benchmark callbacks, inference/training and data tooling; shelling out to an unnoticed Python environment does not close that work.

## Complete component source-object index

This index prevents whole product crates from disappearing behind the local Driver scope. A tree hash covers every tracked file beneath that component at the pinned commit. Capability acceptance must still enumerate public operations and fixtures. License boundaries follow the pinned licensing sources, not a blanket root MIT assumption.

[LICENSING.md](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/LICENSING.md) is blob `c274054477e3b217f6c6b15c3a90c8858d0debd9`; [LICENSE.md](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/LICENSE.md) is blob `b8b198ce3ebaa22c9f59588a80f9f1875d85b34a`. Spaces/Driver Bench, optional SOM/perception and downloaded artifacts require their own pinned license evidence.

| Component | Git tree |
|---|---|
| [libs/cua](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua) | `b7baeeca41de94036024a29c988e3b7716764b87` |
| [libs/cua-bench](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench) | `922c83f6f917c74d28cce6e795ca824f3384966a` |
| [libs/cua-bench-rl](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench-rl) | `fc5421ab7b972449ed88f914190f9175ac9699ae` |
| [libs/cua-bench-s1](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench-s1) | `5af2efcfdec1d793e208811e2a759a099fd2fc22` |
| [libs/cua-driver](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver) | `48a9400a8ee35a11679e33949546874562fff0a4` |
| [libs/cua-driver-fixtures](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver-fixtures) | `f23585116935ef61e220c3b59b18d64ecf045b6f` |
| [libs/cua-s1](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-s1) | `495a36a9cdee0368f659da6bd9cca7844c3cb73d` |
| [libs/cua-spacesd](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-spacesd) | `758236d2ddc4841331cbeaaff661634940a234e4` |
| [libs/cuabot](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cuabot) | `21f40693bbf5b3765c610ec9cf3835f6d85f68cb` |
| [libs/fleet](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/fleet) | `9abe565828d1d995ec3aed3192e1308fecb3168c` |
| [libs/images](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/images) | `8029038aba84806a228747db9be6a8bf133124dd` |
| [libs/kasm](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/kasm) | `0266113292d506dbbb7f4629e6ab7e900046b291` |
| [libs/lume](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/lume) | `425c41c00135b9eb7278d5e2dabb7f33ddb14e41` |
| [libs/lumier](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/lumier) | `bd7556bdb81648a282d532cf2cf4c81c6162e985` |
| [libs/python](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/python) | `429cf1a44d85bad179d0f233450574a96b2d7d01` |
| [libs/qemu-docker](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/qemu-docker) | `f3405f830226de5fd45481e434d159a32694a568` |
| [libs/spaces-app-swift](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/spaces-app-swift) | `707aaf412f11dee9d5dd588ae187ad33e51fd72f` |
| [libs/spaces-notch-swift](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/spaces-notch-swift) | `cd9ff09fe2f539996f0363a60d3716cfa2f9c456` |
| [libs/spaces-sdk-swift](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/spaces-sdk-swift) | `6ed9d9a8feb91b808767fdb3b6723ecba0b4f69b` |
| [libs/typescript](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/typescript) | `2f4552132adf52307f992076d2abbd337aff7f6c` |
| [libs/xfce](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/xfce) | `d6329cc9f1fa555df33bad592aa653e6b3d5e841` |
| [libs/xfce-cua](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/xfce-cua) | `c5d602a45ed7252abdbb788ca9e34e4a3a11e510` |
| [apps/cua-spaces](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/apps/cua-spaces) | `1d04c0b9620844ff2106a37ebc79d09381f4b1ad` |
| [apps/cua-spaces-desktop](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/apps/cua-spaces-desktop) | `fd316642b9374f21d3cbe990422033813d1463f8` |
| [apps/cua-spaces-macos](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/apps/cua-spaces-macos) | `9418a1775317ac559771496a1d4c602ec49f58cd` |
| [apps/cua-spaces-web](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/apps/cua-spaces-web) | `aaf9636c654f6748332a56950454037d3b243aaa` |
| [libs/cua/crates/cua-agent-setup](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-agent-setup) | `b1b689d91b46f4b641cf0ae514a61c4f67ab282a` |
| [libs/cua/crates/cua-agents](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-agents) | `aa6e4d6414ca06a4f2cc5d070f2fc806118088e4` |
| [libs/cua/crates/cua-auth](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-auth) | `164f6410110e4798bf082291a263ccfb7eb8809a` |
| [libs/cua/crates/cua-bindgen](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-bindgen) | `0b742ccd99ec9764261411b537a57ab7b1cc0644` |
| [libs/cua/crates/cua-byoc](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-byoc) | `3a2780f6fad746842d522e0b0a6875213e8a214a` |
| [libs/cua/crates/cua-chromium-storage](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-chromium-storage) | `c1e86df91176aba67cb1aedffce9802d98dd4898` |
| [libs/cua/crates/cua-cli](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-cli) | `4baeacf0dfc3cd017ce968de748c4e6ddcf1f25f` |
| [libs/cua/crates/cua-contrib](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-contrib) | `cb66929e5a0b36847bed24d356edf67589a6db3b` |
| [libs/cua/crates/cua-daemon](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-daemon) | `a335c40d5a6daab80a1c6392f82f2c1fbd56b218` |
| [libs/cua/crates/cua-disk](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-disk) | `14de77c7adfd7894f3083d7657076e97d2ce51b0` |
| [libs/cua/crates/cua-fleet](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-fleet) | `42aaf1d221ad9595663920404589e818bc1d1459` |
| [libs/cua/crates/cua-home](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-home) | `a4a1744de32b31499cfde48d6d2712683eea7e63` |
| [libs/cua/crates/cua-host](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-host) | `9a8ebefd612ad7549c2702cf90d148680e0f205a` |
| [libs/cua/crates/cua-hotspot](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-hotspot) | `7d13b625cb82679e97e78121fc83c638ffc01626` |
| [libs/cua/crates/cua-icon-cache](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-icon-cache) | `aaeb26fe5837fd4345aef05394bd1ea80688c9a4` |
| [libs/cua/crates/cua-image](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-image) | `e419956214ad216041f2ff2afeaece5e67140441` |
| [libs/cua/crates/cua-keyvault](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-keyvault) | `840ad03d8db10eae00e532bf210ca7f3f5cc463a` |
| [libs/cua/crates/cua-logging](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-logging) | `197028c8e84c932340420901dbc138e0d24ebc49` |
| [libs/cua/crates/cua-machine-seal](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-machine-seal) | `52df964c45dc9cdb60c0a7ab421069ad9d8d8e23` |
| [libs/cua/crates/cua-media-client](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-media-client) | `d4a690d8f90323baf797902653d5631fbc9fc51f` |
| [libs/cua/crates/cua-media-codec](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-media-codec) | `09df750880cc50f69a317269b3b1fb7ad6da4338` |
| [libs/cua/crates/cua-media-protocol](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-media-protocol) | `155a216b38f5e21c688d57f15738266e27ce205e` |
| [libs/cua/crates/cua-media-transport](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-media-transport) | `cf5405fa323d5c6327b7353d033aca5d226e3732` |
| [libs/cua/crates/cua-mock-llm](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-mock-llm) | `adfcd995b4421fa65e493b1c3fe0b7c5ff2d0bb0` |
| [libs/cua/crates/cua-proto](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-proto) | `85ef03103affcfd7769416ec96d19720697c7ffb` |
| [libs/cua/crates/cua-sandbox-core](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-sandbox-core) | `484931b9313bf744b957f90a1135f9b61aad04ed` |
| [libs/cua/crates/cua-sdk](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-sdk) | `76477d778f2796af97fa44039a82ec382b9e4851` |
| [libs/cua/crates/cua-spaces](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-spaces) | `03d4422dd4711173bf42b0f584973646ae209655` |
| [libs/cua/crates/cua-spaces-app-core](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-spaces-app-core) | `77458150fe85c7cef407b0f3232964e48073021a` |
| [libs/cua/crates/cua-spaces-cli](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-spaces-cli) | `a935577904161961cb6e7f4a16f92051b72d1c80` |
| [libs/cua/crates/cua-spaces-contract](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-spaces-contract) | `bb98f9bb9e213eba10c44892700d5396d47240d1` |
| [libs/cua/crates/cua-spaces-ext](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-spaces-ext) | `016466e4865bcef17eea412f99c7eb8051b2dcbc` |
| [libs/cua/crates/cua-spaces-ffi](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-spaces-ffi) | `6b93233a9449355786e50e42cf03a2506fd25ae8` |
| [libs/cua/crates/cua-spaces-transcript](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-spaces-transcript) | `adf173dc1f36a3be63ebe2410e73af4fea1a07ba` |
| [libs/cua/crates/cua-spacesd-client](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-spacesd-client) | `8a4d519f3fa1fe187494091601627f9f13817fd7` |
| [libs/cua/crates/cua-spacesd-html5](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-spacesd-html5) | `bd743f80ee0bc4f6e1f7267b888d9bd04ec0be4b` |
| [libs/cua/crates/cua-telemetry](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-telemetry) | `b68f1c5ea40041b9b6e8dbd7952c1681ba9b693a` |
| [libs/cua/crates/cua-teleport](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-teleport) | `e277aee457e3e76e1a810aa847dc10dcb205f999` |
| [libs/cua/crates/cua-teleport-bundle](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-teleport-bundle) | `945d168f4e99c2c962f5a72e7bd72a01eb2aaacf` |
| [libs/cua/crates/cua-viewer](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-viewer) | `4d4c0957360cfb3ea563747bcb6e779aa4e19d1c` |
| [libs/cua/crates/cua-vmm](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-vmm) | `21fb61770f7f1d47a9e370292cfaba41006c45b4` |
| [libs/cua/crates/cua-volume](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-volume) | `6cdcc7aa32ee2b3a39cfdf7ccdf14c381338318a` |
| [libs/cua-driver/rust/crates/cua-cursor-motion](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/rust/crates/cua-cursor-motion) | `8d56618693ea8c6920a62d780a53be4610f9cec6` |
| [libs/cua-driver/rust/crates/cua-driver](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/rust/crates/cua-driver) | `09d08d4ffbfee425b907a27717ab10b1d321d173` |
| [libs/cua-driver/rust/crates/cua-driver-bindgen](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/rust/crates/cua-driver-bindgen) | `92c42486bb902fa854bcb93921453b400f7ef80f` |
| [libs/cua-driver/rust/crates/cua-driver-contract](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/rust/crates/cua-driver-contract) | `98621996db3dee73045848631fa39b074c6374f9` |
| [libs/cua-driver/rust/crates/cua-driver-core](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/rust/crates/cua-driver-core) | `bb133051da49bae60700714c79d3cf99a0ff798e` |
| [libs/cua-driver/rust/crates/cua-driver-e2e](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/rust/crates/cua-driver-e2e) | `4df17a8aadc07ee568465005ac21d4102826b39f` |
| [libs/cua-driver/rust/crates/cua-driver-sdk](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/rust/crates/cua-driver-sdk) | `befdc3b4a76c4f4416e3ca88f7a7eb63a539ae12` |
| [libs/cua-driver/rust/crates/cua-driver-testkit](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/rust/crates/cua-driver-testkit) | `d7423079b91a16cae54554aa1121a01aa4f62046` |
| [libs/cua-driver/rust/crates/cua-driver-uia](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/rust/crates/cua-driver-uia) | `fd9c8d44beb235a764d334ded5bc94197e5aa74f` |
| [libs/cua-driver/rust/crates/cua-perception](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/rust/crates/cua-perception) | `cbee4148eadc1452e4faa0b82003d75a84439a8c` |
| [libs/cua-driver/rust/crates/cursor-overlay](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/rust/crates/cursor-overlay) | `7d302b24ec2a5e218d02b73800eac78336fb6cd0` |
| [libs/cua-driver/rust/crates/cursor-theme-cli](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/rust/crates/cursor-theme-cli) | `928d27d8657280678539c154404c3c309812ae96` |
| [libs/cua-driver/rust/crates/pip-preview](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/rust/crates/pip-preview) | `f7bc009a59c7a6a4185ca262731afe2467f040f5` |
| [libs/cua-driver/rust/crates/platform-linux](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/rust/crates/platform-linux) | `290ecaf9ddf553700ec4c020f77048682afb9f3f` |
| [libs/cua-driver/rust/crates/platform-macos](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/rust/crates/platform-macos) | `f15773ab65aac904053be81bce70bf3c29e61a94` |
| [libs/cua-driver/rust/crates/platform-windows](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/rust/crates/platform-windows) | `73038d054c77799db866b9d0f208530637c8585b` |
| [libs/cua-spacesd/crates/cua-relay](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-spacesd/crates/cua-relay) | `cae97def04abba5c4960ec03405fd43469dc6846` |
| [libs/cua-spacesd/crates/cua-spacesd](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-spacesd/crates/cua-spacesd) | `b48b66e2115364b7c26b6c6cd4f9c5cf806b24d0` |
| [libs/cua-spacesd/crates/cua-spacesd-desktop](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-spacesd/crates/cua-spacesd-desktop) | `5776c5c99d3afea12fb06d18a938ee87687251e0` |
| [libs/cua-spacesd/crates/cua-spacesd-doctor](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-spacesd/crates/cua-spacesd-doctor) | `2661f7382978fdaf57d1729931180c2fc195dce2` |
| [libs/cua-spacesd/crates/cua-spacesd-provider-api](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-spacesd/crates/cua-spacesd-provider-api) | `1aba85ece15c37b4ec4a278b22f3762b764e287f` |
| [libs/cua-spacesd/crates/cua-spacesd-server](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-spacesd/crates/cua-spacesd-server) | `ceed078a0e64b9c5108e6d50869885397d6b6bcb` |
| [libs/cua-spacesd/crates/cua-spacesd-session](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-spacesd/crates/cua-spacesd-session) | `857fc4e0cbbe4beb2b63098cf1b81e0b44907ff5` |
| [libs/cua-spacesd/crates/cua-spacesd-socks](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-spacesd/crates/cua-spacesd-socks) | `404601f68201e5751da72376526dd24eca81bd22` |
| [libs/cua-spacesd/crates/cua-spacesd-teleport](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-spacesd/crates/cua-spacesd-teleport) | `57d0acaa29ef7515654bfed3715e49a891adebd6` |
| [libs/cua-spacesd/crates/cua-spacesd-test-apps](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-spacesd/crates/cua-spacesd-test-apps) | `2d459b7fe7a04836666038ee4cea64456431cd94` |

## Dependency lock/source manifest index

These objects pin upstream dependency declarations and their existing resolutions. Importing a new component needs its own active closure audit, including optional native libraries and generated bindings. Python locks remain reference inputs under the Rust constraint.

| Source manifest or lock | Git blob |
|---|---|
| [apps/cua-spaces-desktop/pnpm-lock.yaml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/apps/cua-spaces-desktop/pnpm-lock.yaml) | `96060e4f5ee2d9499638cbac2abd8ca5aacff12d` |
| [apps/cua-spaces-macos/Package.resolved](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/apps/cua-spaces-macos/Package.resolved) | `7f2df47e163737414c6ae42cf36d3ea504497bb0` |
| [apps/cua-spaces-web/pnpm-lock.yaml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/apps/cua-spaces-web/pnpm-lock.yaml) | `0b5f552e06f69f61337fc6ea9fa60f1a77da6955` |
| [apps/cua-spaces/core-wasm/Cargo.lock](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/apps/cua-spaces/core-wasm/Cargo.lock) | `78a43b64bfb01ac91c996f6803327d8b623c50fa` |
| [apps/cua-spaces/core-wasm/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/apps/cua-spaces/core-wasm/Cargo.toml) | `dbab7cf2ef808d7e36faada8b69d39362ea86dff` |
| [apps/cua-spaces/pnpm-lock.yaml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/apps/cua-spaces/pnpm-lock.yaml) | `54365f1766914eec0fa32a40757e1555cb595390` |
| [apps/cua-spaces/rust-toolchain.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/apps/cua-spaces/rust-toolchain.toml) | `28a74c61f0ba3a13f7073140383407143230c26b` |
| [apps/cua-spaces/src-tauri/Cargo.lock](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/apps/cua-spaces/src-tauri/Cargo.lock) | `62d833e8479f1e8cce5a71d695f8fcab2218017f` |
| [apps/cua-spaces/src-tauri/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/apps/cua-spaces/src-tauri/Cargo.toml) | `191f956a7f152c05fa1081b00cd881122f5d50d6` |
| [flake.lock](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/flake.lock) | `ece13ab181a37aa9e125df3f790de44324c48917` |
| [libs/cua-bench-rl/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench-rl/pyproject.toml) | `6e39a6fe05685a3abb36190c5518ecbb77a21719` |
| [libs/cua-bench-rl/uv.lock](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench-rl/uv.lock) | `476b67d33a8168642e771d16db92dba9edc23097` |
| [libs/cua-bench-s1/python/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench-s1/python/pyproject.toml) | `c57c49f6df7b57e350c632a763955c5c1d828af9` |
| [libs/cua-bench/datasets/cua-bench-basic/click-button/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/datasets/cua-bench-basic/click-button/pyproject.toml) | `d120b99085a0d59cb08d89278859a740453f8a16` |
| [libs/cua-bench/datasets/cua-bench-basic/click-icon/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/datasets/cua-bench-basic/click-icon/pyproject.toml) | `4b3294d20f7a2fa4712475637b52f837c3294a8e` |
| [libs/cua-bench/datasets/cua-bench-basic/color-picker/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/datasets/cua-bench-basic/color-picker/pyproject.toml) | `40e5fda63d036c56117c4d0b17bdc30f516229af` |
| [libs/cua-bench/datasets/cua-bench-basic/date-picker/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/datasets/cua-bench-basic/date-picker/pyproject.toml) | `17cceb26c9f3c4f4f63aece277435ee50d51ca05` |
| [libs/cua-bench/datasets/cua-bench-basic/drag-drop/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/datasets/cua-bench-basic/drag-drop/pyproject.toml) | `ca53ad19a5abc7225f15010c23b12fa926b5c7d3` |
| [libs/cua-bench/datasets/cua-bench-basic/drag-slider/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/datasets/cua-bench-basic/drag-slider/pyproject.toml) | `f86d4dba78a23f1bce9309745579a2ee0637541b` |
| [libs/cua-bench/datasets/cua-bench-basic/fill-form/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/datasets/cua-bench-basic/fill-form/pyproject.toml) | `e9d3a1743968cd1e4bd1de3cd34d2452750849e8` |
| [libs/cua-bench/datasets/cua-bench-basic/right-click-menu/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/datasets/cua-bench-basic/right-click-menu/pyproject.toml) | `53b291bb3fd9ab9b617da495daa93444fd83770c` |
| [libs/cua-bench/datasets/cua-bench-basic/select-dropdown/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/datasets/cua-bench-basic/select-dropdown/pyproject.toml) | `f1cf1e5a5b20a2520873d8872201a342b188bec7` |
| [libs/cua-bench/datasets/cua-bench-basic/spreadsheet-cell/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/datasets/cua-bench-basic/spreadsheet-cell/pyproject.toml) | `4b659266586910080f495dc408067ad887b8998c` |
| [libs/cua-bench/datasets/cua-bench-basic/toggle-switch/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/datasets/cua-bench-basic/toggle-switch/pyproject.toml) | `dd0d8a2407cc3ee8ef583bd87c0bd6bfd3e07349` |
| [libs/cua-bench/datasets/cua-bench-basic/typing-input/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/datasets/cua-bench-basic/typing-input/pyproject.toml) | `bbd109a796675dd8b6d4a96eb612aac85ddd9e3a` |
| [libs/cua-bench/datasets/cua-bench-basic/video-player/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/datasets/cua-bench-basic/video-player/pyproject.toml) | `537cadc7aeb4b688aa6af032c78a703ff261c3a5` |
| [libs/cua-bench/datasets/cua-bench-kicad/154d0750/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/datasets/cua-bench-kicad/154d0750/pyproject.toml) | `ec36ff07d5d1ee096cbc671b7f096f841c25af5f` |
| [libs/cua-bench/datasets/cua-bench-kicad/1625e97a/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/datasets/cua-bench-kicad/1625e97a/pyproject.toml) | `3087a4739c2b9b8cad018f37bdbb49fbff91111c` |
| [libs/cua-bench/datasets/cua-bench-kicad/2c11fe94/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/datasets/cua-bench-kicad/2c11fe94/pyproject.toml) | `253f6b56651b1afbdf438ebe864976acc1683fb9` |
| [libs/cua-bench/datasets/cua-bench-kicad/34877be3/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/datasets/cua-bench-kicad/34877be3/pyproject.toml) | `c44d171634223b9bbc3b3e30247d725b9886652a` |
| [libs/cua-bench/datasets/cua-bench-kicad/458f5f5c/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/datasets/cua-bench-kicad/458f5f5c/pyproject.toml) | `f9d2ece3369baedb70d2a2b8c25456c1847bda1d` |
| [libs/cua-bench/datasets/cua-bench-kicad/4bd3e530/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/datasets/cua-bench-kicad/4bd3e530/pyproject.toml) | `2695a28cb6e971b136300292f6105b26404dd4cb` |
| [libs/cua-bench/datasets/cua-bench-kicad/4c913e30/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/datasets/cua-bench-kicad/4c913e30/pyproject.toml) | `6e896f6fe6faac162cca83d3120c28e47df9340b` |
| [libs/cua-bench/datasets/cua-bench-kicad/55f2eefb/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/datasets/cua-bench-kicad/55f2eefb/pyproject.toml) | `b7b721cbb27993f87c53c08fc2bab4185d174761` |
| [libs/cua-bench/datasets/cua-bench-kicad/589da233/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/datasets/cua-bench-kicad/589da233/pyproject.toml) | `7382ddf073d90a8b6f13399943635be06facc7d0` |
| [libs/cua-bench/datasets/cua-bench-kicad/5dbc9e02/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/datasets/cua-bench-kicad/5dbc9e02/pyproject.toml) | `87e4fa07afde46ace5ad376cf9aff8d5f9c1528e` |
| [libs/cua-bench/datasets/cua-bench-kicad/6cb23037/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/datasets/cua-bench-kicad/6cb23037/pyproject.toml) | `c9c0fd8d93b1f578543e969d64839c645c0e74d6` |
| [libs/cua-bench/datasets/cua-bench-kicad/733d278a/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/datasets/cua-bench-kicad/733d278a/pyproject.toml) | `af7f493738f3e70d1b05d1e34a741c2c981c29ba` |
| [libs/cua-bench/datasets/cua-bench-kicad/83490c3a/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/datasets/cua-bench-kicad/83490c3a/pyproject.toml) | `0f25f4c763646519f58ff4822ccd3060831a9982` |
| [libs/cua-bench/datasets/cua-bench-kicad/84ec333e/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/datasets/cua-bench-kicad/84ec333e/pyproject.toml) | `921d2c98612740b2db826b72ee637fabc5645908` |
| [libs/cua-bench/datasets/cua-bench-kicad/854b8a80/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/datasets/cua-bench-kicad/854b8a80/pyproject.toml) | `a605e5f160fff42bb447029b8f74d8a926e8b896` |
| [libs/cua-bench/datasets/cua-bench-kicad/9ca3ef44/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/datasets/cua-bench-kicad/9ca3ef44/pyproject.toml) | `6c259db41f620d6597dbb1f514cfac6a0734bb17` |
| [libs/cua-bench/datasets/cua-bench-kicad/a6414tw4/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/datasets/cua-bench-kicad/a6414tw4/pyproject.toml) | `a20b0bfab7d6bb9ae3a83ec42bdfdd5b88fa33c3` |
| [libs/cua-bench/datasets/cua-bench-kicad/a6dd38a9/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/datasets/cua-bench-kicad/a6dd38a9/pyproject.toml) | `989f2e2c77e77b2ae4ac7c0a8ec6b507546b06b4` |
| [libs/cua-bench/datasets/cua-bench-kicad/b7148878/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/datasets/cua-bench-kicad/b7148878/pyproject.toml) | `ecda0b757283fd137e60a4d576ca2f7655015966` |
| [libs/cua-bench/datasets/cua-bench-kicad/b7e51b92/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/datasets/cua-bench-kicad/b7e51b92/pyproject.toml) | `8d98b9e42fb6dd6ea97a011b3ece78eefae1c188` |
| [libs/cua-bench/datasets/cua-bench-kicad/ba34f1c3/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/datasets/cua-bench-kicad/ba34f1c3/pyproject.toml) | `5efd138dcdbd8def0abbbe085661e55a11175781` |
| [libs/cua-bench/datasets/cua-bench-kicad/c4baee36/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/datasets/cua-bench-kicad/c4baee36/pyproject.toml) | `b4e04d9c2002951c5ea12e0cec8bfb68a41d9eeb` |
| [libs/cua-bench/datasets/cua-bench-kicad/d1c655da/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/datasets/cua-bench-kicad/d1c655da/pyproject.toml) | `c15e234850ec086eefbddb27775d9cc9ab0fafe6` |
| [libs/cua-bench/datasets/cua-bench-kicad/d5947ae7/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/datasets/cua-bench-kicad/d5947ae7/pyproject.toml) | `20d4dba72cb0773b1bdbf9b89dda46d56d247535` |
| [libs/cua-bench/datasets/cua-bench-kicad/ff6dfdde/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/datasets/cua-bench-kicad/ff6dfdde/pyproject.toml) | `342f1c2052a1e76597200b3cfc29374aeb0f6e9f` |
| [libs/cua-bench/example_tasks/basic_gui_env/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/example_tasks/basic_gui_env/pyproject.toml) | `db3d1f1374174fed0576f8a5751ba1b07892607f` |
| [libs/cua-bench/example_tasks/minesweeper_game_env/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/example_tasks/minesweeper_game_env/pyproject.toml) | `8725d5dfded2cd4d7fc6abe34f67d746d50c1e30` |
| [libs/cua-bench/example_tasks/test_click_env/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/example_tasks/test_click_env/pyproject.toml) | `fc635380b637c286d2f6a11e6270ec2f60be6672` |
| [libs/cua-bench/example_tasks/test_notepad_env/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/example_tasks/test_notepad_env/pyproject.toml) | `aac1568c41be1582529443a185b123a085876c2a` |
| [libs/cua-bench/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/pyproject.toml) | `dfaf1eda0e3fe399b6aa3164a3b1d393fb12a563` |
| [libs/cua-bench/tasks/winarena_adapter/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/tasks/winarena_adapter/pyproject.toml) | `a87cadfc36b7a18e27668f6d075c7af932e50b2b` |
| [libs/cua-bench/tasks/wordpad_env/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/tasks/wordpad_env/pyproject.toml) | `20a08ea34a5e0ed0d6641211cd36973e29b9e728` |
| [libs/cua-bench/uv.lock](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-bench/uv.lock) | `dd87ff68a709c17a6cced00ce5ca16a5f8e251d0` |
| [libs/cua-driver/compat-fixtures/apps/rust/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/compat-fixtures/apps/rust/Cargo.toml) | `6332dfb066850287d05d4cae1b2d691a9c4dc6d7` |
| [libs/cua-driver/examples/agent-sdks/package-lock.json](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/examples/agent-sdks/package-lock.json) | `b20c36ee2fab0e69353bdd0b0e3fd249cfbf1982` |
| [libs/cua-driver/examples/eve/package-lock.json](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/examples/eve/package-lock.json) | `92e79000378a3bddf0a17c1c48d7f60127ea733c` |
| [libs/cua-driver/examples/jev-use/package-lock.json](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/examples/jev-use/package-lock.json) | `7a68b855ff5115096ceaede5491dd1efa7b5a8c3` |
| [libs/cua-driver/examples/jev-use/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/examples/jev-use/pyproject.toml) | `6c929060ddfc873754aad79dc262c2a18da4ac28` |
| [libs/cua-driver/examples/jev-use/uv.lock](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/examples/jev-use/uv.lock) | `ac703cfc2404304a20af96488e49854a008c6430` |
| [libs/cua-driver/python/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/python/pyproject.toml) | `91bbbccd19a16c1faccac5ea0121fe12ef1aa5e3` |
| [libs/cua-driver/rust/Cargo.lock](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/rust/Cargo.lock) | `707ed352b39024b3e3ff855fa455d288acb4592c` |
| [libs/cua-driver/rust/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/rust/Cargo.toml) | `1013c7fa45581c885ab4afcaec2b68c119dbcae1` |
| [libs/cua-driver/rust/crates/cua-cursor-motion/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/rust/crates/cua-cursor-motion/Cargo.toml) | `2b43c7604f1dbe707738f71b3df99510564b0739` |
| [libs/cua-driver/rust/crates/cua-driver-bindgen/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/rust/crates/cua-driver-bindgen/Cargo.toml) | `10257e7b15098f5943eeb13ef269050c967a5b4f` |
| [libs/cua-driver/rust/crates/cua-driver-contract/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/rust/crates/cua-driver-contract/Cargo.toml) | `a41ae2a7769314bd97548493ed9262dd7a43be89` |
| [libs/cua-driver/rust/crates/cua-driver-core/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/rust/crates/cua-driver-core/Cargo.toml) | `de9e618d6b514fa4508b8a247a3cd66213574b8a` |
| [libs/cua-driver/rust/crates/cua-driver-e2e/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/rust/crates/cua-driver-e2e/Cargo.toml) | `d413d182aa997517fe6021ffbb4b049eef51dab1` |
| [libs/cua-driver/rust/crates/cua-driver-sdk/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/rust/crates/cua-driver-sdk/Cargo.toml) | `cdadd053eb4e28b80b47419854518b80129f92af` |
| [libs/cua-driver/rust/crates/cua-driver-testkit/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/rust/crates/cua-driver-testkit/Cargo.toml) | `617bf5df8715e5014baff54b3a4021469ea2ac1b` |
| [libs/cua-driver/rust/crates/cua-driver-uia/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/rust/crates/cua-driver-uia/Cargo.toml) | `52abca90d16966ef4abf0113471f70c01008729f` |
| [libs/cua-driver/rust/crates/cua-driver/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/rust/crates/cua-driver/Cargo.toml) | `8dc0840535a5e12aec7404b89d204eb63c2548c9` |
| [libs/cua-driver/rust/crates/cua-perception/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/rust/crates/cua-perception/Cargo.toml) | `d41c008a5a57e28cd43319a146fee7666ef48b14` |
| [libs/cua-driver/rust/crates/cursor-overlay/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/rust/crates/cursor-overlay/Cargo.toml) | `0b55e9e2b1b999ca4f9e450585611b0643c0a669` |
| [libs/cua-driver/rust/crates/cursor-theme-cli/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/rust/crates/cursor-theme-cli/Cargo.toml) | `348e0ceffc32d1829866785a89b66f97a0aa099c` |
| [libs/cua-driver/rust/crates/pip-preview/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/rust/crates/pip-preview/Cargo.toml) | `efa5f7b49d6e480a7fd703de9fa0a4f858a0d5b1` |
| [libs/cua-driver/rust/crates/platform-linux/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/rust/crates/platform-linux/Cargo.toml) | `e953271c8bb6bdb01e8be3ef498b24d91672e1af` |
| [libs/cua-driver/rust/crates/platform-macos/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/rust/crates/platform-macos/Cargo.toml) | `8d429484aa477f95a1cc6af5dde832cfc019db0e` |
| [libs/cua-driver/rust/crates/platform-windows/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/rust/crates/platform-windows/Cargo.toml) | `1d2a2ae148db9ffe7d9ee599babd4fd89ba416df` |
| [libs/cua-driver/rust/fuzz/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/rust/fuzz/Cargo.toml) | `eb325d8460ee642c83e9c0e37a3221a4b18513f0` |
| [libs/cua-driver/rust/rust-toolchain.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/rust/rust-toolchain.toml) | `a46af26fb51b5b80a7634a77f649a041c1bb61e9` |
| [libs/cua-driver/tests/fixtures/apps/cross-platform/electron/package-lock.json](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/tests/fixtures/apps/cross-platform/electron/package-lock.json) | `24db1b58e94b9151813641fc5871369efaf3263a` |
| [libs/cua-driver/tests/fixtures/apps/cross-platform/tauri/src-tauri/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/tests/fixtures/apps/cross-platform/tauri/src-tauri/Cargo.toml) | `2e8fd68cf1c98b972d094f868368d4bf3b72461e` |
| [libs/cua-driver/typescript/package-lock.json](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/typescript/package-lock.json) | `c2f569d4c24977db6f65cf534c4046e61e3517b3` |
| [libs/cua-s1/python/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-s1/python/pyproject.toml) | `824eadf1fcdf8cf7eda468acdbf020acee5f9a43` |
| [libs/cua-s1/python/uv.lock](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-s1/python/uv.lock) | `8c4ef13a85db7d58a2a41241a5ac2ab32d8cccb9` |
| [libs/cua-spacesd/Cargo.lock](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-spacesd/Cargo.lock) | `089b8babf0b9b0bb294fcbbb8d9d591ce3aab64f` |
| [libs/cua-spacesd/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-spacesd/Cargo.toml) | `a38ade6e414cbef2ecc365bd3659aa43299dc6da` |
| [libs/cua-spacesd/crates/cua-relay/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-spacesd/crates/cua-relay/Cargo.toml) | `92cce08eefe9703c02d5d84476c6932ef2c199d2` |
| [libs/cua-spacesd/crates/cua-spacesd-desktop/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-spacesd/crates/cua-spacesd-desktop/Cargo.toml) | `848832e73e1c45afdf207b8f540009a5eb4f6394` |
| [libs/cua-spacesd/crates/cua-spacesd-doctor/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-spacesd/crates/cua-spacesd-doctor/Cargo.toml) | `e86e37f24029689ef43023d86b8540f179c4be40` |
| [libs/cua-spacesd/crates/cua-spacesd-provider-api/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-spacesd/crates/cua-spacesd-provider-api/Cargo.toml) | `ac0e2dec59b8cc41e23f21a9b0c14c5672f47460` |
| [libs/cua-spacesd/crates/cua-spacesd-server/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-spacesd/crates/cua-spacesd-server/Cargo.toml) | `9b1f89f1d4e589286d2cdeb848eee57a3d7c8526` |
| [libs/cua-spacesd/crates/cua-spacesd-session/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-spacesd/crates/cua-spacesd-session/Cargo.toml) | `e66bd4ecff22bdd1026622ffacc366e57dce0bda` |
| [libs/cua-spacesd/crates/cua-spacesd-socks/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-spacesd/crates/cua-spacesd-socks/Cargo.toml) | `6a64ced3a4c240d9a3bf44e6f50437bcb8c05b92` |
| [libs/cua-spacesd/crates/cua-spacesd-teleport/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-spacesd/crates/cua-spacesd-teleport/Cargo.toml) | `1fb11259bd0bc96353ac69c137ace3b68352d0b1` |
| [libs/cua-spacesd/crates/cua-spacesd-test-apps/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-spacesd/crates/cua-spacesd-test-apps/Cargo.toml) | `743fdd349a551c57fd30262f3c2d4ac543481843` |
| [libs/cua-spacesd/crates/cua-spacesd/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-spacesd/crates/cua-spacesd/Cargo.toml) | `5b3196cbf657021bfd61947ab1a939663b8d7cd7` |
| [libs/cua-spacesd/tests/spaces-e2e/Cargo.lock](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-spacesd/tests/spaces-e2e/Cargo.lock) | `1698bab1448057f3194c0bde9d2df71026e2b318` |
| [libs/cua-spacesd/tests/spaces-e2e/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-spacesd/tests/spaces-e2e/Cargo.toml) | `202ca4887b7f74470ce0bd655291975d1d7d2048` |
| [libs/cua-spacesd/tests/spaces-e2e/rust-toolchain.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-spacesd/tests/spaces-e2e/rust-toolchain.toml) | `77640d561123b9c9b3b1c83af00f78ac291b8a7e` |
| [libs/cua-spacesd/tests/teleport-e2e/Cargo.lock](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-spacesd/tests/teleport-e2e/Cargo.lock) | `c6dbb848642044b716d93432fd5ff408209eada9` |
| [libs/cua-spacesd/tests/teleport-e2e/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-spacesd/tests/teleport-e2e/Cargo.toml) | `092e7989acf6e6dbcb6aeca224179999d54dd0cc` |
| [libs/cua/Cargo.lock](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/Cargo.lock) | `24de7d6368b88663bc2780d2c813a22ae8580900` |
| [libs/cua/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/Cargo.toml) | `bf428e22fde1f7334acb8de9fa0fbcdb27477326` |
| [libs/cua/bench/streaming/Cargo.lock](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/bench/streaming/Cargo.lock) | `d0cabb84277cdd6169178275a2588030a5509a00` |
| [libs/cua/bench/streaming/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/bench/streaming/Cargo.toml) | `dee4564b85635c592e5d0328a837a06bf53297ee` |
| [libs/cua/crates/cua-agent-setup/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-agent-setup/Cargo.toml) | `b3ccdb8f2759e9d9a6364159a708a5d25c60a0d1` |
| [libs/cua/crates/cua-agents/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-agents/Cargo.toml) | `dbef4f2ddd555f84f2879306f0732c20e5bbb0c2` |
| [libs/cua/crates/cua-auth/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-auth/Cargo.toml) | `6ca099315ee4ba53fe51e1d08afbd2e539e9b01f` |
| [libs/cua/crates/cua-bindgen/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-bindgen/Cargo.toml) | `8eaa45e1185cf0c7e21ea8da7676c310a08337c0` |
| [libs/cua/crates/cua-byoc/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-byoc/Cargo.toml) | `ec45b49a47cf7611238436cbb684efa101a90d9f` |
| [libs/cua/crates/cua-chromium-storage/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-chromium-storage/Cargo.toml) | `91f9091ec68654ff939033b3afb079b59027dfc3` |
| [libs/cua/crates/cua-cli/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-cli/Cargo.toml) | `0f1bd2948cbc2936eeef1d90380bdd4cd154560e` |
| [libs/cua/crates/cua-contrib/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-contrib/Cargo.toml) | `cb36898d4958268076c941534a66b51cc74c2f2a` |
| [libs/cua/crates/cua-daemon/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-daemon/Cargo.toml) | `5a9727cd48c1c0e3292d235c5e6735c01b498f08` |
| [libs/cua/crates/cua-disk/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-disk/Cargo.toml) | `1abdd196e53b63fa84f5ac5f2f1f065596e8ae63` |
| [libs/cua/crates/cua-fleet/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-fleet/Cargo.toml) | `4be2ae24dd6f3175fa0fc23c0804c747e09d1324` |
| [libs/cua/crates/cua-home/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-home/Cargo.toml) | `d7c83178d547af09adcd60b4c63caaf08341d155` |
| [libs/cua/crates/cua-host/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-host/Cargo.toml) | `aded14acb25797843fa2c8331a0e908d3edf62c1` |
| [libs/cua/crates/cua-hotspot/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-hotspot/Cargo.toml) | `85bab260aaff894a63603d4e1e87dd8e165eb2df` |
| [libs/cua/crates/cua-icon-cache/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-icon-cache/Cargo.toml) | `47c9f9d98e12cd9a60545365eeab4be64eeeb08f` |
| [libs/cua/crates/cua-image/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-image/Cargo.toml) | `1f62be9643bdfaeebba5db5143a7c10baa3a85b5` |
| [libs/cua/crates/cua-keyvault/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-keyvault/Cargo.toml) | `1350ec53194ca8d775dee47c1d2c4ce62ab6ede0` |
| [libs/cua/crates/cua-logging/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-logging/Cargo.toml) | `5e52b38d18bacbd9adc15141a4823ab37f9355ce` |
| [libs/cua/crates/cua-machine-seal/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-machine-seal/Cargo.toml) | `2b663e3aeba768767a48c00402407ff223e39dc6` |
| [libs/cua/crates/cua-media-client/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-media-client/Cargo.toml) | `98235451ef17e452e7911031d941e7e22ae470f2` |
| [libs/cua/crates/cua-media-codec/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-media-codec/Cargo.toml) | `d94bab2e38b3e014ce9f6295ba1761104d3edbd4` |
| [libs/cua/crates/cua-media-protocol/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-media-protocol/Cargo.toml) | `99be1960460cea3e1149ff5caada0fd1df6974b5` |
| [libs/cua/crates/cua-media-transport/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-media-transport/Cargo.toml) | `88f28cbeff5f087970ea9555e21f3fb3106f4925` |
| [libs/cua/crates/cua-mock-llm/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-mock-llm/Cargo.toml) | `0f57b0a4596c841d6b82e54039b7a6d5a865c13b` |
| [libs/cua/crates/cua-proto/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-proto/Cargo.toml) | `e6c6865c33448376dc0b8570f56c74c1e0bd256a` |
| [libs/cua/crates/cua-sandbox-core/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-sandbox-core/Cargo.toml) | `f0bd75cf8448e9664876de74eb9db38cc6597fc6` |
| [libs/cua/crates/cua-sdk/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-sdk/Cargo.toml) | `baa8f667ec2a25de9582d3f415568ebea404ab5a` |
| [libs/cua/crates/cua-spaces-app-core/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-spaces-app-core/Cargo.toml) | `1844a00a046ca090c4667420aa89b5503fc1f235` |
| [libs/cua/crates/cua-spaces-cli/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-spaces-cli/Cargo.toml) | `cf7de53b1c862a48314bd840476ce229a614d67a` |
| [libs/cua/crates/cua-spaces-contract/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-spaces-contract/Cargo.toml) | `c8016abdc43f827308c6ccb053fef665dbbf555a` |
| [libs/cua/crates/cua-spaces-ext/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-spaces-ext/Cargo.toml) | `f2ab1deb45634f5e4088bce886a26883515dc934` |
| [libs/cua/crates/cua-spaces-ffi/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-spaces-ffi/Cargo.toml) | `e5c5676031faff555a384fa6c397e5fbb76d4bc3` |
| [libs/cua/crates/cua-spaces-transcript/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-spaces-transcript/Cargo.toml) | `281e8794c9399de915ea49cde1e85aa4fe1b9b93` |
| [libs/cua/crates/cua-spaces/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-spaces/Cargo.toml) | `11ad4981ce3c5b733bd8f9e024b067829c95545a` |
| [libs/cua/crates/cua-spacesd-client/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-spacesd-client/Cargo.toml) | `f9177787fd3aea81333c682bf0607de04417c164` |
| [libs/cua/crates/cua-spacesd-html5/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-spacesd-html5/Cargo.toml) | `d9c5405d021df047014ba9e9d23ec9244b03d6f6` |
| [libs/cua/crates/cua-spacesd-html5/web/pnpm-lock.yaml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-spacesd-html5/web/pnpm-lock.yaml) | `ac8f84fc88dee888335014927b509ad762a87f69` |
| [libs/cua/crates/cua-telemetry/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-telemetry/Cargo.toml) | `732437749deacf5ccf946ca61750f64115848630` |
| [libs/cua/crates/cua-teleport-bundle/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-teleport-bundle/Cargo.toml) | `e7ae889cf8441a57fa1d354f73cf089b4cec138c` |
| [libs/cua/crates/cua-teleport/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-teleport/Cargo.toml) | `a1117ba5e511e8fcd05fa40b9955d07f67f5e27c` |
| [libs/cua/crates/cua-viewer/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-viewer/Cargo.toml) | `460a8bd6364a30959f52cd8de2b44c11f8d58b30` |
| [libs/cua/crates/cua-vmm/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-vmm/Cargo.toml) | `05a7aaa4780484d4cf794527a77f5b00425d0ec1` |
| [libs/cua/crates/cua-volume/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/crates/cua-volume/Cargo.toml) | `16b6c316ece78aa7bb4e4e16aba728b935ff61d5` |
| [libs/cua/python/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/python/pyproject.toml) | `7b18de004e522ddcd4babda6a29fc78acdd54056` |
| [libs/cua/python/uv.lock](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/python/uv.lock) | `2cd8ca0053843432a948711739c3c0e60a26508b` |
| [libs/cua/rust-toolchain.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/rust-toolchain.toml) | `a153311fe2698f96d56577200788a7d071b2cbc5` |
| [libs/cua/typescript/package-lock.json](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/typescript/package-lock.json) | `6b91edebb7a054388b91d2d31b46f60413cdc2a8` |
| [libs/cuabot/pnpm-lock.yaml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cuabot/pnpm-lock.yaml) | `c8f0f9a8fffa69b65c598aaff0db9177de0b1f14` |
| [libs/fleet/Cargo.lock](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/fleet/Cargo.lock) | `e9bf84bdf0660f732bee5e00c3af0bf16ff667c7` |
| [libs/fleet/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/fleet/Cargo.toml) | `302f1956553243e9122b7bea33a834bed09744da` |
| [libs/fleet/bindgen-cli/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/fleet/bindgen-cli/Cargo.toml) | `bb0dcdef5e2b6b30c7762132b3be471e5cf32eb3` |
| [libs/fleet/pnpm-lock.yaml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/fleet/pnpm-lock.yaml) | `4b6d65d0983c94f05f0e0fe6bfbd1855ea5a1625` |
| [libs/fleet/rust-toolchain.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/fleet/rust-toolchain.toml) | `c10b12e6bb930e9e7df5d7876fc45fe22ed13e4f` |
| [libs/fleet/sdk-bindings/ts-uniffi-browser/package-lock.json](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/fleet/sdk-bindings/ts-uniffi-browser/package-lock.json) | `b91f5f63f4e5d7eed016da849c7f34b3c9f8653f` |
| [libs/fleet/sdk-schema/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/fleet/sdk-schema/Cargo.toml) | `cfe73164cdea054d8542400193854aa582bcd8e5` |
| [libs/fleet/sdk/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/fleet/sdk/Cargo.toml) | `1fedf8bcf8fed561faf8cf348581ec8aa6ddcc6a` |
| [libs/fleet/uniffi-builder-derive/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/fleet/uniffi-builder-derive/Cargo.toml) | `73298282d37b4a970c9eba21b91c3ac4dc65a1e0` |
| [libs/images/tools/cua-image-pack/Cargo.lock](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/images/tools/cua-image-pack/Cargo.lock) | `7054a1d5062446d24fff25f267cc44430cdc50c7` |
| [libs/images/tools/cua-image-pack/Cargo.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/images/tools/cua-image-pack/Cargo.toml) | `43d580b463b1ae932e950bd557d8a094f94a6fb7` |
| [libs/images/tools/cua-image-pack/rust-toolchain.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/images/tools/cua-image-pack/rust-toolchain.toml) | `a153311fe2698f96d56577200788a7d071b2cbc5` |
| [libs/lume/Package.resolved](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/lume/Package.resolved) | `03988e0e4b127c8aadafb8bb4e2819e4b0506332` |
| [libs/python/agent/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/python/agent/pyproject.toml) | `4bb24ffeab3b77fdf5863eb188d420bfda8e3083` |
| [libs/python/bench-ui/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/python/bench-ui/pyproject.toml) | `e5128865b0bea84e5329197230db16a9683326bc` |
| [libs/python/core/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/python/core/pyproject.toml) | `47f7c1c925aed9d1a64c946da48b9a50b7f4ade5` |
| [libs/python/cua-sandbox-apps/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/python/cua-sandbox-apps/pyproject.toml) | `1af0270003379b2475879186759e3fe615b4ff3d` |
| [libs/python/cua-sandbox-apps/uv.lock](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/python/cua-sandbox-apps/uv.lock) | `feb662098b17fcdc6f04b732878e1cd97f814e31` |
| [libs/python/cua-sandbox/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/python/cua-sandbox/pyproject.toml) | `06b4ab044ee368cce0c4d5c683bbed5d0b29043b` |
| [libs/python/cua-sandbox/uv.lock](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/python/cua-sandbox/uv.lock) | `2369d58440eaf63e04bf3cdce63389ce473c1b39` |
| [libs/python/cua-train/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/python/cua-train/pyproject.toml) | `bef5804d2a2e11f3c77ca9fdbaf5fd80412e014a` |
| [libs/python/som/pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/python/som/pyproject.toml) | `918a9c1337997d11d3042c7d37d9c6a08e8e135b` |
| [libs/typescript/pnpm-lock.yaml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/typescript/pnpm-lock.yaml) | `ae0aa4da92497c078aea9cf395225d889621c3af` |
| [pnpm-lock.yaml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/pnpm-lock.yaml) | `af0feb4892dfa03f043f00eb082ff360b92d0e7c` |
| [pyproject.toml](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/pyproject.toml) | `9103fc1b5d74533c2a043bba92c6751d5499da25` |
| [uv.lock](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/uv.lock) | `f63f7a07b5557e42daa352c97ec2b143708e1bcc` |

## Evidence limits

This pass inspected the pinned README/licensing/SDK/Driver source inventories, model/perception locks, model download callers, benchmark file catalog and Dojo result/map/Python inventory. The generated indexes use git objects directly. No dependency installation, model download, VM startup, remote API call, training run or production deployment occurred. The matrix deliberately marks unsupported integrated products as gaps. It is a coverage baseline, not a full behavioral proof.

## Public tool contracts at this revision

Generated manifests are the operation inventory authority. The SDK README describes 33 Spaces tools, while this exact generated manifest declares 86; use the manifest rather than that stale prose count. A renamed Dojo operation only counts as parity after matching inputs, outcome/error classes, authority and a corresponding fixture.

### libs/cua-driver/contract/manifest.json

Contract `0.8.0`, MCP `2025-06-18`, 29 tools, blob `fcc4a8a0a25a686d957a092c1da148752e9defda`. [Immutable manifest](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/contract/manifest.json).

| Tool | Upstream description |
|---|---|
| `click` | Click coordinates or a snapshot-bound element with an explicit target and delivery mode. |
| `clipboard_read` | List available system clipboard types and optionally return privacy-sensitive plain text. Clipboard content is never retained in telemetry. |
| `clipboard_write` | Replace the system clipboard with exactly one value: plain text, an image from an absolute local path, or a file URL from an absolute local path. Returns the available types for read-back before paste. |
| `drag` | Drag between two absolute points in get_desktop_state coordinates. |
| `end_session` | End one visible lifecycle session and run its cursor, recording, configuration, and other cleanup hooks exactly once. Omit `session` to end the authenticated transport's implicit session. Idempotent. |
| `escalate_session` | Deprecated compatibility tool for legacy capture-scope sessions. New callers select window or desktop modality on each action. No deescalate_session tool exists. |
| `get_agent_cursor_state` | Return the session cursor's theme, semantic playback, position, visibility, and motion. |
| `get_cursor_position` | Return the OS cursor position when the platform can observe it. |
| `get_desktop_state` | Capture the complete primary display at native resolution for a desktop-scope GUI loop. |
| `get_screen_size` | Return the primary display dimensions and scale factor in the platform's desktop coordinate space. |
| `get_session` | Read content-free lifecycle, cursor, recording, and idle status for one session visible to this authenticated transport. Omit `session` to inspect its implicit session. |
| `get_session_state` | Deprecated compatibility alias that reads a live legacy session's capture policy. Use get_session for lifecycle state. |
| `get_window_state` | Observe an exact native window with snapshot-bound elements and optional screenshots. |
| `hotkey` | Press a key chord in the foreground desktop application. |
| `invoke_menu` | Resolve an exact application-menu path one live native level at a time and invoke its final item through accessibility APIs. Missing, ambiguous, disabled, or structurally mismatched segments fail closed; this tool never falls back to pixels. |
| `list_apps` | Discover installed and running native applications. |
| `list_sessions` | List content-free lifecycle summaries attached to this authenticated transport lease. It does not enumerate other callers' sessions. |
| `list_windows` | Discover exact native windows and observable bounds and stacking order. |
| `move_cursor` | Move the real OS pointer in get_desktop_state coordinates. |
| `parse_visual_regions` | Parse one immutable registered capture into model-neutral text and icon regions. |
| `press_key` | Press one key, with optional modifiers, in the foreground desktop application. |
| `scroll` | Scroll at an absolute point in get_desktop_state coordinates. |
| `set_agent_cursor_enabled` | Show or hide the agent cursor owned by a session. |
| `set_agent_cursor_motion` | Configure the movement style, timing, effects and visibility timing for a session cursor. |
| `set_agent_cursor_theme` | Select an already-installed cursor theme for a session. |
| `set_window_frame` | Set one exact top-level window's frame in the desktop-coordinate space reported by list_windows and verify the resulting geometry through an independent readback. |
| `start_session` | Optionally create or return a lifecycle session before acting. For multi-call work, prefer a short public `session` label and repeat it on every call that accepts it; an omitted value uses the authenticated transport lease's implicit session instead. This tool is optional because an ordinary action can create or reuse a named run directly. Use it to set the initial cursor theme before acting or to revive a public name after it has ended; ordinary actions never revive ended names. `capture_scope` is deprecated compatibility input; new callers select window or desktop modality per action. Idempotent. |
| `type_text` | Type text into the current foreground desktop application. |
| `verify_state` | Deterministically verify bounded predicates against one exact window. The driver evaluates structured window/accessibility state and may return the final screenshot as uninterpreted visual evidence for a multimodal caller. Predicate results are satisfied, unsatisfied, or unknown; unknown never implies success. Accessibility projections are conservative: absence remains unknown unless the observed search domain is proven exhaustive. |
### libs/cua/spaces-contract/manifest.json

Contract `0.7.0`, MCP `2025-06-18`, 86 tools, blob `15f430e9c23b311c3701fd16a51941db5a816814`. [Immutable manifest](https://github.com/trycua/cua/blob/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua/spaces-contract/manifest.json).

| Tool | Upstream description |
|---|---|
| `add_space` | Add an existing machine as a Space by address and token. |
| `remove_space` | Forget a registered Space without touching it. |
| `list_spaces` | List the Spaces this connection can see. |
| `create_space` | Create a Space: a new sandbox on this machine, on one of your machines, or in the cloud. |
| `delete_space` | Delete a Space's sandbox and forget it. |
| `stop_space` | Turn a Space off: suspend it (memory kept) or stop it (disk kept). |
| `start_space` | Turn a Space back on: resume or boot it. |
| `space_bash` | Run a shell command inside the Space. |
| `space_write` | Write literal text to a path inside the Space. |
| `upload` | Send a local file or folder into the Space. |
| `send_file` | Send a host file or folder into the Space's Downloads, verified by sha256 on arrival. |
| `download` | Fetch a file or folder out of the Space. |
| `stream_endpoint` | Return a ticketed media endpoint for the Space's desktop or one window. |
| `list_space_windows` | List the windows open inside the Space. |
| `stream_space_window` | Draw one Space window on the operator's desktop. |
| `show_space_pip` | Pin the Space as picture-in-picture on the operator's desktop. |
| `hide_space_pip` | Unpin the picture-in-picture window. |
| `open_space_viewer` | Open the full Space viewer on the operator's desktop. |
| `list_tools` | List the MCP services and tools reachable inside the Space. |
| `call_tool` | Call a service tool inside the Space. |
| `agent_start` | Start a coding agent in the Space. |
| `agent_message` | Send a follow-up to an agent run. |
| `agent_events` | Read an agent run's events after a cursor. |
| `agent_status` | Read an agent run's status and result. |
| `agent_interrupt` | Interrupt an agent run's current turn. |
| `agent_stop` | Stop an agent run. |
| `agent_list` | List agent runs in the Space. |
| `agent_capabilities` | Report what the agent harnesses support. |
| `persistent_agent_create` | Create a named agent whose memory outlives its Space. |
| `persistent_agent_list` | List persistent agents. |
| `persistent_agent_remove` | Forget a persistent agent. |
| `persistent_agent_send` | Give a persistent agent a turn. |
| `persistent_agent_save` | Save a persistent agent's home now. |
| `agent_pause` | Pause a persistent agent: its run, its routines and its Space. |
| `agent_resume` | Resume a paused persistent agent. |
| `routine_add` | Schedule a recurring turn for a persistent agent. |
| `routine_list` | List routines. |
| `routine_remove` | Delete a routine. |
| `routine_set_enabled` | Turn a routine on or off. |
| `notify_user` | Tell the user something in the Cua app. |
| `notifications_list` | Read the notifications feed. |
| `notifications_ack` | Mark notifications read. |
| `computer_access_grant` | Let a persistent agent use one of the user's computers. |
| `computer_access_revoke` | Take back a persistent agent's access to a computer. |
| `computer_access_list` | List per-agent computer access. |
| `teleport_manifest` | Describe what a session teleport would move. |
| `teleport_app` | Move an app session into the Space. |
| `request_site_login` | Sign in to a site in the Space with the user's saved password. |
| `hotspot_start` | Start sharing this machine's network with a Space. |
| `hotspot_stop` | Stop sharing this machine's network. |
| `hotspot_status` | Report which Spaces are sharing this machine's network, with counters. |
| `volume_ls` | List a folder of the Cua Volume. |
| `volume_read` | Read a file from the Cua Volume. |
| `volume_write` | Write a file to the Cua Volume. |
| `volume_delete` | Delete a file from the Cua Volume. |
| `volume_history` | List a Cua Volume file's versions. |
| `volume_restore` | Restore an old version of a Cua Volume file. |
| `volume_grant` | Grant an agent or Space more Cua Volume access. |
| `volume_revoke` | Revoke a Cua Volume grant. |
| `volume_grants` | List Cua Volume grants. |
| `volume_request_access` | Ask the user for more Cua Volume access. |
| `volume_requests` | List Cua Volume access requests waiting for the user. |
| `volume_approve` | Approve a Cua Volume access request. |
| `volume_deny` | Decline a Cua Volume access request. |
| `volume_audit` | Read the Cua Volume audit log. |
| `volume_storage` | Show where the Cua Volume keeps its bytes. |
| `volume_storage_set` | Test or change where the Cua Volume keeps its bytes. |
| `volume_mount_status` | Show whether the Cua Volume is mounted as a volume. |
| `volume_mount` | Mount the Cua Volume as a volume. |
| `volume_unmount` | Unmount the Cua Volume volume. |
| `volume_sync_status` | Show the Cua Volume's sync state across devices. |
| `volume_sync_events` | Wait for Cua Volume sync events. |
| `volume_sync_resolve` | Mark a Cua Volume sync conflict as seen. |
| `volume_cache_stats` | Show the Cua Volume block cache. |
| `volume_cache_set` | Set the Cua Volume block cache's size cap. |
| `volume_cache_clear` | Clear the Cua Volume block cache. |
| `share_space` | Let another cua.ai account watch or edit a Space. |
| `unshare_space` | Stop sharing a Space with an account, or with everyone. |
| `space_shares` | List who a Space is shared with. |
| `relay_register_space` | Publish a Space on the relay for the account's other devices. |
| `relay_unregister_space` | Take a Space off the relay. |
| `cloud_status` | List your clouds (AWS, Google Cloud, Modal): found credentials, the connected account and region, what each can run at what hourly cost, and what Cua created there. |
| `cloud_connect` | Connect a cloud account Spaces can be created in. |
| `cloud_test` | Check a cloud account without creating anything. |
| `cloud_disconnect` | Forget a connected cloud account. |
| `cloud_sweep` | Find and delete what Cua left in your clouds. |
