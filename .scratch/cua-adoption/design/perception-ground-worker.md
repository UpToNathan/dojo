# Pinned perception worker ground findings

Source revision is `88aff7e5424e9ed45b93ded85eb4111472ca5190`. All upstream paths below are relative to `~/.cache/dojo/upstream/cua/88aff7e5424e9ed45b93ded85eb4111472ca5190/libs/cua-driver/rust/`. These are source observations. No model download, executable launch, or inference verification was performed in this exploration. The how explorer instructions and unslop skill informed this report.

## Components found

| Component | Path and symbol | Responsibility |
| --- | --- | --- |
| Public contract | `crates/cua-driver-contract/src/visual.rs`: `ParseVisualRegionsInput`, `ParseVisualRegionsOutput`, `VisualCaptureProvenance`, `VisualActionCoordinateSpace`, `VisualParserMetadata` | Defines capture-bound parsing, image pixel bounds, model identity, options and errors. |
| Host tool | `crates/cua-driver-core/src/perception_tools.rs:33`: `register_perception_tool`, `ParseVisualRegionsTool::invoke` | Resolves a retained capture under its binding, invokes the worker and validates canonical output. |
| Host process client | `crates/cua-driver-core/src/perception_client.rs:46`: `PerceptionWorkerConfig`, `PerceptionClient`, `PerceptionClientHandle`, `WarmWorker` | Launches a contained process, bounds IPC and deadlines, checks response and extension identity, and cleans up. |
| Linux containment | `crates/cua-driver-core/src/perception_client/containment.rs`, `containment/linux.rs`, `containment/unix.rs` | Applies filesystem, network, process, resource and lifecycle boundaries before worker execution. |
| Worker protocol | `crates/cua-perception/src/lib.rs:184`: `Worker`, `Request`, `Command`, `read_frame`, `write_frame` | Handles framed health, self_test and parse commands with explicit fixture or ONNX backend. |
| Artifact validator | `crates/cua-perception/src/manifest.rs:160`: `ValidatedManifest::load`, `ModelManifest::validate_fields` | Validates local manifest fields, target and artifact hashes before runtime initialization. |
| Inference | `crates/cua-perception/src/runtime.rs:63`: `OnnxBackend::load`, `parse`, `recognize`, `load_session`, `verify_io`, `verify_recognizer_io` | Loads three ONNX sessions, runs icon detection and OCR, restores geometry and emits regions. |
| Image transforms | `crates/cua-perception/src/preprocess.rs`: `Letterbox`, `detector_tensor`, `ocr_detector_tensor`, `ocr_recognizer_tensor` | Converts decoded RGB images to each model's tensor and retains model-to-image transform data. |
| Output decoding | `crates/cua-perception/src/postprocess.rs`: `decode_detector`, `decode_ocr_probability_map`, `ctc_decode` | Performs class score selection, NMS, text component extraction and CTC recognition. |
| Worker executable | `crates/cua-perception/src/main.rs`: `parse_startup`, `run`, `run_gate` | Serves framed stdio or runs installed health/inference/mismatch gates. |
| Artifact authority | `crates/cua-perception/scripts/artifacts.lock.json`, `models/model-manifest.template.json` | Pins reviewed artifacts and inference configuration. |

## Flow

