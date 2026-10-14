# Computer recordings and offline evaluation

`dojo-computer-recording` stores bounded, redacted projections of Dojo tool events, exports ATIF-v1.8 trajectories, renders bounded terminal recordings and runs isolated dataset evaluations. Pi remains the canonical conversation and intent owner. This crate sends no model requests and never dispatches recorded desktop effects.

## Commands

Build and test from the repository root:

```sh
devenv shell -- cargo test --locked --manifest-path crates/dojo-computer-recording/Cargo.toml
devenv shell -- cargo build --locked --manifest-path crates/dojo-computer-recording/Cargo.toml
```

The CLI reads one JSON command from stdin and writes JSON to stdout. Invalid input exits with code 2. Input is limited to 8 MiB. Commands are `record`, `analyze`, `atif`, `transcript`, `evaluate`, `evaluate_general`, `metrics`, `usage`, `gui_r1`, `aguvis` and `benchmark`. Their typed inputs are in `src/bin/dojo-computer-recording.rs`.

`evaluate_general` compares supplied evidence with ten typed metric families from the pinned CUA benchmark. It never runs getters, scripts, SQL or recorded effects. Missing evidence produces an unknown reward. JSON subsets require the expected key to exist, including when its expected value is null. This differs deliberately from the upstream comparator. Fuzzy text uses bounded Unicode LCS and the normalized Indel operation order from hash-pinned RapidFuzz 3.14.6. Its three fixtures match that reference exactly.

The 25 general metric fixtures run through the actual CLI in `tests/general_metrics.rs`. An optional development reference check requires CPython 3.14.8, the exact upstream checkout and the RapidFuzz source archive in `provenance/artifacts/rapidfuzz-reference.lock.json`. It extracts reviewed functions from pinned Git objects and substitutes supplied evidence for host getters:

```sh
devenv shell -- python3 scripts/check-cua-metrics-reference.py /home/nathan/.cache/dojo/upstream/cua/88aff7e5424e9ed45b93ded85eb4111472ca5190 --rapidfuzz-archive /home/nathan/.cache/dojo/artifacts/rapidfuzz/3.14.6/source.tar.gz
```

This check compares 25 fixtures, with 24 matches and the explicit missing-key divergence. Python is not part of the production evaluator.

Run the isolated grounding fixture set:

```sh
devenv shell -- cargo run --quiet --manifest-path crates/dojo-computer-recording/Cargo.toml < crates/dojo-computer-recording/tests/fixtures/offline-benchmark.json
```

The three fixture tasks produce rewards `1`, `0` and `null`. The mean over known rewards is `0.5`. `null` means evidence is missing or execution was interrupted; it does not mean a passing task.

## Recording API

Create `Recorder::new(session_id, RecordingPolicy)`, call `push(TraceEvent)` and obtain a `Trace` with `finish()`. Trace fields are private so callers cannot create an unredacted stored trace through that type. `ingest(bytes, policy)` applies the same validation and privacy rules to imported traces. Event times must be monotonic; intent and receipt events require matching Pi intent and call IDs. A receipt has exactly one explicit effect classification.

`analyze(&trace)` reports unresolved intents, orphan or duplicate receipts, confirmed effects, partial effects, unknown effects, refusals and suspected no-ops. Its `safe_to_replay_effects` field is always false. Missing receipts preserve the interval between an external effect and a stored result. Neither a receipt nor replay analysis authorizes dispatch.

`export_atif(&trace)` preserves tool call IDs and emits ATIF-v1.8 tool calls and observations. Relative recording time travels in `extra.time_ms`; the ATIF timestamp remains null because a monotonic relative time is not an ISO date. This export does not advertise equivalence to every Harbor field or CUA screenshot renderer.

