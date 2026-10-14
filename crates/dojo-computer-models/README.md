# Rust computer-action decoding

This is a pure Rust translation of selected CUA model-output behavior at
`88aff7e5424e9ed45b93ded85eb4111472ca5190`. It performs no desktop input,
provider requests, scheduling, code evaluation, or credential access.
Pi remains the durable run owner. A provider bridge must create and persist a
Pi tool intent before any decoded action may execute.

`decode(profile, raw, model_view)` returns one explicit action, done, ask-human,
or refusal outcome, retaining the source response, profile/parser version,
call ID and observation binding. The caller supplies preprocessing dimensions
and the model-to-delivered-image transform. Rust preserves fractional delivered
pixels; the capture authority performs desktop mapping and admission separately.

Supported parsing profiles:

- Canonical JSON with `action`, optional `call_id`/`thought`, action-specific
  fields, and coordinates in model pixels.
- UI-TARS `Thought: ... Action: click(start_box='(x,y)')` and related quoted
  literal forms. Box centers use normalized 0–1000 coordinates and floor.
- Qwen3.5 JSON or parameter-form `tool_call` envelopes for `computer`, using
  normalized 0–1000 coordinates and ties-to-even rounding.
- Generic VLM JSON `tool_call` envelopes with the same coordinate convention.

The port follows `libs/python/agent/cua_agent/responses.py`,
`loops/uitars.py:parse_uitars_response/convert_to_computer_actions`, and
`loops/qwen35.py` / `loops/generic_vlm.py` tool parsers and normalization.
The upstream MIT notice is in `LICENSE.md`. This is translated code, not an
unchanged source import. Upstream fixtures informed literal golden cases; tests
and production builds require no Python.

Deliberate differences:

- Duplicate JSON/XML fields, multiple calls, missing coordinates, bool/nonfinite
  numbers, oversized values, unknown fields and malformed outputs refuse.
- UI-TARS accepts a restricted literal grammar. Attribute calls, expressions,
  nonliteral arguments and arbitrary code are never evaluated.
- Terminal, ask-human and refusal outcomes are distinct. They do not authorize
  actions or resolve a durable human request.
- Targeted typing and drag paths are retained. Qwen's underspecified
  `left_click_drag` and modified pointer actions refuse rather than inventing a
  start point or discarding modifiers. Triple clicks refuse instead of becoming
  double clicks. Unsupported extra parameters also refuse.
- Canonical scroll uses positive right/down, with explicit pixels versus wheel
  steps. Qwen documented positive up/left values are converted to negative
  canonical deltas; UI-TARS directions map explicitly to five wheel steps.
- Wait duration is preserved and bounded to 30 seconds. No safety acknowledgment
  is automatic. No stale image lookup or description-only grounding cache exists.

The caller must independently revalidate the model-view record against a live
session/capture, capabilities, target identity, human ownership and freshness.
A parsed ID is not an authorization token. Native provider wire formats,
request-image preprocessing, Pi provider adaptation, inference, S1 policy,
retention and durable ask-human workflows remain separate planned work.

```sh
devenv shell -- cargo test --manifest-path crates/dojo-computer-models/Cargo.toml
```

## Additional portable policies and profiles

`Anthropic`, `OpenAiNative` and `Gemini` decode one bounded native block and retain
its complete source block in `DecodedStep.provider_metadata`, including call IDs,
safety checks and thought signatures. These are decoder capabilities. Enabling
native Pi transport still requires native wire, replay and subscription evidence.
No safety check is acknowledged by decoding. Anthropic enhanced held-input,
endpoint-only drag and Gemini type-plus-enter compound calls are explicit
unsupported errors. Native browser scripts/navigation remain outside this decoder.
Anthropic scroll uses wheel steps; Gemini scroll uses pixels. Per-axis restoration
uses the supplied model view. Full original coordinates remain in provider metadata.

`UiTars2` accepts a single restricted seed function/parameter block. `Fara` accepts
its `computer_use` JSON tool wrapper and normalized points. `OpenCua` accepts the
whole literal `click(x=...,y=...)` form with optional `pyautogui.` prefix, using
processed-image pixels. Multiple functions, duplicate parameters, arbitrary code,
unknown action fields and unavailable start coordinates are refused.

`preprocessing::plan` ports area resize, factor resize and maximum-size fit sizing.
It returns exact dimensions and per-axis restoration metadata. The caller records
its actual image resampler; this module does not claim Pillow image-byte parity.
`policy` supplies immutable capture/session/view/model/description grounding keys,
explicit failed-grounding entries, checked integer usage reservations and image
retention positions. Pi owns persistence, usage accounting and request projection.
Zero image retention keeps no images; durable messages and signatures are preserved.

`s1` ports entity options, Unicode character context truncation, exact token plan
validation, confidence ordering and at most one explicitly selected submit.
`mutation` refuses unknown checkbox state or unadvertised value replacement.
`validate_refresh` requires a new complete snapshot of the same process/window,
session and runtime and stops on unconfirmed effect. Unlike upstream index/label
fallback, changed tokens require replanning. These functions emit policy data and
never grant authorization or dispatch effects. Byte collation ports UTF-8 byte+1
encoding, zero padding and context/option masks without Torch or model weights.
Allocations are bounded. S1 source is MIT under `libs/cua-s1/python/LICENSE`;
weights, adapters and datasets are separate unimported artifacts.

Expanded validation: `devenv shell -- cargo test --manifest-path
crates/dojo-computer-models/Cargo.toml`.

The S1 distribution validator also checks each finite probability, the option
count and the distribution sum. Upstream validates decision confidence but leaves
these distribution checks to its backend. Rust performs them at the planner
boundary. Tensor scoring and weights remain outside this crate.

S1 snapshot binding includes the PID, process start time, exact window and runtime
and session generations. Planner role filtering removes underscores and spaces
from observer role names. Submit labels follow upstream punctuation normalization,
so `Submit form!` remains a recognized submit control.
