# Pi durable research for Dojo

Research date: 2026-10-11. This is source research, with no dependency installation, implementation, benchmark or runtime test.

## Assessment

`@earendil-works/pi-durable` is a plausible runtime for Dojo's own agent. Its useful part is the combination of persisted conversations, observable execution state, resumable tasks, tool execution policies and provider access. Dojo would still supply the desktop tools, visual UI, voice input and local process lifecycle.

The package labels itself experimental and permits API changes between releases. npm's latest is **1.1.0**, published 2026-10-07. The initial release was 2026-10-01. The inspected main commit is **ea448f454af474f3e2fe9733b331e17b0662f5d6**, dated 2026-10-10. Main already includes breaking changes and additions under Unreleased. Pin a release or exact commit rather than assuming main matches the published package. [README][readme], [package][package], [changelog][changelog], [npm metadata](https://registry.npmjs.org/@earendil-works%2fpi-durable), [inspected commit](https://github.com/earendil-works/pi/commit/ea448f454af474f3e2fe9733b331e17b0662f5d6).

## What it contains

The TypeScript ESM library requires Node >=22.19.0. It uses pi-ai for providers and Chord for typed JSON document state. It does not depend on pi-agent-core or pi-coding-agent. A `Harness` opens a `Session` over storage and schedules work. Conversations hold immutable transcript entries and typed documents. Atomic commits can append entries, edit documents and create tasks together. Entries and observable updates appear after storage commits. [Package][package], [README][readme], [harness implementation][harness].

A submitted input is admitted durably. Generation tasks call a model and create owned tool tasks. Tools return model output plus separate program-facing details; main also adds typed `structuredOutput` and nested `executeTool()` calls. Tools, hooks, prompts and custom tasks belong to named extensions installed in a registry. Stored conversations remember names and configuration; the host must reinstall executable extensions after a restart. Per-conversation choices include model, reasoning, tools, extensions, instructions and working directory. [README][readme], [tool implementation][tool], [harness types][types].

Built-in tools are read, write, edit and bash, with optional image processing. They are generic coding tools. Screenshots, pointer control, app accessibility and desktop coordination require our integrations. Image reading supports model limits and optional resizing; that is not an implementation of screenshot region search or action grounding. [README][readme].

## Recovery semantics and the desktop risk

This is checkpointed execution, not transparent continuation of arbitrary JavaScript or exactly-once external actions. Custom tasks save named phases and JSON checkpoints. An interrupted running phase can run again after reopening. Missing task definitions or failed migrations block scheduling and appear in inspection. The application must make external effects safe itself. [Task definitions][tasks], [scheduler][scheduler], [normative spec][spec].

The tool path has a particularly useful boundary:

1. Validate arguments and apply before-tool hooks.
2. Commit the final arguments and replay policy as execution intent.
3. Execute the external tool.
4. Commit its result.

The default is `replay: "unsafe"`. If a process dies after intent, recovery reports that the tool was interrupted and may have partially run. It does not execute that call again. A safe tool reruns only when both its stored intent and its current registration declare `replay: "safe"`. Main also abandons unfinished child tasks of unsafe callers after a restart. These policies are implemented in code and covered by upstream recovery tests, which this research did not run. [Tool implementation][tool], [upstream recovery tests][recovery].

**Recommendation for Dojo:** keep click, typing, scrolling and drag operations unsafe. An interrupted result should require new observation of desktop state before the agent chooses another action. A click might already have submitted a form even when its result was never committed. The agent could still issue a *new* equivalent click after seeing the interruption; the policy prevents automatic execution replay, not mistaken reasoning. Dojo's coordinator should require fresh state, valid window identity and a current ownership revision on recovery. Restarting the runtime must not replay old desktop coordinates.

Screenshot and context reads can be safe in the execution sense, but their repeated results describe a new screen. Save capture time, dimensions, output scale and target window identity with results. A single generic `desktop_action` tool should remain unsafe even if one action variant is harmless. Separate observations from effects when replay policies differ.

Submission `requestId` deduplicates admission of the same input after a reconnect. Nested tool keys let safe callers reattach to prior child calls. Neither gives arbitrary external services exactly-once semantics. Use target-side operation receipts or idempotency keys where a target actually supports them. [README][readme], [tool implementation][tool].

