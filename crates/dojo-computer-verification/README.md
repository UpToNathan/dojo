# Computer verification and bounded steps

This crate implements CUA-derived deterministic predicates and a pure bounded-step protocol. Dojo supplies trusted observations, exact session/window identity, time, cancellation and effects. Pi owns durable conversations, phase documents and tool intents. The crate creates no desktop seat, timer, transport or task scheduler.

## Predicate observations

`ObservationSnapshot.source` explicitly distinguishes unavailable evidence, native evidence and trusted browser DOM evidence. An unavailable snapshot cannot prove negative window existence. `observation::browser_observation` requires exact equality of the expected and observed session, compositor epoch, native window, client generation and process/start identity. It also requires a current document binding and bounded unique element identities. A changed identity or missing binding produces unavailable evidence.

Window predicates support existence, rendered bounds, exact title and exact browser URL. Missing properties remain unknown. Native and browser element predicates support existence, value, enabled and selected state. Sensitive values are omitted. Positive existence can use a partial trusted list. Negative existence requires complete traversal, no truncation, no missing frames and no actionable-only filtering. Property checks on incomplete coverage remain unknown because unique-match evidence is unproved.

The current CUA ref-only BrowserReadObservation list is not a complete DOM tree. Adapters must declare filtered coverage. `unbound_accessibility_observation` remains unavailable because application-wide AT-SPI observations do not prove an exact native top-level window. The native service can supply trusted GTK7 element evidence only after verifying the exact surface annotation and owner. Redacted controls prevent complete semantic coverage. Native AX/AT-SPI web regions remain untrusted unless supplied through the distinct browser evidence adapter.

`Stability` requires consecutive satisfied samples. A final true sample without the required consecutive count becomes unknown. The production adapter owns fresh acquisitions and bounds each external wait; the crate performs no idle polling.

## Pure step protocol

`steps::new_run(run_id, limits, now_ms)` creates a bounded run. `steps::transition(state, event, now_ms)` returns its next serializable state. The limits permit 1 through 64 steps, at most 128 acquired observations and a 1 through 300,000 millisecond run deadline. Serialized state is bounded to 60 KiB, and IDs/digests have strict limits. Clock rollback, expiry and malformed checkpoints cannot admit an effect.

The phases are observe, ready, in-flight, stopped and finished. An effect requires a new server observation and a matching one-use capture. Capture and observation IDs cannot be reused. The caller must commit the in-flight phase before transport. Its `StepIntent` includes the current index, capture, intent ID and exact action digest; it stores no action text or future coordinate list.

Handled or confirmed receipts append the completed prefix and require another observation before the next effect. Handled means delivery, not verified UI success. Refusal, stale evidence and human pause stop immediately. Partial and unknown effects retain the completed prefix plus the complete ambiguous intent. Restart during an in-flight phase becomes unknown and never recreates an executable action. Restart before intent retires a ready capture. Cancellation and deadlines during effects preserve ambiguity.

Finishing the requested step count is not task verification. A caller can run `desktop_verify` against new trusted evidence to assess its own success criterion. The protocol does not turn a handled receipt into a successful predicate or silently resume an ambiguous action.

## Pi integration and CLI

`tools/speech/computer-steps.mjs` installs the bounded facade after the existing desktop extension. It uses `runtime.executeDesktopTool`, including its authorization, capture consumption, durable budget, cancellation and transport behavior. Each apply tool executes one current action. Rust owns phase transitions, and Pi stores their results in one conversation document. There is no automatic action loop or second persistence owner.

The facade registers begin, observe, apply, status and cancel operations. Unknown effects block a replacement run. Operator recovery is outside the model tool catalog. Stop closes the desktop transport and retains any pending ambiguity. Raw desktop tool results and images pass through unchanged, with a separate step summary that always reports `task_verified: false`.

The `dojo-computer-verification` CLI reads one bounded JSON command from stdin. `steps_new` accepts a run ID, limits and current time. `steps_transition` accepts a checkpoint, one event and current time. It performs no desktop effects. Pi calls this compiled Rust codec rather than duplicating the protocol in JavaScript.

Build and verify from the repository root:

```sh
devenv shell -- cargo build --manifest-path crates/dojo-computer-verification/Cargo.toml --bin dojo-computer-verification
devenv shell -- cargo test --manifest-path crates/dojo-computer-verification/Cargo.toml
node --test tools/speech/computer-steps.test.mjs
```

The Rust suite passes 24 tests: 15 existing predicate/stability cases with the negative-coverage expectation updated, plus nine fresh-observation, restart, ambiguity, fake-clock, DOM coverage and identity cases. Five integration tests use the compiled Rust codec and actual PiRuntime, including real SQLite reopen, blocked unknown-effect replay, the persisted intent/effect gap and stop during an in-flight effect. The fifth reads the actual Rust tools/list catalog and proves nested region/drag/model-source references validate before transport; a separate pure normalization case brings the Node suite to six tests. Desktop transport responses are deterministic fixtures; this evidence does not claim a real browser or installed runtime.

## Provenance and divergences

The MIT baseline is `trycua/cua` revision `88aff7e5424e9ed45b93ded85eb4111472ca5190`. The archived verification contract and expectation implementation remain unchanged under `provenance/`. The adapted contract removes UniFFI/session registration labels. The evaluator retains upstream predicates and their relevant tests.

The step protocol adapts the ownership and bounded dispatch semantics of `cua-driver-core/src/batch_tools.rs`. Dojo deliberately requires a fresh observation for every action instead of importing the upstream precomputed action list and optional final-only observation. Pi handles the ordered tool tasks, so no CUA ToolRegistry or scheduler is instantiated. Window title/URL predicates and complete trusted browser absence are local extensions. Exact source hashes, scope and retained divergences are in `provenance.json`, and MIT attribution remains in `LICENSE.md`.