`usage::aggregate` projects up to 4,096 canonical model receipts into integer token and reported cost totals. Each sample requires a lowercase SHA256 receipt identity, inclusive input tokens, output tokens, cached input subsets and an optional reported estimate in microUSD. Duplicate identities, unknown fields, inconsistent subsets and integer overflow refuse the whole projection. Missing costs remain explicit in `samples_without_cost`; no price is inferred. Output omits receipt identities. `Recorder::set_usage` and the `record` CLI's optional `usage` input attach validated totals to `Trace.usage`. The field is omitted for legacy/no-usage traces; schema version remains unchanged, ingestion accepts and validates it, and ATIF carries it in `final_metrics.extra.usage`. Pi retains budget enforcement and canonical receipts.

The runtime's actual `exportDesktopTrace` hook extracts usage from canonical assistant entries. Installed `pi-ai` stores uncached input separately from `cacheRead` and `cacheWrite`; the adapter adds those once for inclusive input. It hashes the conversation ID and canonical entry ID together, exporting no provider response IDs. Explicit finite nonnegative cost estimates are rounded to the nearest microUSD only within JavaScript safe integer precision. Missing, negative, nonfinite or unrepresentable estimates remain unknown; explicit zero remains zero. These are provider/library estimates, not measured bills. Four Node tests include the real Pi immutable entry transaction API and Rust record codec, assert exact totals and verify history is unchanged.

## Privacy and artifact limits

Recording is opt-in at the caller. The default policy retains at most 1,000 events, 64 KiB per event and 8 MiB overall. Redaction runs before an event enters the recorder. It removes password, token, credential, authorization, cookie, clipboard, typing text, raw content, HTML and screenshot fields recursively. Caller-supplied secret values are removed from every string and map key. Common bearer and basic credentials, `sk-` tokens and secret query parameters are suppressed. Redacted map key collisions fail instead of dropping a field silently. JSON nesting is limited to 32 levels.

Image retention defaults to disabled. `add_png(bytes, capture_consent)` requires the caller's explicit capture consent when images are enabled. It accepts in-memory PNG data, limits byte count, image count and pixel dimensions, validates the full chunk envelope and checksums, and deduplicates by SHA256. Imported images must match their declared dimensions and digest. The validator does not decompress or render PNGs. No artifact path or URL is opened and no filename comes from a call ID. Image bytes can contain visual secrets; the session owner must deny sensitive capture scopes or obtain consent before supplying them. Text filtering cannot establish the absence of visual PII.

Transcript input requires asciinema v2, bounded dimensions, ordered finite timestamps and valid event shapes. A computation budget and bounded control parameters protect the imported renderer. CSI `M` and `S` scroll-up sequences are refused because the pinned renderer can underflow on a full-region scroll. Caller-supplied secrets in wrapped screen lines suppress all terminal lines, so wrapping cannot bypass the filter. Unknown visual or terminal secrets still require caller scope policy.

## Offline processors

`processors::gui_r1` accepts consented annotated HTML and an image digest/dimensions. It ports ARIA-label preference, instruction fallback, center-hit filtering, deduplication and clamped normalized bounding boxes. Output includes `coordinate_space: normalized_0_1` and the upstream instruction, action, history, task type, OS and resolution fields.

`processors::aguvis` ports instruction, ARIA and leaf-text candidate extraction, content-border shrinking, candidate filtering, click/move templates, bounded attempts and deduplication. `rgb_base64` is an explicit bounded RGB buffer with three bytes per pixel. RGB data is required for the text-border pool. Without it, the output declares that pool unavailable. Annotated HTML is limited to 256 KiB, targets to 1,024 and total border comparisons to a bounded pixel budget. A versioned SplitMix64 seed makes Rust augmentation repeatable; its random choices differ from Python's MT19937.

Predefined HTML `data-actions` pairs require `action_coordinate_space`, either `image_pixels` or `normalized01`. The parser consumes an entire allowlisted click/move or endpoint-drag call. It normalizes both endpoints and refuses extra syntax, unsupported actions and coordinates outside the image. It never executes code. Explicit spaces replace upstream's ambiguous heuristic that treats values greater than one as pixels. Output includes typed points as well as the textual training pair.

