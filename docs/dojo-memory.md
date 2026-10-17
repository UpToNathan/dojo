# Dojo agent memory

Dojo runs its desktop agent locally through pinned `pi-durable` and `pi-ai`
1.3.0 packages. T3 remains available for other work. The speech socket, warm
transcription worker, floating bubbles and desktop input ownership remain the
entry points. Dependencies are local to `tools/speech`; no global agent settings
or provider defaults are changed.

## Memory algorithm

The implementation follows the UniiChat gist's binary memory tree. Daily
`main/YYYY-MM-DD.jsonl` files retain messages with permanent IDs, kind, text,
UTF-8 size and date. Daily `tree/YYYY-MM-DD.jsonl` files retain immutable summary
nodes. The single owning process flushes appends under an OS lock. Operational
receipts and crash checkpoints live separately in `state/`.

The foreground view grows by appending lines. Above 128,000 UTF-8 bytes, it
merges built sibling nodes until reaching 64,000 bytes. The merge order uses
age measured from a pair's last message, divided by its range size, with oldest
pairs winning ties. An unfinished batch remains pending until its parent nodes
are built. `view.json` stores the exact live `[level,index]` ranges; startup
never refits the view from history.

A separate persisted compaction view grows from 16,000 to 32,000 bytes. It
contains built history only, ending before a leaf input or through a merge's
last message, and stops at the first unbuilt message. Eight queued summary
workers may run concurrently. A leaf can start only when fewer than eight
preceding leaves are unbuilt; a parent needs both children. No periodic work
scan or cache keep-alive runs.

Short sources become verbatim nodes without a model call. Semantic nodes aim
for 512 bytes. Summary tasks use the byte ruler and up to five correction
attempts in the same conversation, retaining the shortest candidate. Slight
oversize is allowed as in the gist and measured at its actual byte size. Failed
jobs become eligible again on a subsequent message.

Each independent user request waits for previous leaf summaries, receives the
saved view before its own text, and starts fresh model context. Messages arriving
during active work steer that run between tool calls. The durable logical
conversation and provider cache identity survive resets and restarts. Pi's
ordinary automatic compaction is disabled. The shared instructions and tool
schemas stay fixed for foreground and summary calls; summary tools are blocked
before execution.

Only `zoom(id,n)` and `date(id)` navigate memory. A range opens into its two
children; a leaf returns the original message in UTF-8-safe pages. On its first
page, an owned tool result also restores its original images from the durable
transcript. Thoughts are excluded. Tool-result text retains a combined head and
tail of 30,000 Unicode characters; other long text splits losslessly into
consecutive messages. Full durable tool results remain separate from that
intentional memory-log clipping.

## Provider implementations

The active OpenAI path uses the existing Codex subscription through Pi's
`openai-codex` adapter. A private Dojo credential store is seeded from the local
Codex login. Borrowed access tokens are read from their owning app without
copying or rotating its refresh token. Renewal happens in that app, or through
an independent Dojo login. There is no automatic API-key fallback. The current subscription endpoint rejected explicit
`prompt_cache_options` on both tested models. Its adapter therefore uses a
stable cache key and four-line view blocks as separate input-message boundaries
for supported implicit cache reuse. Cache hits remain provider-dependent.

The Anthropic adapter preserves Claude's required OAuth identity text, divides
views into four-line content blocks, removes default marks, and marks the fixed
system/tools, the last complete view block, and the request end. Subscription
marks request one-hour retention; API-key mode is not enabled by the runtime.
Matching cold-prefix writers wait until a response starts, rather than waiting
for its entire completion. Claude summary calls select Haiku at xhigh effort.
The real Anthropic OAuth payload builder is tested before network dispatch;
a live Claude subscription run requires a Claude login and is not yet verified.

These are provider-specific caching mechanisms around one shared memory
algorithm. The shared prompt preserves the gist's behavior in Dojo wording,
followed by Dojo's existing desktop instructions. No cache-hit percentage,
subscription quota saving, or battery improvement is claimed.

## State and operation

State normally lives in `~/.local/state/dojo/voice/`:

- `memory-v2/` holds the new archive, tree and saved views.
- `pi/pi.sqlite` holds durable runs, original tool results and task checkpoints.
- `pi/credentials.json` holds private credentials or read-only access snapshots.
- `pi/workspace/` is the fixed starting directory, not a machine-access sandbox.
- The previous T3 archive and its imported notes were removed from active state.

The earlier archive was verified against its imported notes before removal.
At the user's request, all 29 imported notes and their derived context were
removed. The remaining setup-test records were subsequently cleared too,
including their durable session context. Dojo starts with an empty archive.
Credentials and settings were retained. Old T3 conversations remain in T3.

Local settings in `config.json` under `pi` select `provider`, `model`,
`summaryModel` and `thinkingLevel`. Supported subscription providers are
`openai-codex`, `openai` and `anthropic`. The voice service preserves the previous
Dojo model selection unless `pi.model` overrides it. Anthropic requires an OAuth
credential; the runtime can seed it from an existing Pi login. Stop Dojo before
changing its credential store. For an independent login, run
`node tools/speech/login.mjs openai-codex`, `openai`, or `anthropic` in an
interactive terminal. The command takes the voice ownership lock and writes
credentials only in Dojo state.

Run `node --test tools/speech/*.test.mjs` for deterministic behavior tests.
Run `node scripts/check-dojo-memory.mjs /tmp/dojo-pi-check` for an isolated live
OpenAI subscription check of source retrieval, restart persistence, fixed
instructions/tools/cache identity and protected-file hashes. The live check
uses clearly labeled synthetic requests and stores evidence in the given folder.
`node scripts/check-dojo-cache.mjs` separately checks cache reuse with a growing
synthetic view and removes its temporary credential snapshot. This is a cache
compatibility check, not a cost or battery benchmark.

Stop `dojo-voice.service` to stop the agent. The archive and durable work remain.
Restarting resumes admitted work. Tool effects are not automatically replayed:
only explicitly safe tools may rerun after interruption. Durable storage does
not make clicks or arbitrary shell commands exactly-once.
