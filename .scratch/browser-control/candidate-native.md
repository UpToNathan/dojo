# Candidate A: Niri writes each extension command

## Problem

Reuse `anomalyco/browser-control` at `b4e4238742fa73daabfb664f1108113e9577849d` literally for Playwright, snapshots, target ownership, relay and extension. Pi owns task scheduling, models, durable intent and outcome history. The missing boundary is each outbound browser command, including commands produced later inside one `execute(code)`. A Niri query followed by `chrome.debugger.sendCommand` has a race. The proposed admission point is Niri's nonblocking kernel write of one immutable extension command. Browser execution after that write is explicitly in flight. This satisfies issue 16's transport admission contract, subject to proving this actual route. It cannot promise that Chrome or page effects cease at physical human input.

## Usage (caller's view)

Pi's browser extension uses the existing upstream MCP execute operation. A Dojo wrapper binds every MCP connection to a task lease and owns early retirement.

```js
const browser = await BrowserDriver.open({ owner: pi.taskId, signal: pi.signal });
// Upstream exclusive adoption, after the human attaches this existing tab.
await browser.execute('state.page = await browser.contexts()[0].pages()[0]');
const observed = await browser.execute('return await snapshot({ page: state.page })');
await browser.execute('await state.page.getByRole("button", { name: "Save" }).click()');
// Persist observed/result/unknown in Pi using its existing attempt rules.
```

```js
// Pi.stop and Pi.close call this before storage or conversation teardown.
const stopped = await browser.retire("user-stop");
// stopped distinguishes retired-before-send, in-flight and unknown.
// release adopted tabs without closing them; cleanup is separate from retirement.
```

```text
Coding client: dojo-browser-control mcp --owner codex:<connection-instance>
Pi voice: the browser extension supplies Pi's task identity and generation.
Two clients may own different tabs; a tab cannot have two live owners.
Human activity in a browser client pauses all browser leases until explicit human yield/resume.
```

These are illustrative call sites, not claims that upstream currently exposes `BrowserDriver`. The public adapter has open, execute and retire. Upstream session, Playwright and extension behavior stay behind it.

## Shape

There is one native outbound WebSocket writer for the extension connection. A small trusted Rust broker performs the extension WebSocket handshake, transfers its write descriptor to Niri through the existing external-admission enrollment socket, and retains a read-only code path for extension messages. The broker exposes an authenticated Unix ingress to the pinned upstream relay. The extension connects to this broker endpoint instead of the relay's current extension endpoint. The broker forwards extension events and responses back into upstream `ExtensionRpc`. It has no browser operations, planner or task storage.

The implementation must remove every broker library path that can write WebSocket data, including automatic pong, close and error frames. A read descriptor is a socket duplicate and is technically writable; descriptor transfer alone cannot enforce this against malicious trusted code. The broker is pinned and audited, as the current trusted final-writer broker is. Niri exclusively owns application writes in the implementation. Protocol control frames also pass through Niri, with a separate closed control-frame enum that cannot carry browser commands. No ordinary relay-to-extension bypass remains enabled in Dojo mode.

Descriptor/framing feasibility is concrete. The broker binds a dedicated loopback extension endpoint with the existing extension-origin allowlist and no compression negotiation. It performs only the HTTP upgrade before registration, duplicates the TCP descriptor for a sealed receive-only Rust type, and sends the original descriptor with `SCM_RIGHTS` through the existing enrolled Unix socket. It relinquishes the original immediately and uses `recv` alone on the read wrapper. Niri builds unmasked server WebSocket text frames from the bounded immutable JSON envelope. The existing `navigate_frame` is a masked client frame and must not be reused unchanged. Accept no outbound fragmentation or compression. A write-ready calloop source owns the persistent descriptor. The broker decodes masked extension replies and forwards them to upstream through Unix ingress; it routes protocol pong/close requests to Niri. No Node `ws` socket or its `socket.send` retains the extension connection. The native bridge supplies `ExtensionRpc`'s incoming events and results using its existing handlers, while its outgoing implementation submits bounded commands to the native broker. This replaces transport, not the debugger driver.

Niri owns a persistent channel, rather than accepting a writer descriptor for every command. Its actor receives bounded command envelopes, validates the exact session and channel generations, and attempts one kernel send without an await, lock or library send buffer. A WouldBlock result leaves the command queued and forces a fresh check on the next actor turn. A full write consumes the command ID and marks it in flight. A partial write retires the channel and records unknown; it never sends the remainder or retries. Only one browser command is in flight on this connection. This deliberately bounds already-admitted work beyond the native gate.

