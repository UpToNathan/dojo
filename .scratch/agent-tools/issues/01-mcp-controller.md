# Connect agent tools to the independent controller

Type: task
Status: resolved

Implement [the spec](../spec.md), provide client configuration and verify the real MCP subprocess against a nested desktop.

## Answer

Implemented `dojo-agent` with the official Rust MCP SDK. Three tools query desktop context, operate a retained independent controller, and release its seat. Blocking operations execute off the async runtime; calls serialize through session ownership. Invalid arguments preserve the existing seat; execution errors drop it and never retry input.

`cargo check --all-targets` passes. Both server unit tests pass, and the Dojo library suite remains green. Real stdio MCP subprocess runs pass against raw Wayland and GPUI recorders, including held-key release on EOF, failed-action cleanup and independent server seats. Evidence: `/tmp/dojo-agent-nested.l8z12I/events.jsonl` and `/tmp/dojo-agent-nested.TpFmTu/events.jsonl`.

[The client guide](../../../docs/agent-input.md#connect-an-ai-client) documents configuration and scope. Selecting and registering an actual AI client is a separate integration choice; this server does not introduce a model runtime.
