# Python-to-Rust port inventory

Source-inspected at CUA `88aff7e5424e9ed45b93ded85eb4111472ca5190` and locally installed pi-ai/pi-durable 1.1.0. This is source evidence, not live provider/inference verification. Implements the scope of [ticket 08](issues/08-python-models-to-rust.md), [11](issues/11-recording-rust-bench.md) and [14](issues/14-rust-specialist-inference.md).

Paths below are relative to the [pinned upstream tree](https://github.com/trycua/cua/tree/88aff7e5424e9ed45b93ded85eb4111472ca5190). Prefix A = `libs/python/agent/cua_agent/`; B = `libs/cua-bench/cua_bench/`; S = `libs/cua-s1/python/src/cua_s1/`.

## 1. Canonical action codecs and execute→observe behavior

**Sources:** A `responses.py`, `computers/base.py`, `callbacks/operator_validator.py`; `agent.py:717–881` dispatch and `:884–1045` run orchestration.

Port Responses-style action/output construction, call pairing, action names and coordinate/description transformations. Pure Rust interface: `decode(dialect, raw, observation_metadata) -> normalized step or classified error`. Keep native envelopes separate from executable actions. Preserve stable call IDs, target/session/generation, explicit coordinate spaces, scroll units, waits, safety metadata and finish/ask-human/refusal outcomes. Suggested pure dependencies: serde, serde_json, thiserror, optionally a bounded parser/regex; no HTTP/Tokio/GPUI required for decoding.

Port execute→observe as one bounded Rust operation behind the shared session module—not an autonomous run. After an effect, produce fresh screenshot/semantic evidence where requested. Failure to capture after delivery reports unknown/partial effect and never retries input. Termination is a run outcome, not UI input.

**Deliberate divergences:**

- A `agent.py:795–805` automatically acknowledges pending safety checks after execution, with actual consent left TODO. Require explicit durable approval before dispatch instead.
- `operator_validator.py` strips x/y from targeted typing and keeps only path-form drag. Retain typed-target and endpoint-drag semantics or refuse; do not discard intent.
- Missing coordinates are errors; `(0,0)` remains a valid coordinate. Never normalize missing values into zero.

**Tests:** finite/range/item/text/path limits; quoted Unicode; key/button aliases; press/release cleanup; screenshot failure after delivery; cross-session refs; safety refusal before any effect.

## 2. First model profile: ordinary computer function calls

**Source:** A `loops/openai.py`: `_map_computer_tool_to_openai`, `_prepare_tools_for_openai`, `_is_native_computer_use_model`, `OpenAIComputerUseConfig.predict_step`.

Upstream routes some models through an ordinary `computer` function schema rather than native computer-use blocks. Port schema/profile generation and argument decoding to Rust and register execution in Pi. All formats—not only native ones—must commit Pi-owned intent before any effect. Persist parser/profile version, source output and observation binding. Model image preprocessing creates a versioned model-view record (capture ID/digest, processed dimensions, parameters, transform to delivered-image pixels), composed with the desktop transform rather than inferred from the last historical image. Advertise only implemented action variants. Preserve current Pi model transport, usage and credentials. Upstream model-name routing is pinned behavior, not a current endpoint guarantee.

**First integrated test:** fake Pi provider returns computer tool call → Rust executes once → result has PNG/structured outcome → next model request pairs the correct call and result. Restart cannot replay its effect.

## 3. UI-TARS, Qwen and other textual action profiles

**UI-TARS:** A `loops/uitars.py:109–447`: `parse_box_coordinates`, `parse_action`, `parse_uitars_response`, `convert_to_computer_actions`, `process_image_for_uitars`; history rendering follows. Port restricted allowlisted call/keyword grammar, normalized 0–1000 boxes and center mapping, completion/call-user outcomes, preprocessing and prompt/history layout. Do not execute model code. Distinguish the source's actual area-based image processing from its separate factor-snapping smart-resize helper.

**Qwen3.5/generic:** A `loops/qwen35.py:139–275` and `generic_vlm.py:132–236` support differing JSON/XML-like tool formats and normalized coordinates. Port both dialects explicitly. Upstream triple-click-to-double-click mapping is lossy; implement triple click or refuse. Preserve answer/terminate.

**Next inventory:** A `loops/uitars2.py` seed-tool parsing/coordinate conversion/message rendering; `loops/fara/helpers.py` call extraction/action conversion; `loops/opencua.py` click extraction/rescaling. These symbol families were located, not exhaustively audited. Give each profile its own fixtures and capability entry.

**Fixtures:** `libs/python/agent/tests/test_uitars_coordinate_security.py`, `test_predict_click_zero_coords.py`; golden resize/color images, punctuation/escaping, wait, scroll sign/unit, normalized drag endpoints and malformed extra syntax. Pillow/Rust filter names alone do not establish image-byte parity.

Text-only predictions must be converted to Pi ToolCalls inside the model/provider adapter before Pi classifies the answer. pi-durable's `afterResponse` hook is void, not an output transformation interface. Keep parsing Rust and the stream/ToolCall bridge thin JavaScript.

## 4. Native provider formats: distinct integration projects

### Anthropic

A `loops/anthropic.py`: `_get_tool_config_for_model`, `_map_computer_tool_to_anthropic`, `_prepare_tools_for_anthropic`, `_convert_responses_items_to_completion_messages`, `_convert_completion_to_responses_items`, `predict_step`.

Port native declaration/beta requirements, enhanced actions, key translation, resize and per-axis coordinate restoration. pi-ai already carries `tool_use`, image-bearing `tool_result` and signed thinking, making this narrower than OpenAI native support. Native declarations still differ from function schemas. Test OAuth tool-name transformations, beta flags, image dimensions and cache wrapper composition. Store original provider coordinates plus mapping, not only executed coordinates.

### OpenAI native

Installed pi-ai `dist/api/openai-responses-shared.js` handles function/custom tool events but lacks `computer_call` decoding/replay. Native computer blocks/results are not represented directly by the current assistant/tool-result unions. `onPayload` injection cannot fix that.

Add maintained provider decoding, native-envelope persistence and outbound replay conversion (or an upstream Pi extension) before enabling. Round-trip IDs, safety checks, reasoning/signatures, image results, interruptions, reset/model change and SQLite storage. Do not claim native wire parity from a generic-function implementation.

### Gemini

A `loops/gemini.py:177–338` history conversion, `:343+` declarations, `:558–688` mapping. Built-in versus custom function routes differ. Preserve targeted typing/press-enter, 0–1000 points, scroll-at/document, endpoint drag and actual wait durations. Arbitrary `playwright_exec` is not implicitly supported: use the typed browser module or refuse executable scripts pending a sandbox policy.

Use pi-ai's signature-aware function-message conversion rather than upstream's lossy textual history. Preserve native identity/thought signatures instead of creating new IDs repeatedly. Dojo's current provider builder accepts only openai-codex/openai/anthropic, so Google authentication/model registration is separate work.

**Shared gate:** real endpoint support for the chosen subscription or explicit credential mode; no incidental API-key fallback. Pi memory resets between independent submissions; native metadata needs explicit retention/reset rules rather than assuming one uninterrupted provider transcript.

## 5. Composed grounding, retention and budgets

**Grounding:** A `loops/composed_grounded.py:108–287` plus description conversions in `responses.py`. Port planner-description-grounder composition into owned Pi tasks. Cache by capture ID + description + model/profile, never description alone. Upstream persistent `desc2xy` can retain old coordinates when new grounding fails. Unresolved targets must remain unresolved; account planner and grounder usage.

**Retention:** A `callbacks/image_retention.py:40–99`. Keep original durable entries immutable; build a bounded request projection preserving call/result pairing and required signatures. Source retention only trims certain adjacent screenshot records and Python `[-0:]` gives surprising zero behavior. Define 0/1/N explicitly and distinguish request retention from recording/memory storage policy.

**Privacy:** A `callbacks/pii_anonymization.py` is a TODO/pass-through implementation; it returns unredacted inputs and is not usable redaction. Build Dojo deterministic field filtering and scoped capture-consent/storage policy explicitly. Test Pi entries, memory mirrors/summaries and zoom, not just exports.

**Human requests:** A `human_tool/server.py` and `adapters/human_adapter.py` provide pending/completed/failed request IDs, responses/tool calls and timeout behavior, but use an in-memory queue and HTTP polling. Port the useful request/response concepts into a Pi-owned durable pending-human task/document and authenticated Dojo UI response. Expiry/cancellation/restart/deduplication and clarification-versus-permission are explicit. Never resume stale coordinates after a human response.

**Budgets:** A `callbacks/budget_manager.py`, `base.py`. Port useful gating semantics but counters belong in durable Pi documents. Registry before-tool/generation hooks apply operator consent and budgets without a second callback scheduler. Additional logging/PII/OTel/trajectory callbacks need separate audit before parity is claimed.

## 6. Benchmark ports

**Action interchange:** B `actions.py:1–310`, `types.py`. Port repr/snake-case/dictionary codecs with whole-input consumption and explicit spaces; upstream regex matching and missing-coordinate defaults are not adequate validation.

**Episode core:** B `runner/episode.py`, `agents/base.py`, `computers/base.py`. Port setup→agent/oracle→evaluate→trace→cleanup/result/reward phases. The desktop protocol includes apps/install/shell/HTML/JS/window lifecycle beyond input; classify each task's prerequisites. Arbitrary Python task callbacks are rewritten as Rust task/evaluator modules, not dynamically imported by a hidden bridge.

**Offline data processors:** B `processors/base.py`, `gui_r1.py`, `aguvis_stage_1.py`. GUI-R1 needs annotated HTML/ARIA plus hit-test/bounding boxes. Aguvis includes image-border shrinking, target filtering, normalized coordinates and random template augmentation. Use seeded RNG, explicit normalized/pixel metadata and gold HTML/image fixtures. Arrow/Hugging Face exports are optional adapters, not required runtime dependencies.

**Fixture sources:** B `tests/test_actions.py`, `test_export_schema.py`, `test_results_harbor.py`. Port evaluator families coherently and demonstrate one isolated task scored identically before widening. Never run task setup on the human desktop by default.

## 7. S1 pure policy before tensors

**Immediate portable policy:** S `schema.py`, `planner.py:60–207`, `driver.py:60–353`. Port entities/elements/decisions, option encode/decode/context rendering, finite [0,1] confidence, one-decision-per-element validation, action ordering, confidence gate and controlled submit. Driver policy requires complete snapshot, unique element identity/token, exact window target and fresh observation after every mutation including transport failure. Only confirmed effect is confirmed success.

This policy is useful independently of S1 weights. Schedule it with model/grounding policy after semantic observations, not only behind the inference project. Candidate computation is Rust; Pi owns planning calls and durable phases. Preserve Python character-based context truncation intentionally, not unsafe Rust byte slicing.

**Inference expansion:** S `model.py:48–238`, `nano.py`, `four_b.py`. Port byte+1 tokenization/masks/options and prompt/letter assignment first; then one small scorer/checkpoint, nano text scorer, vision pipeline and finally Qwen3.5-4B+LoRA after feasibility. Candle/Burn/ONNX/LibTorch routes are candidates, not proven compatible backends; a LibTorch runtime is distinct from a Python dependency and still needs explicit deployment/dependency approval.

**Fixtures:** S `tests/test_schema.py`, `test_planner.py`, `test_driver.py`; logits/probabilities/top-1 comparisons for tensor ports with specified tolerances. Source, base-model weights, adapters and datasets have separate licenses. No download or live inference has been verified.

## Delivery sub-milestones

08a canonical codecs/function profile; 08b UI-TARS/Qwen and preprocessing; 08c retention/budgets/composed grounding; 08d S1 pure policy with 06; 08e Anthropic native feasibility; 08f OpenAI native and Gemini; 08g remaining dialect sweep. Basic supported profiles can ship without blocked native profiles.

11a recording/trace interchange; 11b Rust episode/reward core and one evaluator family; 11c GUI-R1/Aguvis offline processors; 11d broader evaluators/tasks. 14 owns tensor/backend work only, with exact model variants and quality/resource gates.

No production Python, independent ComputerAgent scheduler, automatic pending-safety acknowledgment or unbounded generated-code execution is part of this plan.