The first implementation guards the union of native browser clients. There is no proven existing-tab-to-Niri-window mapping. Niri collects every mapped Helium/Chromium window's Wayland client using configured browser app IDs and live process credentials, then gates against that union. Any human pointer or keyboard activity on a member revokes all browser leases directly inside the human-input actor before normal input delivery. Typing in T3 does not revoke browser sessions. If the browser's native identity cannot be established, broaden to desktop scope or refuse mutation, rather than guess from a tab title or PID alone. Registering new browser windows updates the union before admission; losing identity retires its leases. Registration and dispatch require an enabled, unlocked agent seat, a live enrolled broker/native peer, explicit human yield for the guarded browser clients, and no human-owned guarded client. Use a dedicated browser takeover epoch incremented by every relevant human activity, even if `Coordination::update` leaves its revision unchanged. Mere polling of the existing revision is insufficient when ownership already belongs to the same client. A new lease cannot silently clear human ownership. Lock, channel loss, native-peer retirement and compositor restart fail closed. This scope is conservative across tabs/windows and makes no exact native-window observation claim. App ID alone is classification, not target attestation; unknown clients require the conservative fallback.

```rust
// Existing external_admission.rs, names shown as a sketch.
struct BrowserChannelId { compositor_epoch: u64, connection_generation: u64 }
struct BrowserLeaseId { owner: OwnerId, generation: u64 }
struct CommandId(u64);
struct BrowserCommand {
    lease: BrowserLeaseId,
    channel: BrowserChannelId,
    command: CommandId,
    takeover_epoch: u64,
    intent: IntentId,
    deadline: Instant,
    envelope: BoundedExtensionEnvelope,
}
enum Delivery { Queued, InFlight, Finished(TerminalOutcome) }
enum TerminalOutcome { Refused, Completed, Unknown }
struct BrowserChannel {
    writer: OwnedFd,
    leases: BTreeMap<BrowserLeaseId, LeaseState>,
    pending: VecDeque<BrowserCommand>,
    in_flight: Option<BrowserCommand>,
}
impl State {
    fn register_browser_channel(&mut self, attested: EnrolledChannel) -> Result<BrowserChannelId, Refusal> { todo!() }
    fn enqueue_browser(&mut self, command: BrowserCommand) -> Result<(), Refusal> { todo!() }
    fn try_write_browser(&mut self, channel: BrowserChannelId) -> Delivery { todo!() }
    fn browser_human_activity(&mut self) { todo!() }
    fn retire_browser(&mut self, lease: BrowserLeaseId) -> Retirement { todo!() }
    fn browser_result(&mut self, result: CorrelatedBrowserResult) { todo!() }
}
```

These types stay internal. Niri treats the command envelope as bounded data attested by the enrolled broker, and does not grow a CDP implementation. Exact tab/CDP-child ownership remains upstream's registry invariant. A command binds target ID, tab ID, child CDP session and target generation in the immutable envelope; target replacement consumes the old lease. All browser API operations use the gate, avoiding an incomplete mutating-method blacklist. Internal Playwright helper commands, attachment, grouping, tab creation/removal and recording are included. Operations exceeding the bounded envelope size fail before admission rather than bypassing it.

The extension accepts a command once, verifies its lease/channel/target generations and tombstones, and immediately invokes that command's single Chrome API call. Any await before a Chrome call requires a new admission, not retention of the original envelope. The minimal fork removes upstream automatic debugger recovery/retry and navigation fallback in Dojo mode. If recovery is required, relay code submits a new tagged command through the same gate after fresh observation. Unknown execution never retries. This avoids silently changing one admitted command into several effects. Extension-origin reconnection recovery must also stop automatically reattaching old tabs without current admission.

```ts
// Narrow fork around upstream lifecycle and ExtensionRpc; domain IDs are opaque.
interface BoundExecution { lease: BrowserLeaseId; attempt: IntentId; signal: AbortSignal }
interface Retirement { state: "retired" | "in-flight" | "unknown" }
interface NativeExtensionTransport {
  dispatch(command: BoundExtensionCommand): Promise<CorrelatedExtensionResult>;
  retire(lease: BrowserLeaseId): Promise<Retirement>;
}
// SessionManager retirement never acquires the execute semaphore.
retireSession(lease: BrowserLeaseId, reason: StopReason): Effect<Retirement>;
```

Retirement first tombstones the exact lease in Niri and rejects its pending sends; that acknowledgment defines native stop completion. The relay separately rejects queued execute workers, closes all child CDP routes and marks their late responses unusable. The extension installs a lease tombstone and acknowledges it, rejects continuations and ignores the old generation after reconnect. Startup has no restored live leases. A lost extension acknowledgment produces unknown for the one admitted command, while the native gate still blocks new writes. Native retirement messages never wait for Chrome or the extension to finish. A Chrome error is conservatively unknown unless its semantics prove no effect; a successful correlated response completes transport work without asserting task success. Timeouts, late responses after takeover and process death cannot clear uncertainty automatically. Pi requires fresh observation and its existing outcome resolution before resumed mutation.

