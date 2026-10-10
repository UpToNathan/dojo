# Desktop agent tools

Expose Dojo's independent input controller to an AI client through a local stdio MCP server, `dojo-agent`. Dojo currently has no model runtime. The server provides the desktop tools for a client that already runs an agent; it does not add voice input or select a model provider.

## Contract

- Read connected outputs and mapped windows without creating an input seat.
- Lazily create one independent seat on the first input action. Retain its controller across tool calls.
- Support focus, pointer movement, clicks, key press/release/tap, scrolling and reset. Use logical output coordinates and Linux evdev codes.
- Serialize actions per server process. Multiple server processes own separate seats.
- Validate action arguments before creating a seat. Report tool execution failures as MCP tool errors. Drop the controller after an execution failure to release any held input; never replay failed actions.
- Explicit release, stdin EOF and process exit close the controller socket. Reset releases input but retains the seat.
- Keep protocol messages on stdout and diagnostics on stderr. Use the official Rust MCP SDK for lifecycle and tool dispatch.
- Inherit `NIRI_SOCKET`, with an explicit `--socket` option for nested sessions. Keep the experimental compositor gate.

## Shape

The server owns an optional `AgentController` behind a mutex. Blocking IPC executes on Tokio's blocking pool. `desktop_context` queries Niri separately; `desktop_action` operates the retained seat; `desktop_release` drops it. The AI client's own UI supplies conversation and approval controls.

## Verification

Test argument parsing, MCP initialization/discovery, persistent seat ownership, errors and EOF. Drive the actual binary against a nested compositor and recorded Wayland windows. Confirm key and pointer delivery and cleanup, preserving human focus.