1. `ParseVisualRegionsTool::invoke` resolves the caller's `CaptureBinding`, parses and validates `ParseVisualRegionsInput`, parses its capture ID, and calls `CaptureService::read_for_perception`. The request references retained pixels rather than accepting a file path or caller-provided image. Capture lookup errors preserve unknown, expired and generation mismatch distinctions.
2. The tool reads the retained encoded dimensions and PNG bytes and calls `PerceptionClient::parse`. Its cancellation argument is a newly created default token. This tool does not wire an external request cancellation token itself.
3. Cold mode launches `WarmWorker::launch`, whose containment creates a fresh private temporary working directory and a process with stdio pipes. It sends health first. The client checks ready, protocol and expected extension identity before sending parse.
4. The wire format is four big-endian length bytes followed by UTF-8 JSON. Protocol is `cua-perception/1`. A request has `protocol`, `request_id`, `method` and `params`. Parse params contain `capture_id` and `image` with `media_type`, `width`, `height`, `byte_length` and `data_base64`. There is no transform, options, deadline or cancellation command in the worker protocol.
5. Startup requires `--manifest <path> --onnx-runtime-library <path> --extension-id <id> --extension-version <version>`. `Worker::from_manifest_with_extension_identity` validates extension identity, loads and hashes the manifest's model/dictionary artifacts and runtime, then calls `OnnxBackend::load`. Runtime version is read through `OrtGetApiBase` and checked before ort initializes with telemetry disabled.
6. `Worker::handle_payload` validates JSON shape, protocol and request ID. `validate_image` verifies declared length, base64 length, media type, decoded dimensions and allocation bounds. SHA-256 is computed over exact encoded input bytes, not RGB pixels. The worker accepts PNG and JPEG; the capture-bound host always sends PNG.
7. `OnnxBackend::parse` runs the fixed icon detector first. `detector_tensor` resizes with triangle filtering, centers the image in a gray 114 canvas, converts RGB to float32 CHW in [0,1], and records actual independent X/Y scales. The decoder selects class scores, sorts and truncates candidates, applies class-agnostic NMS, and restores boxes to source image pixels.
8. OCR detection uses BGR CHW with ImageNet mean/std and longest-edge resize rounded to multiples of 128. The manifest input width supplies the resize length; its OCR detector input height is validated but does not set this tensor's height. Probability-map connected components become expanded axis-aligned rectangles. Recognition crops those rectangles, produces normalized BGR 48 by 320 tensors with gray padding, and performs CTC decoding against the hashed dictionary. Text confidence multiplies detection and recognition confidence. Empty or whitespace-only results are dropped.
9. The worker sorts regions by Y, X and kind. `inference_result` floors left/top and ceils right/bottom, clamps bounds, drops degenerate/blank regions, assigns local IDs such as `text-1`, labels icons with `icon-class-0`, and sets `interactive: false`. The output echoes capture ID, encoded-byte digest, dimensions, `coordinate_space: image_pixels`, runtime and model identity.
10. The host checks response protocol and request ID plus expected installed extension ID/version. `build_output_with_provenance` checks any worker image echo against retained digest/dimensions, derives parser metadata, and uses the host's capture provenance. It applies requested kinds, minimum confidence and maximum region count after inference. Filtering also clears parent/group references to removed regions. `ParseVisualRegionsOutput::validate` rejects invalid regions or provenance before returning structured `cua.visual_regions_v1`.
11. Cold mode closes stdin and awaits successful worker exit before returning. Cancellation or timeout drops the owned worker; the process guard kills and reaps it. Warm mode serializes requests through an async mutex, reuses sessions, and shuts down after idle TTL or failures.

## Boundaries

### Capture identity and coordinates

The public input is capture ID plus options. The output includes exact screenshot identity and a six-coefficient affine mapping or identity mapping to action coordinates. `capture_provenance` in `perception_tools.rs:403` derives that mapping from the retained capture. The worker only returns source screenshot pixels. `VisualRegionBounds::center` preserves half-pixel centers, and `VisualRegion::action_center` maps them without rounding.

The production worker identity includes model name/version/source revision/manifest SHA-256, ONNX runtime version/library SHA-256/thread count, and extension ID/version. `worker_parser_metadata` requires extension ID `cua-perception`. The IPC client verifies installed extension ID/version against caller configuration. Model/runtime hashes in output are structurally checked by output normalization; their expected values are not compared to a host artifact allowlist at that stage. Trust comes from the separately verified installation and worker startup validation.

### Artifact validation and pins

`ValidatedManifest::load` reads schema 1 JSON with unknown fields denied. It requires nonempty identity and IO names, model dimensions up to 4096, thresholds in [0,1], nonzero candidate limits and 1..64 intra threads. Runtime target must equal the current synthesized target such as `x86_64-unknown-linux-gnu`. Artifact paths must contain only normal relative components. Final artifact entries must be nonempty regular files according to `symlink_metadata`; SHA-256 comparison ignores hex case.

These checks do not validate every ancestor against symlinks. They hash files and then open their paths again for library/session/dictionary loading. The worker does not own an immutable handle or verified byte buffer across that sequence. Manifest bytes themselves are hashed for identity but no expected manifest hash is supplied to this constructor. Adoption needs a trusted manifest source and immutable staging to close substitution/race gaps.

The crate version is 0.2.1. Workspace Rust license is MIT. Dependencies include `ort = =2.0.0-rc.10` with only std/load-dynamic, `image 0.25` with png/jpeg and no default features, `libloading 0.8`, base64, serde/JSON/YAML, sha2 and thiserror. Cargo.lock supplies resolved Rust dependency versions. ONNX runtime is an explicitly supplied CPU dynamic library, not a bundled or downloaded Rust dependency.

