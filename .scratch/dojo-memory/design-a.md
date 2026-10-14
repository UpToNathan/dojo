# Candidate A: bounded memory service beside Dojo voice

Keep persistence in a Dojo-owned local service and expose it as private MCP tools to Dojo-launched workspaces. The service owns a per-user Dojo memory directory under `dataDir` (`~/.local/state/dojo/voice` today), while each workspace receives only a narrow MCP client configuration and a short `AGENTS.md` instruction describing when to recall or save. Do not change provider/global agent configuration. The current voice flow already creates a scratch project, launches a root workspace, writes `.mcp.json`/`opencode.json`, and settles a thread after idle. Extend those seams; do not make T3 the durable store.

The durable core is an append-only, framed event log plus a derived binary summary tree and persisted budgeted view. Store only selected durable facts and explicit user preferences as memory events; do not copy whole user/assistant/tool transcripts, tool JSON, or secret-bearing outputs. A small, bounded extract of a current task may be retained only when explicitly useful and scrubbed at the boundary. Use versioned records with sequence number, event kind, UTF-8 byte length, payload, and checksum. A single lock-owning writer appends and flushes before acknowledging. On startup, scan to the last complete valid record and truncate a torn tail; repeated `append(eventId, ...)` returns the original result, making retries idempotent. Persist tree nodes and view via temp-file write, fsync, rename; treat them as rebuildable from committed memory events if interrupted. Never rebuild the view on routine startup: load its persisted node references to keep it stable.

Suggested module boundary:

```ts
type MemoryRecord = { id: string; kind: 'preference' | 'decision' | 'fact' | 'open'; text: string; createdAt: string };
type MemoryNode = { firstId: string; count: number; level: number; text: string; utf8Bytes: number };
interface DojoMemory {
  append(eventId: string, record: MemoryRecord): Promise<void>;
  view(maxUtf8Bytes: number): Promise<MemoryNode[]>;
  zoom(firstId: string, count: number): Promise<MemoryNode[]>;
  idleWork(budget: { maxCalls: number; deadlineMs: number }): Promise<void>;
}
```

Use byte limits for storage and prompt assembly; UTF-8 bytes are not model tokens, so leave a separate conservative prompt-token budget to the caller/provider. Keep nodes small (e.g. 512 UTF-8 bytes) and the rendered view within a fixed byte cap (e.g. 32–64 KB), retaining detailed recent nodes and coarser old summaries. `zoom` opens one stored node to its children or source record. These are the only memory MCP operations; scope server startup and data path to Dojo’s service/workspace, with restrictive file permissions.

Lifecycle: before launching each new T3 thread after an idle settle, ask the service for a bounded view and inject it into that workspace’s `AGENTS.md` or a generated workspace-local memory note. Keep the existing 30-second follow-up behavior within one active conversation; the fresh thread receives the same durable memory view. Offer MCP `memory_search`/`memory_zoom` tools for details. Run compaction only after the agent run, capture, and submission have ended, through an idle queue with one worker, a small call cap and deadline; never delay the next user request. Prefer a queued low-cost T3 summary call if the existing API supports a dedicated background run, but verify that API before relying on it. Otherwise use deterministic extractive summaries and label them as such; do not imply they are model-written. If work misses its deadline, leave pending nodes for the next idle window.
