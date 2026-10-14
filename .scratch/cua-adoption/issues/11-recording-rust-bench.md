# 11 — Recording/history reuse and Python benchmark behavior port

Type: research
Status: needs-triage
Blocked by: 02, 03, 09, 10
Effort: large; split recording and evaluator families

## Brief

Import core recording/action history/render/export formats and port useful Python trajectory callback behavior. Supply scoped capture/semantic adapters, eliminate conflicting global hooks, reference Pi conversation/tool intent IDs, and report routes/refs/partial effects/verification. Recording is bounded, opt-in and redacted before storage; Pi is still the canonical conversation owner.

Inventory `libs/cua-bench` task models, runner, evaluators, setup/lifecycle and trace/result formats. Translate deterministic evaluators and a Rust runner behind Dojo desktop and Pi agent adapters. Record per-task application/image/filesystem/OSWorld/dataset dependencies; do not assert whole-benchmark compatibility from click support. Prefer reusable task data/gold fixtures; do not run generated scripts with host privileges. Disposable nested desktop is the default evaluation target.

## Sub-milestones

Follow [the exact port inventory](../python-port-inventory.md): 11a recording/trace interchange; 11b `cua_bench/actions.py`, `runner/episode.py`, result/reward core and one evaluator family; 11c `processors/gui_r1.py` and `aguvis_stage_1.py` offline HTML/image processors with explicit coordinates and seeded augmentation; 11d wider evaluator/task families. Semantic tasks/processors require 06/07 observations and annotated target evidence; ordinary trace/window tasks need not wait for them.

## Acceptance

- Trace round-trip and upstream-format comparisons; receipt/result/crash gaps remain explicit.
- Gold evaluator success/failure/unknown fixtures; actual small end-to-end Rust-run task set.
- Setup/teardown own only their processes/files; failures leave no seats or held input.
- Baseline comparison versus current Dojo with task/model/version/conditions and latency/resources.
- Default Rust tests require no Python; dataset/artifact licenses recorded and sensitive data excluded.

## Comments

2026-10-12 implementation evidence: `crates/dojo-computer-recording` now implements bounded privacy-filtered traces, effect-gap analysis, ATIF-v1.8 projection, bounded asciinema observations through the unchanged MIT transcript crate, scalar rewards and Harbor pass@k. It also ports GUI-R1 annotated-HTML row extraction/normalization and Aguvis target pools, RGB border shrinking, deterministic augmentation and restricted predefined-action parsing with explicit coordinate spaces. The Rust dataset episode harness runs setup, answer recording, grounding evaluation, trace and cleanup in memory, with task/step/report bounds and cancellation-prefix/unknown outcomes. Source paths, hashes, MIT notices and divergences are in the crate's provenance manifest. No external dataset artifacts, live desktop effects, generated setup scripts or extra model scheduler are introduced.

Validation: `devenv shell -- cargo test --manifest-path crates/dojo-computer-recording/Cargo.toml`. The tests execute literal upstream ScreenSpot exactly-one-click, inclusive box/polygon and infeasible gold cases, GUI-R1 normalized-box fixtures, Aguvis RGB border gold output, redaction/artifact/terminal bounds and a CLI fixture set with rewards 1/0/unknown. Live desktop recording hooks, Pi export integration, application-specific evaluator families, dataset artifact license gates, disposable desktop lifecycle and real model baseline comparison remain separate integration work. These unit and offline scores do not establish live benchmark compatibility or ticket completion.
