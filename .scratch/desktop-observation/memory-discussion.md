# Dojo-only memory discussion

The [UniiChat gist](https://gist.github.com/VictorTaelin/91837951a5ce5b38f341ec1ba1df6449)
proposes an append-only conversation log, a binary summary tree, a persisted
bounded view, and zoom/date retrieval. Each turn starts fresh with that view.
It batches view merges to preserve prompt-cache prefixes. Its reported cache
ratios come from a cache simulation, not a Dojo deployment measurement.

Retaining source messages allows recovery, but summaries can omit the clue
needed to retrieve them. The gist clips long tool results, so its no-loss claim
requires qualification. Its 64–128 KB context is a byte budget, not a token
budget, and may be excessive for short voice commands. It also requires control
of request assembly and background summarization; copying the prompt does not
implement the runtime.

Any experiment must use a dedicated Dojo agent workspace and memory directory,
Dojo-only tools/instructions, and its own lifecycle. Do not change global Codex
configuration, root AGENTS.md, T3 defaults, shared scratch instructions, other
agents' histories, or their compaction behavior. Current voice dispatch uses T3,
so its supported per-agent integration boundary must be established first.

No memory implementation or configuration has been applied.
