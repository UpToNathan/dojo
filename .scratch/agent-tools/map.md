# Desktop agent tools

## Notes

Dojo has an independent input controller and no model runtime. [The spec](spec.md) defines a local MCP bridge for an existing AI client.

## Decisions-so-far

- [MCP controller](issues/01-mcp-controller.md): retain one seat per subprocess, use the official SDK, and release input on execution failures and EOF. Raw Wayland and GPUI live checks pass.

## Fog

The AI client for registration is not selected. Voice input, screenshots/accessibility observations and the planned agent UI remain separate work.