HTML and RGB are supplied by the caller; this crate does not acquire DOM hit-test evidence, fetch images, read Arrow datasets or upload to Hugging Face. The source image digest is supplied metadata and must be correlated with captured pixels by the acquisition adapter. Raster annotation labels/fonts are not ported; the typed normalized boxes are available for an annotation adapter.

## Dataset episode and evaluators

`benchmark::run_tasks` runs setup, answer recording, evaluation, trace export preparation and cleanup in an isolated in-memory dataset session. `max_tasks`, `max_steps` and an 8 MiB combined report bound apply. A missing answer or interruption produces unknown reward and a completed prefix. Dataset answer IDs carry `fixture_intent: true` and route `dataset`; they never represent a Pi-owned live desktop intent. Confirmed dataset receipts mean the answer was recorded, not that a UI changed.

The grounding family ports `cua_bench.adapters.grounding` and the ScreenSpot-Pro exactly-one-click evaluator. Left, right, double and middle clicks count as point answers; moves do not. Boxes accept reversed endpoints and include boundary points. Polygons use the upstream even-odd rule and edge tolerance. Infeasible targets require an explicit infeasible report and no click. Image pixel and normalized spaces are explicit and must agree for target and answer.

`reward_of` ports scalar/boolean/nested-list reward reduction. `pass_at_k` ports the Harbor unbiased binary estimator, grouped by task and variant, with missing rewards counted as failure. Generic exact, substring and JSON pointer predicates return success, failure or unknown. Tests use literal gold cases for these semantics and execute the CLI fixture task set.

Prerequisites for external dataset use are a licensed dataset artifact, verified image metadata, matching coordinates and supplied model answers. ScreenSpot-Pro artifact downloading is gated separately and is not performed by this crate. Live task setup, model generation, browser/accessibility observation, OSWorld application metrics and disposable desktop lifecycle require adapters outside this crate. There is no host shell, generated-script executor, scheduler, credential store or automatic retry here.

## Source and parity

`provenance.json` records the pinned source paths, hashes, scope and divergences. `LICENSE.md` preserves Cua AI's MIT notice; `LICENSE-cua-bench` preserves TryCua's benchmark MIT notice. The transcript renderer remains the imported MIT Rust dependency. No FSL, AGPL code, model weights or dataset images are included in this crate.

Tests cover privacy before retention, trace round-trip, effect gaps, image opt-in/validation, terminal bounds, exact ScreenSpot score cases, episode cancellation/budgets, GUI-R1 gold geometry, Aguvis RGB border gold output, seeded augmentation, usage totals and the standalone CLI. The Pi `desktop-trace.mjs` integration projects canonical desktop intents/results and assistant usage through Rust. Full CUA benchmark compatibility remains separate integration evidence.

## Callback portability audit

| Retained callback behavior | Dojo seam and limits |
| --- | --- |
| Trajectory and logging usage accumulation | Typed `usage::aggregate`; exact bounded integers replace unrestricted nested addition and floating costs. |
| Trajectory artifact and image URL sanitization | `Recorder` and consented bounded PNG ingestion; no untrusted artifact filenames or executable replay. |
| Image retention | Rust model retention policy and Pi request projection; canonical history remains intact. |
| Budget manager and prompt instructions | Existing Pi/model policy owner; recording adds no enforcement loop or prompt mutation. |
| Operator validator | Existing strict Rust model/action codecs; unsupported or ambiguous forms refuse. |
| PII anonymization | Upstream pass-through placeholder is replaced by deterministic pre-persistence filtering. |
| Telemetry and OpenTelemetry | No external sinks added. Canonical Pi receipts and exported usage provide local evidence. |
| Raster annotations and live app metrics | Require separate licensed artifacts and trusted acquisition/application adapters. |

This audit covers the pinned MIT callback sources, not arbitrary third-party dataset evaluators or downloaded artifact licenses.