The artifact lock records these reviewed inputs:

| Input | SHA-256 | License recorded upstream |
| --- | --- | --- |
| `omniparser-icon-detect-1280-opset17.onnx`, 80,933,219 bytes | `d8a876bf7f9fb73d7da9432904ade7fa78e092e9a91674e5a2806b45562a9ab2` | AGPL-3.0-only |
| `ppocrv5-mobile-det.onnx`, 4,826,518 bytes | `a431985659dc921974177a95adcfbb90fd9e51989a5e04d70d0b75f597b6e61d` | Apache-2.0 |
| `ppocrv5-mobile-rec-en.onnx`, 7,848,423 bytes | `b5f833dfc5d0eb71da397b4efa06ebeee9b431b690a47d6af40d77d8eabc557f` | Apache-2.0 |
| `ppocrv5-mobile-rec-en-inference.yml`, 3,964 bytes | `27e91d0582f40168aa218303c76e184bc78fa7a5d105aad0cfbad8458b441067` | Apache-2.0 |
| Linux x64 `libonnxruntime.so`, 23,023,576 bytes, version 1.26.0 | `5bd5bedf736fc501692435d0ec4f6e8b2bdf48cd30af8e6d00d61b3ddc9a7ab8` | MIT with runtime third-party notices |

Linux runtime archive is pinned at `https://github.com/microsoft/onnxruntime/releases/download/v1.26.0/onnxruntime-linux-x64-1.26.0.tgz`, archive digest `1254da24fb389cf39dc0ff3451ab48301740ffbfcbaf646849df92f80ee92c57`, member `onnxruntime-linux-x64-1.26.0/lib/libonnxruntime.so.1.26.0`. Only Linux x64 has a reviewed Linux target in this lock. The worker can synthesize a Linux arm64 target, but this lock has no matching reviewed runtime artifact.

OmniParser source revision is `6600256cb0f1b07651e3bc86166196307bad7e2d`. Detector ONNX has `url: null` and must be supplied locally as the exact reviewed conversion. OCR detector revision is `e6f4fa85f00e168c862bc462aebca69eef9b3d3d`; recognizer/dictionary revision is `3fafbc3b5dcf93dd72add9f48368be8a3a2cd33b`. Their immutable Hugging Face download URLs are in the lock. Model lock entries say `release-verified`; that declaration is not proof that these bytes are installed on this Dojo machine. No ONNX weights or ONNX runtime library were found in the pinned upstream checkout.

The model ledger records the conversion toolchain, source model hash and expected conversion output. Production Rust needs no Python. Upstream assembly, verification, export and quality tooling are Python scripts. A no-production-Python adoption can preserve those provenance records and implement equivalent build-time acquisition/assembly/verification in Rust or Nix. It must obtain the exact converted detector or reproduce its digest. The accompanying source offer includes source model, pinned exporter snapshots, recipe, notices and license texts. The full worker source is MIT; a package with detector weights has separate licenses.

### Lifecycle and limits

Worker limits are 16 MiB frames, 8 MiB encoded images, 8192 per dimension, 32 Mi pixels and decoder allocation budget 128 MiB. Host frame limits are derived independently from capture registry byte limits plus base64 expansion and metadata. Worker frame reading treats an empty incoming frame as an empty JSON payload and continues; host response reading rejects zero length.

Default cold request deadline is 30 seconds including launch, model validation/loading, health, parse and exit wait. Warm defaults are 15 seconds startup, 30 seconds inference, 2 seconds shutdown and 30 seconds idle TTL. There is no automatic replay. Errors preserve timeout, cancellation, launch, crash, incompatible protocol, invalid artifact and resource-limit categories.

Containment defaults are 16 GiB virtual address space on Linux, 15 minutes accumulated CPU time, one process and 256 open files. Linux uses Landlock ABI at least 3, seccomp, `PR_SET_NO_NEW_PRIVS`, `PR_SET_PDEATHSIG`, resource limits, a process group and descriptor close-on-exec. Environment is cleared except private TMPDIR/TMP/TEMP. Stderr is discarded in the contained Unix command. Filesystem reads come from canonical executable/absolute argument roots and fixed system loader/CPU metadata paths; writes are confined to the private work directory. Seccomp blocks sockets, io_uring, process creation, desktop IPC and cross-process access, while allowing runtime threads. There is no desktop capture/action authority inside inference.

