# Scale the desktop-control worker to zero

Type: task
Status: resolved

Implement [the spec](../spec.md). Keep the MCP adapter responsive while idle and prove actual process and compositor cleanup rather than only adapter bookkeeping.

## Answer

The MCP adapter starts the same executable in worker mode on the first valid action. Only that child owns the Niri controller. It waits for input with a configurable deadline (60 seconds by default), exits on idle or stdin EOF, and drops the controller. The adapter handles child-exit notifications to reap it without a timer or polling loop. Explicit release and execution errors stop and reap the worker. Context and idle reset/release do not start it, and context does not renew the input lease. Failed input is not replayed.

`cargo test --bin dojo-agent --lib` passes all 31 tests. Extended nested raw Wayland and GPUI scenarios pass process absence, idle exit/reaping without MCP traffic, held-key release, seat removal, cursor screenshot restoration, restart, context traffic during expiry, worker crash, repeated release, stdin EOF, and abrupt adapter crash. The GPUI evidence is `/tmp/dojo-agent-nested.yehoMP`. Its crash case uses a 60-second timeout and requires the worker to stop within one second, distinguishing disconnect cleanup from idle expiry.

The rebuilt binary also passes a live host lifecycle check (`/tmp/dojo-agent-worker-host/result.json`) and Codex app-server tool discovery with T3's override shape (`/tmp/dojo-agent-worker-discovery.log`). This active T3 session still owns the previously loaded adapter. Reopen T3 or reload the provider to activate the rebuilt executable; no compositor or shell restart is needed. See [the guide](../../../docs/agent-input.md#other-mcp-clients).
