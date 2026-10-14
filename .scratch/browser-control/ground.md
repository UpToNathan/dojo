# Browser control: execution map and constraints

Source grounding inspected 2026-10-13. The proposed upstream source is `anomalyco/browser-control` at commit `b4e4238742fa73daabfb664f1108113e9577849d` (package `@opencode-ai/browser-control` 0.8.4). This is a source-based explanation and integration map; it does not claim the extension was installed or cancellation was exercised live. See [ground-lifecycle.md](ground-lifecycle.md) and [ground-install.md](ground-install.md) for detailed findings.

## Overview

Browser Control is a browser driver: a local MCP/CLI client sends Playwright code to a detached relay, which routes CDP commands through a user-installed Chromium extension. It can adopt existing tabs, inspect them, and perform browser actions. The relay and its session journal own driver state; they are not a task scheduler or source of authorization.

Dojo should leave task identity, durable intent, stop, and unknown-effect handling with Pi. Replacing the old browser adapter reuses upstream Playwright, extension, and relay, while a small lifecycle fork adds generation-scoped retirement. That retirement can refuse future work after the extension processes it. It does not by itself make a Chrome command atomic with Niri human input.

## Key concepts

- **Task vs. driver session:** Pi owns the task and stop decision. Browser Control sessions own adopted/created browser targets and JavaScript session state. A session ID is not a Niri ownership grant.
- **Adopted vs. created tabs:** The target registry tracks exclusive adoption of user tabs separately from relay-created tabs. Releasing a session must preserve adopted tabs; cleanup must not close the user's tab.
- **Queued vs. admitted effects:** Before dispatch, queued actions can be refused. Once Chrome accepts a command, the outcome may be in flight or unknown; transport interruption cannot retract it. Require fresh observation and do not automatically retry uncertain mutations.
- **Native takeover vs. explicit handoff:** Browser Control's handoff button is task UI for a target. It is not detection of ordinary human keyboard or pointer ownership in Niri.
- **Native admission:** Niri's final-writer prototype checks ownership and sends in the compositor actor. An asynchronous Niri EventStream mirror can promptly retire browser sessions, but it cannot establish the same atomic ordering for a later Chrome send.

## How it works

1. The client invokes MCP `execute(code)`. `SessionManager.execute` takes a per-session execution permit, starts a detached worker, then runs the code through `ExecuteSandbox`.
2. `ExecuteSandbox` attaches Playwright to the relay using the Browser Control session identity. The relay routes browser/CDP traffic to the extension's shared WebSocket; the extension uses `chrome.debugger.sendCommand` against its attached tab.
3. Session teardown and HTTP disconnect are not emergency cancellation. An interrupted caller cancels work only before it is marked started; reset/delete wait on the execution lifecycle permit. In the extension, commands may await debugger recovery, retry, or use navigation fallback. A stop check must cover each such continuation, not just initial receipt.
4. Pi's `stop()` and `close()` already release desktop clients before storage/harness teardown. Browser retirement belongs at those early lifecycle hooks. The extension must acknowledge that a generation tombstone is installed before Pi can say queued follow-on commands are stopped. Already accepted Chrome work remains in-flight/unknown.
5. Niri tracks human ownership by Wayland client connection and publishes coordination snapshots/stream updates. A Dojo bridge can use them to trigger retirement. Exact browser-tab-to-Niri-window mapping is needed for per-window retirement; without it, stop all browser session mutations on unbound human activity. If the contract requires strict no-start-after-takeover ordering, route every mutation through the compositor's same-actor gate or refuse the mutation.

## Where things live

- Upstream execute/session/MCP lifecycle: `src/mcp.ts`, `src/session-manager.ts`, `src/execute.ts`.
- Upstream routing and extension transport: `src/relay.ts`, `src/extension-rpc.ts`, `extension/src/background.ts`.
- Target ownership and explicit handoff: `src/target-registry.ts`, `src/handoff.ts`.
- Dojo stop and CUA transport-generation patterns: `tools/speech/pi-runtime.mjs`, `tools/speech/cua-extension.mjs`.
- Dojo durable attempts and unknown effects: `tools/speech/desktop-extension.mjs`, `tools/speech/computer-steps.mjs`.
- Niri coordination and compositor admission: `vendor/niri/src/agent_input/coordination.rs`, `vendor/niri/niri-ipc/src/lib.rs`, `vendor/niri/src/ipc/server.rs`, `vendor/niri/src/external_admission.rs`.
- Pin/package and Nix integration findings: [ground-install.md](ground-install.md). Intended package seam is a shared `nix/browser-control.nix` used by devenv and Home Manager, with a stable user path for the unpacked extension and manual pinned MCP registration.

## Gotchas

- Interrupting the Effect around `execute` does not stop arbitrary JavaScript in the relay. Closing the browser transport may cut later browser sends, but not synchronous relay loops or effects already started in a page.
- Relay-side waiter cleanup is not command cancellation. A generation must travel through relay and extension dispatch, including queued input, helper commands, retries, fallback, tab mutations, and focus changes. Sessionless CDP clients must not provide a bypass in Dojo mode.
- A Browser Control tab request can focus the browser window and activate a tab before approval. Browser tab identity does not prove the mapped Wayland window; avoid surprise focus changes and do not infer identity from PID alone.
- Browser Control executes trusted Node code, and its read-only mode only blocks selected CDP input methods; `page.evaluate()` can still mutate. Do not advertise its observations as Niri-authorized desktop receipts.
- The upstream build bundles local code but leaves runtime packages external, so Nix must ship runtime `node_modules` as well as `dist` and extension assets. Its build ID uses the current timestamp; make it deterministic for reproducible builds. Extension distribution is unpacked and user-approved; managed force-install needs a signed CRX/update source that this pin does not provide.
- Evidence here is repository source, issue records, and the supplied user corrections. No external incident or issue-tracker provider was consulted; there is no `docs/adr/` directory in this checkout. No live install, build, browser operation, or cancellation proof is claimed.

The motivating human-priority contract is [issue 16](../cua-adoption/issues/16-side-channel-admission.md): queued effects stop, human input remains immediate, and already-admitted effects are reported as unknown/in-flight without rollback or automatic retry. Permission approval alone is not a coordination lease.