The Linux system loader allowlist includes `/lib`, `/lib64`, `/usr/lib*` and `/usr/libexec`, but not `/nix/store`. Dojo's Nix-linked executable/runtime closure needs exact dependency read and interpreter execute grants or a suitable self-contained packaging approach. Additional readable roots alone do not grant execution. Broadening access to the entire store would change the upstream boundary.

`shutdown_now` increments a shared cancellation epoch. Warm parsing selects that epoch and cancels active work. Cold parsing selects only explicit token cancellation and its request timeout; the epoch is not observed there. Therefore the session-end hook's stated cancellation does not cancel an already active cold request through this mechanism. A Dojo host adapter should wire actual request/session cancellation into cold calls before claiming parity for cancellation.

## Non-obvious things

- The worker is a complete Rust real-inference implementation. `Worker::fixture()` and top-level `handle_payload` compatibility helper are deterministic fixture helpers that accept one exact 1x1 PNG digest. Fixture health says `fixture_only` and `parse_fixture`; it cannot prove general image inference.
- Actual inference exposes icon boxes with generic class labels and OCR text. Every region is noninteractive. It does not infer button semantics, names for icon actions, groups, parent links or reading-order fields. Those fields exist in the contract for other backends. Pinned worker parity should retain this limitation.
- OCR uses axis-aligned bounds and crops. It does not perspective-correct rotated text. Health/parse identity declares the limitation, but host `VisualParserMetadata` does not retain its freeform `limitations` field.
- `self_test` runs synthetic white-image inference. The ignored real-model test asserts eight regions and known text on a separately supplied UI fixture. The CLI real-parse gate requires success, while stronger expectations live in tests and bundle tooling. A success response alone is weaker evidence than known-answer region verification.
- `validate_worker_echo` permits missing capture ID and image fields. When image is present, digest/dimensions must match. For worker output that already decodes as canonical output, host provenance replaces worker provenance. A new host boundary can require echoes from this fixed worker because it always supplies them.
- Options do not change inference costs. Full inference runs first, then the host filters/truncates.
- Separate cold workers avoid global ort initialization conflicts across differing manifests or runtime libraries. Session mutexes are local inference synchronization, not durable scheduling.

## Adoption scope and precise gaps

Whole `cua-perception` Rust sources and its workspace dependency pins can be adopted with MIT notices. Its protocol, manifest validator, preprocess/postprocess and real CPU inference can stay intact. The transport-free visual contract can be reused. The core worker client and Linux containment are substantive implementation worth reusing rather than replacing with an unbounded child process.

The host tool itself depends on CUA's `CaptureService`, capture registry, binding resolution, tool registry and session-end hooks. Dojo should adapt it to its existing capture authority and Pi tool dispatch instead of importing a second capture registry, durable scheduler or approval state. This report proposes no changes to durable approvals.

Actual parity requires separately supplied exact model/runtime artifacts, a Dojo package/manifest trust path, corresponding source/notices, a bounded worker adapter with real cancellation, a Linux/Nix containment dependency closure, normalized output tied to retained Dojo image/digest/transform/options, and execution of the real known-answer and live screenshot paths. None of those operational facts is proved merely by adopting source or passing fixture tests.

## Files read

Dojo `GLOSSARY.md` and `docs/agents/domain.md`; how explorer prompt and unslop skill. Upstream files: Rust workspace `Cargo.toml`; `cua-perception/Cargo.toml`; all six `cua-perception/src/*.rs`; `cua-perception/tests/real_models.rs`; `cua-perception/models/model-manifest.template.json`, `MODEL_LEDGER.md`, `THIRD_PARTY_NOTICES.md`, `SOURCE_OFFER.md`; `cua-perception/scripts/artifacts.lock.json` and `README.md`; `cua-driver-contract/src/visual.rs`; `cua-driver-core/src/perception_tools.rs`, `perception_client.rs`, `perception_client/containment.rs`, `containment/linux.rs`, `containment/unix.rs`. Large source files were read in relevant implementation ranges, rather than all test bodies.

## Open questions

- Where are the exact reviewed OmniParser ONNX conversion and a sealed Linux bundle obtainable for this adoption? The lock has no detector download URL and the pinned checkout contains no weights/runtime.
- Can a Nix package preserve the pinned runtime digest while satisfying that runtime's dynamic dependencies? PatchElf would change the pinned library bytes, so any required patch needs a distinct reviewed artifact identity.
- What exact capture cancellation and output types does the existing Dojo capture authority provide? The separate Dojo explorer owns this question.
- Which evidence will establish live text/icon inference on this Linux host, including artifact identity and canceled worker cleanup? This exploration did not run it.