`execute(code)` remains trusted Node code. Closing its transports prevents further admitted browser commands, but cannot terminate a synchronous JavaScript loop or undo timers already installed in the page. Keep the native broker outside that event loop so such a loop cannot delay native takeover. Pi stop does not wait for the loop. Sessionless/raw CDP access, unbound CLI execution and optional upstream features that cannot propagate the exact lease must refuse in Dojo mode.

Module changes are deliberately concentrated:

| Location | Responsibility |
| --- | --- |
| Pinned Browser Control source plus small patches | `SessionManager` retirement; carry lease/attempt/target IDs through relay and `ExtensionRpc`; replace direct outbound extension send with native transport; extension single-call/tombstone dispatch; remove automatic recovery and focus activation in Dojo mode |
| `crates/dojo-external-admission` broker and narrow browser transport executable | Extension handshake and read/event forwarding; bounded authenticated relay ingress; existing enrollment and descriptor transfer; no new browser engine |
| `vendor/niri/src/external_admission.rs` and human-input hooks | Persistent extension channel, aggregate native admission, per-command state, bounded writer, direct takeover retirement and correlated outcome barrier |
| `tools/speech/browser-extension.mjs`, `pi-runtime.mjs` | Reuse stdio MCP client; Pi-owned stable task/session generation; early stop/close; existing durable attempt/outcome rules |
| `nix/browser-control.nix`, Home Manager and agent setup | One frozen pnpm source package plus transport package; stable extension path; managed MCP commands and shared Browser Control skill |

Per boundary-discipline, parse and authenticate native packets at ingress. Per separate-before-serializing-shared-state, each task/client owns an independent lease and generation; shared extension transport only serializes physical sends. Upstream target registry remains the sole tab ownership authority. Pi remains the sole durable task authority. The adapter hides transport admission, retries and teardown behind three operations, while exposing unknown outcomes because callers must resolve them.

## Synthesis decision

Candidate only. Parent synthesis selects the base and records adaptations.

## Tradeoffs accepted

- We accept a native transport change and a small upstream fork in exchange for an admission point ordered with compositor human input.
- We accept pausing every browser session on human browser-client activity in exchange for avoiding an invented tab/window mapping. Narrower scope can follow only after exact binding is proven.
- We accept one browser command in flight on the shared connection in exchange for bounded effects after takeover and simple outcome correlation.
- We accept already-admitted Chrome/page effects in exchange for preserving immediate human input. Queued commands remain revocable; admitted effects may be unknown.
- We accept losing automatic recovery and surprise tab/window activation in exchange for one admission per effect and no stolen focus. Task handoff remains explicit and user tabs survive retirement.

## Alternatives considered

A per-command Niri query returning a one-use permit hides less complexity: every extension caller must protect the check-to-send interval across another process. Even consuming that permit once does not order Chrome's call with native takeover. It loses unless its returned response is explicitly the command's admission point, with all later work treated as in flight. The physical writer makes that commitment and its bytes reviewable.

An EventStream subscriber plus extension tombstones has a smaller deployment cost and useful retirement behavior, but exposes a native propagation race to every caller. It can guarantee no command after extension revocation, not no native admission after takeover. Keep the stream for diagnostics and prompt relay cleanup only.

Direct Chromium CDP sockets would fit the existing Navigate prototype more closely, but replace the upstream extension route, weaken existing-profile access and reproduce Browser Control driver work. It loses the user's literal reuse requirement.

## Open questions and risks

- Can the existing trusted broker enrollment accept a persistent extension socket attested to its own transport channel, rather than falsely treating its peer as the exact page process? This requires a new explicitly typed channel registration; `Navigate` and `verify_endpoint(browser_pid)` cannot be reused unchanged.
- Does actual Helium allow the pinned extension at the stable Home Manager path without restarting or altering the live profile? Build and install the concrete artifact first; an unsupported automatic activation remains a human extension-load step.
- Does the actual Chrome route preserve the envelope's single-call semantics through attachment, recording and child sessions? Refuse any unconverted path until measured boundary tests and a live nested Niri run prove it.
- Can a bounded admission/outcome exchange sustain upstream snapshot command volume acceptably? No throughput claim exists in this sketch.

Packaging preserves the upstream manifest key, source pin and lockfile, freezes dependencies and fixes build timestamp identity. Existing-profile tabs require the extension's human attach approval; tab requests show approval without activating the tab or focusing its window. Agent configs point only at the managed local command. OpenCode changes its maintained Home Manager declaration; Pi receives a runtime extension rather than an invented MCP config. Shared setup ownership becomes keyed by file and server name. Remove the old browser adapters/callers after integration; retain native desktop/CUA perception and independently used host-policy helpers. Browser Control's session catalog may retain target ownership metadata, but disable its execute journal in Dojo mode to avoid duplicating Pi task history.

## Next implementation step

Prove a real extension WebSocket command sent by Niri's native actor, with human input paused before write and after write, before migrating callers or advertising mutation capability.