Generation recovery re-enters a model request and converts committed partial text into an aborted assistant entry. It does not resume the upstream token stream at the exact lost token. Thus recovery may make an additional billable request. Usage totals track reported model and tool usage, including failed attempts, but cannot recover unreported provider billing from a hard crash. Conversations keep provider session identities for cache affinity. [Generation implementation][generation], [README][readme].

## Storage and scale to zero

Available backends are memory, Node SQLite, JSONL and Cloudflare Durable Object SQLite. Local SQLite uses WAL with synchronous NORMAL. It survives process crashes, while the newest commits may be lost on power or host failure. JSONL offers `fsync: true` before commit markers. One process owns a storage; the library supplies no cross-process owner locking. SQLite's connection-level locks do not make two independent harness schedulers safe. [README][readme], [Node SQLite adapter][sqlite].

**Recommendation:** a Dojo-owned Node service can open one SQLite store on demand, install extensions, resume work and stream state to GPUI. Shut down after all relevant work finishes and close the harness/storage. On the next request, reopen the same store. This fits our process-on-demand approach and needs no systemd requirement, but pi-durable does not supply a Unix socket launcher, supervisor, leases or singleton lock.

Be careful with `waitForIdle()`: it ignores background task branches. Use `inspect()` or the full task graph when deciding whether it is safe to exit. Future scheduled waits and background agents need a wake mechanism outside a stopped process. `close()` signals running tasks and waits for their code to return; a tool that ignores cancellation delays shutdown. Read-only observers do not start scheduling. [Harness types][types], [README][readme], [harness implementation][harness].

## Custom Dojo UI and agent state

`conversation.viewState()` returns a disposable Chord state with transcript and documents. `watch()` delivers a current snapshot and exact operations after each commit. `watchEvents()` translates those commits into message, thinking, tool and compaction events. Late observers start with the current state. Slow consumers get a fresh snapshot after more than 100 queued frames, rather than an unbounded event backlog. Default partial text/output commits are throttled to 100 ms, configurable. [README][readme], [events implementation][events].

`pi.live` includes running generation and tools, `pi.inbox` queued inputs, `pi.agent` configuration and `pi.usage` spend. Task graph watches expose execution ownership and status across agents. Custom typed documents can hold Dojo-specific state, with history/fork policies, via `defineDoc`, `snapshot`, `watchDoc` and `documentState`. Foreground and background subagent examples show child conversation ownership, cancellation and persisted reports. [README][readme], [harness types][types].

**Recommendation:** bridge snapshots/events to GPUI over our own local socket. These APIs fit the desired visible thinking, response text, action progress and observation of subagents. UI rendering and transport remain Dojo work. pi-durable cannot automatically discover agent state inside unrelated processes; those applications still need adapters.

## Models, authentication and decision models

The host injects a pi-ai `Models` collection. Providers own credentials, catalogues, streaming and OAuth flows. The inspected pi-ai code implements ChatGPT subscription OAuth in its OpenAI provider and a separate legacy openai-codex provider; its README also lists Claude and Copilot subscription OAuth. This is a statement about Pi's implementation, not a tested guarantee of account access or billing. Persistent credentials are an app-supplied `CredentialStore`; the default is memory. Refresh goes through its serialized `modify` operation. [Pi-ai documentation][ai], [OpenAI provider][openai], [ChatGPT OAuth implementation][oauth].

Pi-ai already includes typed classifier APIs for Jev, Clef and OpenAI Decisions. Main's durable tools/hooks can use `api.models` to access the same catalog. That makes classifier-assisted observation or verification possible inside a tool without calling the main conversational model. It does not automatically select cheap models, crop screens or prove cost savings. Crucially Pi's OpenAI provider filters classifier models out for subscription OAuth because its Decisions adapter requires an API key. Paid decision calls would therefore remain separate from subscription chat access. [Pi-ai classification documentation][ai], [OpenAI provider][openai], [changelog][changelog].

## MCP and the choice among Pi packages

`pi-durable` has no built-in MCP client or dependency. The sibling `@earendil-works/pi-mcp` supports stdio, Streamable HTTP, tool discovery/calls, content conversion and MCP OAuth. Its example wraps MCP tools for pi-agent-core. We can implement equivalent durable registrations for Dojo's existing MCP server, preserving schemas, image content, errors, cancellation and replay policy. MCP metadata alone should not decide whether desktop effects are safe to rerun. [Pi-mcp documentation][mcp], [package][package].

