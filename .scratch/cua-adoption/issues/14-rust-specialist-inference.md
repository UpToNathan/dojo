# 14 — Port specialist model/perception behavior with Rust inference

Type: research
Status: claimed
Blocked by: 01, 06, 08, 10, 11
Effort: exploratory/large

## Brief

Inventory `libs/cua-s1/python` semantic candidate scoring/action integration and `cua-perception` worker/runtime/model artifacts. Reuse ticket 08's Rust schema/planner/driver policy instead of deferring or duplicating it here. Port tensor preprocessing (byte+1 tokenization, masks, options, context encoders and prompt/letter assignment), then prove a Rust backend (for example a compatible tensor or exported-runtime backend) for each chosen model. ONNX/export availability is a question, not an established fact. Validate tokenizer, preprocessing, dtype/quantization, LoRA loading, scores and grounding accuracy against reference fixtures.

Do not ship Torch/Transformers/Python as a temporary product shortcut. Keep inference a bounded Rust worker owned by a Pi task/tool, with cancellation/resource limits and usage accounting. Model calls cannot become a hidden autonomous loop. No training pipeline is required for inference; source license does not license weights/datasets or AGPL detector bundles.

## Acceptance

- Model/backend/artifact feasibility and exact licenses recorded before integration.
- Numerical/behavioral reference comparisons, task accuracy and latency/memory baseline on intended hardware.
- Snapshot requirements satisfied, stale tokens refused, fallback/unknown behavior explicit.
- No Python runtime dependency or uncontrolled model download/start at idle.
- If backend parity is infeasible, document blocked slice and retain useful Rust preprocessing/contract ports.

## Comments

The capture-bound perception slice now uses the exact baseline Rust worker, an immutable Nix installation, exact loader files and interpreter, cold-process cancellation/reaping, and an off-loop service completion gate. The public `parse_visual_regions` input is the pinned manifest schema. Pi classifies it as read-only and retains its capture marker. The session resolves the immutable PNG and rechecks binding, consumption, retirement and expiry before publication. It creates no approval or action reference.

Real detector/OCR execution and cancellation passed against staged hash-pinned artifacts. The full worker suite passed 35 tests; a separate original known-answer test still requires its missing fixture. The actual contained known-answer parse uses the pinned native-controls corpus. Mutable installations, malformed ELF and missing capture/artifact echoes are refused. The fake-clock lifetime test proves reads do not renew expiry.

This remains partial parity. Named-output desktop parsing, exact error envelopes, warm reuse, broader host/deployment matrices, complete corpus quality metrics and S1 tensor inference remain open. Durable approvals are reserved for discussion and are not prerequisites for this local read-only slice.
