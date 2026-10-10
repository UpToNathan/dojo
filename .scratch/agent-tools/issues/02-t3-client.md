# Connect T3 Code's Codex provider

Type: task
Status: resolved

Register the desktop MCP server for this Dojo project in T3 Code's active Codex provider. Forward the current Niri socket without hardcoding a session-specific socket path. Verify that Codex discovers the server alongside T3's injected MCP configuration. Keep the desktop MCP interface independent for later pi-durable integration.

## Answer

Added `.codex/config.toml` with the built executable, `NIRI_SOCKET` forwarding and the three-tool allowlist. Marked only `/home/nathan/Dojo` trusted in the local Codex config, preserving a timestamped backup. The installed T3 bundle's `codexThreadRuntimeParams` applies a whole `mcp_servers` table as a thread override; the verification script reproduces that merge shape with a disabled T3 placeholder.

`codex mcp get dojo-desktop --json` recognizes the server. `tools/agent-input/check-codex.py` passes through a real ephemeral Codex app-server session, discovers all three tools and reads desktop context with no seat or model calls. Evidence: `/tmp/dojo-t3-registration.log`. Existing provider sessions need to reload their tool catalog; actual input still requires the experimental Niri fork. pi-durable remains future work.
