# Desktop agent tools

## Notes

Dojo has an independent input controller and no model runtime. [The spec](spec.md) defines a local MCP bridge for an existing AI client.

## Decisions-so-far

- [MCP controller](issues/01-mcp-controller.md): retain one seat per subprocess, use the official SDK, and release input on execution failures and EOF. Raw Wayland and GPUI live checks pass.
- [T3 client](issues/02-t3-client.md): registered for the active Codex provider through project configuration; real app-server discovery and context calls pass with T3's override shape.

## Fog

T3 Code is the first client; pi-durable is planned later. Voice input, screenshots/accessibility observations and the planned agent UI remain separate work. The current upstream desktop compositor still needs the experimental fork for independent-seat actions.
