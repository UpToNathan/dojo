# Candidate B: session generation gate

## Problem

Browser Control already owns Playwright, attached tabs, the relay, and the extension. Pi owns durable task intent and stop. The replacement should reuse that path and add the smallest lifecycle seam that can stop one task's browser work. A Pi abort alone is insufficient: started `execute` work is detached, closing its HTTP client does not retract Chrome commands, and the extension can retry after asynchronous recovery. Niri's EventStream can promptly retire a browser lease, but cannot make an external Chrome side effect atomic with human input in the Niri actor.

## Usage (caller's view)

Pi owns one browser lease for each task attempt. The lease binds the Pi task identity to one Browser Control session generation; the existing MCP/Playwright API remains the browser interface.

```ts
const lease = await browserSessions.bind({ taskId, agentId: "pi-voice" });
const result = await browser.execute(lease, async ({ page }) => {
  return await page.getByRole("button", { name: "Continue" }).click();
});
```

On Pi stop, stop the browser lease before waiting for Pi storage or aborting its conversation:

```ts
const stop = await browserSessions.retire(lease, "task-stopped");
// stop reports extension retirement acknowledgment and any unresolved admitted work
await pi.stopConversation();
```

A Niri ownership event uses the same retirement path; the event handler does not schedule tasks or create an approval record:

```ts
niriEvents.onHumanOwnership((event) => {
  void browserSessions.retireBoundTo(event.window, "human-takeover");
});
```

New binding follows the same flow after retirement; a retired generation is never revived. The UI and agent result classify effects as `not_sent`, `in_flight`, `completed`, or `unknown`; a retry of an `unknown` mutation requires a fresh observation and a new explicit task decision.

## Shape

Keep upstream `SessionManager`, Playwright execution, target registry, relay and extension protocol. Add one Dojo lifecycle module at the Pi/browser bridge, plus a small pinned fork patch at the upstream session and extension command boundaries. Do not wrap every Playwright call in new public Dojo APIs.

```ts
type TaskId = string & { readonly __brand: "TaskId" };
type AgentId = string & { readonly __brand: "AgentId" };
type SessionId = string & { readonly __brand: "BrowserSessionId" };
type Generation = number & { readonly __brand: "Generation" };

type Lease = Readonly<{
  task: TaskId;
  agent: AgentId;
  session: SessionId;
  generation: Generation;
}>;
type RetireReason = "task-stopped" | "human-takeover" | "agent-closed" | "stream-lost";
type EffectState = "not-sent" | "in-flight" | "completed" | "unknown";
type Retirement = Readonly<{
  extensionAck: "installed" | "unavailable";
  effects: ReadonlyArray<{ commandId: string; state: EffectState }>;
}>;

interface BrowserSessions {
  bind(input: { task: TaskId; agent: AgentId }): Promise<Lease>;
  retire(lease: Lease, reason: RetireReason): Promise<Retirement>;
  retireBoundTo(window: NiriWindowIdentity, reason: RetireReason): Promise<void>;
}

interface SessionManager {
  execute(session: SessionId, generation: Generation, code: string): Promise<ExecuteResult>;
  executeCancel(session: SessionId, generation: Generation, reason: RetireReason): Promise<Retirement>;
}
```

The type sketch deliberately omits `NiriWindowIdentity` details: only a proved browser-window mapping may supply it. If no mapping exists, takeover retires all active browser leases. The Pi bridge persists no task history here; it receives task and agent identity from Pi and holds only current lease bindings. Browser Control's session journal remains driver diagnostics/state, not a second scheduler or approval store.

`SessionManager.executeCancel` is an emergency path independent of the per-session execution semaphore. It atomically marks `(session, generation)` retired, rejects queued work, interrupts the worker where possible, closes that generation's CDP client routes, and sends extension revocation. Ordinary reset/delete still use orderly lifecycle locking. A newer task gets a fresh generation; stale cleanup is keyed by generation and cannot release or adopt the newer task's targets.

Every relay command carries session, generation, and command ID. The extension stores a retirement tombstone per session generation and checks it immediately before each `chrome.debugger.sendCommand`, reattach/recovery, retry, fallback, tab mutation, helper command, and response-driven continuation. A tombstone is retained across extension reconnect/service-worker restart for the process lifetime; after state loss, the extension fails closed until the relay rebinds a current generation. Relay response-waiter cancellation is not treated as command cancellation. The acknowledgment means only that the extension installed the tombstone; Chrome calls already made are in flight or unknown and are not undone.