`pi-agent-core` is an in-memory stateful agent loop with tools and streaming events. It is simpler for a short-lived custom agent, but the host must arrange persistence and crash recovery. `pi-coding-agent` is the ready coding application with terminal UI, commands, sessions, packages, SDK and RPC. `pi-durable` is a separate persist-first runtime that suits a custom desktop assistant with recoverable tasks and observable state. These are different adoption choices rather than layers Dojo must all install. [Agent-core documentation][agent], [coding-agent documentation][coding], [package][package].

## Recommended adoption boundary

Keep T3 as the present agent client. For a later built-in Dojo runtime, evaluate a pinned pi-durable service with SQLite, one desktop extension, observation/event streaming to GPUI and the existing desktop coordination layer. Start with a single conversation and unsafe effect tools. Add voice transcripts as ordinary persisted inputs and classifiers only for decisions that reduce paid calls. Resolve shutdown ownership and interrupted-action observation before background agents. The package's week-old release history and active breaking API changes are real maintenance costs; use a small app-owned wrapper so Dojo's Rust/UI protocol does not expose Pi's internal types.

[readme]: https://github.com/earendil-works/pi/blob/ea448f454af474f3e2fe9733b331e17b0662f5d6/packages/durable/README.md
[package]: https://github.com/earendil-works/pi/blob/ea448f454af474f3e2fe9733b331e17b0662f5d6/packages/durable/package.json
[changelog]: https://github.com/earendil-works/pi/blob/ea448f454af474f3e2fe9733b331e17b0662f5d6/packages/durable/CHANGELOG.md
[harness]: https://github.com/earendil-works/pi/blob/ea448f454af474f3e2fe9733b331e17b0662f5d6/packages/durable/src/harness/harness.ts
[tool]: https://github.com/earendil-works/pi/blob/ea448f454af474f3e2fe9733b331e17b0662f5d6/packages/durable/src/harness/tool.ts
[types]: https://github.com/earendil-works/pi/blob/ea448f454af474f3e2fe9733b331e17b0662f5d6/packages/durable/src/harness/types.ts
[tasks]: https://github.com/earendil-works/pi/blob/ea448f454af474f3e2fe9733b331e17b0662f5d6/packages/durable/src/tasks.ts
[scheduler]: https://github.com/earendil-works/pi/blob/ea448f454af474f3e2fe9733b331e17b0662f5d6/packages/durable/src/harness/scheduler.ts
[spec]: https://github.com/earendil-works/pi/blob/ea448f454af474f3e2fe9733b331e17b0662f5d6/packages/durable/docs/spec.md
[recovery]: https://github.com/earendil-works/pi/blob/ea448f454af474f3e2fe9733b331e17b0662f5d6/packages/durable/test/harness-tools-recovery.test.ts
[generation]: https://github.com/earendil-works/pi/blob/ea448f454af474f3e2fe9733b331e17b0662f5d6/packages/durable/src/harness/generation.ts
[sqlite]: https://github.com/earendil-works/pi/blob/ea448f454af474f3e2fe9733b331e17b0662f5d6/packages/durable/src/storage/sqlite/node.ts
[events]: https://github.com/earendil-works/pi/blob/ea448f454af474f3e2fe9733b331e17b0662f5d6/packages/durable/src/harness/events.ts
[ai]: https://github.com/earendil-works/pi/blob/ea448f454af474f3e2fe9733b331e17b0662f5d6/packages/ai/README.md
[openai]: https://github.com/earendil-works/pi/blob/ea448f454af474f3e2fe9733b331e17b0662f5d6/packages/ai/src/providers/openai.ts
[oauth]: https://github.com/earendil-works/pi/blob/ea448f454af474f3e2fe9733b331e17b0662f5d6/packages/ai/src/auth/oauth/openai-chatgpt.ts
[mcp]: https://github.com/earendil-works/pi/blob/ea448f454af474f3e2fe9733b331e17b0662f5d6/packages/mcp/README.md
[agent]: https://github.com/earendil-works/pi/blob/ea448f454af474f3e2fe9733b331e17b0662f5d6/packages/agent/README.md
[coding]: https://github.com/earendil-works/pi/blob/ea448f454af474f3e2fe9733b331e17b0662f5d6/packages/coding-agent/README.md
