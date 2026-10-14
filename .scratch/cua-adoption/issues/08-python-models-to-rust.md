# 08 — Port Python computer-use and grounding semantics to Rust, integrate through Pi

Type: research
Status: claimed
Blocked by: 01, 02, 03
Effort: medium for pure parsers; exploratory/large for native provider turns

## Brief

Inventory `libs/python/agent/cua_agent/{loops,types.py,responses.py,callbacks}`. Build a path-to-behavior-to-fixture ledger. Translate action decoding, coordinate transforms, grounding composition, completion/refusal/ask-human handling, image retention and budget behavior into bounded Rust modules. Do not translate `ComputerAgent` into a second scheduler or port Python-generated-code execution literally.

First milestone: generic tool-based vision and representative UI-TARS/Qwen-style parsers. Follow with native Anthropic/OpenAI/Gemini formats only after proving pi-ai/pi-durable support for native call IDs, request/result messages, streaming, durable transcript, safety acknowledgments and account endpoint capabilities. Keep pi-ai credential/provider ownership; subscription availability is not implied by API-key computer-use docs. Use pinned provider extension/upstream changes where needed rather than silently serializing native blocks as generic tools.

Grounding model calls are owned Pi tasks or registered model/tool invocations; Rust decoder does no model HTTP requests. Bound action lists, execute observation-dependent steps with fresh-state rules, and integrate normalized actions through the shared Rust module. Fixtures may be generated from isolated reference code, but ordinary tests and production use no Python.

## Sub-milestones and exact source inventory

Follow [python-port-inventory.md](../python-port-inventory.md): 08a canonical codecs/ordinary function profile; 08b UI-TARS/Qwen restricted parsers and image preprocessing; 08c image retention/budgets/composed grounding; 08d S1 `schema.py`/`planner.py`/`driver.py` pure policy after 06; 08e Anthropic native declarations/transforms; 08f OpenAI native response support and Gemini; 08g remaining dialects individually audited. Tensor inference stays in 14.

Use `responses.py`, `agent.py:717–881`, `callbacks/operator_validator.py`, `loops/openai.py`, `uitars.py`, `qwen35.py`, `generic_vlm.py`, `composed_grounded.py`, `anthropic.py`, `gemini.py`. Begin fixtures with upstream coordinate-security/zero-coordinate tests and S1 schema/planner/driver tests. Do not inherit automatic safety acknowledgment, lossy targeted-type/drag normalization or stale description-only grounding caches.

Enforce model-issued action order, not only serial socket arrival. After-effect observation failures report unknown and never repeat input. Text predictions must become ToolCalls before Pi answer classification; `afterResponse` cannot transform the answer. Request retention preserves native IDs/signatures/call-result pairs across bounded projection; define reset/model-change semantics for Dojo memory's fresh-context behavior.

Every profile, including textual UI-TARS/Qwen, must produce a Pi-owned tool intent before any effect. Persist source output/profile-parser version/observation reference and a model-view record binding capture ID/digest, preprocessing parameters, processed dimensions and model-view→delivered-image transform. Crash-test decoding, intent, effect and result boundaries. Never infer the applicable image from the last history item.

Inventory `human_tool/` and `adapters/human_adapter.py`; port request IDs/status semantics into a Pi pending-human document/task with authenticated Dojo UI response, expiry, deduplication, cancellation and restart. Do not import in-memory queue/polling. Clarification is not authorization, and resumption requires fresh observation.

## Acceptance

- Exact upstream paths and useful parser families inventoried, including unsupported/retired cases.
- Rust differential/golden tests for coordinate spaces, malformed output, key/scroll mapping, completion/refusal and bounds.
- Model output containing code cannot execute arbitrary Python/shell in the desktop module.
- Each enabled provider has real wire/transcript/restart evidence and explicit credential-mode support.
- One durable run/usage ledger/cancellation path; no autonomous inner CUA loop.

## Comments

Implementation evidence, 2026-10-12: completed the interrupted standalone
`dojo-computer-models` decoder crate. Canonical, UI-TARS, Qwen3.5 JSON/parameter
and generic JSON forms have twelve Rust golden/adversarial tests. Model-view
mapping, zero coordinates, paths, targeted typing, terminal outcomes and bounded
literal parsing are covered; provenance/notice and explicit divergences are
recorded. No Python, model requests or desktop effects are in this crate.
Pi provider-intent bridge, native formats, image preprocessing/retention, S1
policy/inference and durable ask-human integration remain unimplemented.
See [result](../result.md).