Subscribe to Niri `EventStream` in the bridge. A human ownership event, lock, or stream loss retires the relevant lease immediately (all leases if mapping is absent). This is a fast revocation signal and blocks extension sends after the extension processes it. It does not serialize the prior Chrome send against compositor input. Tab request approval and recording activation must not focus/activate a browser window by default; expose those actions only through an explicit focus request whose UI explains the focus change. Background observe/execute must not invoke upstream activation helpers.

**Niri admission conclusion:** per-effect compositor admission is necessary only if Browser Control must retain issue 16's strict guarantee that no mutation starts after compositor takeover. The generic upstream `execute(code)` accepts arbitrary trusted JavaScript and sends CDP commands from the extension, so an asynchronous Niri watcher cannot provide that guarantee. Routing each effect through an Niri same-actor admission mechanism would require a verified browser-target-to-Niri-window mapping and interception of every mutating CDP/Chrome path; that is not a small session fix, and arbitrary page JavaScript can launch consequences that are not later driver sends. This candidate therefore makes a narrower, honest contract: after extension tombstone acknowledgment, no further extension command for that generation is sent; effects already sent may continue, and EventStream takeover has a propagation interval. If strict post-takeover admission remains mandatory, do not expose Browser Control mutation tools under that claim; implement a separate constrained effect route before making it.

The public surface has three operations: bind, execute, retire. It hides transport retirement, cancellation, stale-generation rejection and command accounting. Callers retain only the task/agent association and the honest in-flight/unknown outcome. This keeps interface depth high without introducing a task runner or new durable policy (per `minimize-reader-load` and `boundary-discipline`).

## Synthesis decision

This is a standalone candidate package for comparison. It chooses one generation gate across Pi, relay and extension rather than a second approval broker or a rewrite of the browser execution stack. The design intentionally does not claim atomic Niri ordering.

## Tradeoffs accepted

- We accept that an admitted Chrome operation can outlive lease retirement in exchange for keeping Browser Control's extension and Playwright as the execution engine.
- We accept asynchronous Niri takeover propagation in exchange for a small event-driven retirement bridge; strict compositor admission would need a constrained, fully intercepted mutation path.
- We accept that arbitrary code inside a started `execute` may keep computing in exchange for promising only that the retired generation sends no later browser commands after extension acknowledgment.
- We accept retaining upstream session diagnostics and transport state in exchange for avoiding another Dojo task scheduler or approval database.
- We accept fail-closed reconnection and conservative all-session retirement without exact window mapping in exchange for avoiding guessed window identity.

## Alternatives considered

- **Kill/restart the MCP child on stop:** exposes transport lifecycle to every caller and does not own the detached relay or retract extension commands. It hides little cancellation complexity and is weaker than generation retirement.
- **Per-effect Niri final-writer gate for generic Browser Control:** could serialize a narrowly defined send with native input, but exposes mapping and effect classification to callers and requires intercepting arbitrary CDP/Chrome effects, which this trusted-code interface cannot guarantee. It loses on interface depth and completeness; a separate constrained route is required if strict ordering is a product requirement.
- **Rewrite the broker and browser executor around Dojo sessions:** would duplicate upstream Playwright, tab adoption and relay machinery, greatly expanding the surface without fixing effects already running inside Chrome.

## Open questions and risks

- Is the narrower asynchronous takeover contract acceptable for Browser Control, or must mutation be withheld until a strict per-effect Niri route exists?
- Can Dojo prove a Chromium window to Niri window mapping for selective retirement, or should any takeover retire all browser leases?
- Which upstream extension mutation paths beyond CDP forwarding (tab create/update/remove, focus, network and recording) are enabled in the chosen tool set, and can unused paths be disabled at registration rather than patched?
- Does Pi's browser bridge need a durable record of unresolved outcomes across Pi restart, or is the existing Pi attempt record plus upstream command journal sufficient?

## Next implementation step

Patch the pinned fork's `SessionManager` with generation tombstones and an out-of-semaphore `executeCancel`, then carry the generation through relay-to-extension forwarding and prove the tombstone checks cover every send and retry path.
