# Replace private workers with a shared socket service

Type: task
Status: resolved

Implement [the spec](../spec.md). Preserve the compositor's shared pointer coordination and avoid systemd dependencies.

## Answer

Replaced the private stdin/stdout child worker with `dojo-agent --service` and a
versioned JSON-lines Unix socket protocol. The MCP adapter starts the executable
directly, waits for a handshake, and maintains its own session connection. A
service-held file lock prevents duplicate owners per canonical Niri socket.
Startup handshake failures may retry; input actions never replay. The service
runs in a separate process group and can outlive its launching adapter.

Each session owns its Niri controller and idle deadline. Release, disconnect,
execution error and idle expiry remove only that seat. Status does not renew the
lease. The service exits after all controllers are gone, with a 250 ms grace
period. Context and idle reset/release do not start it. The existing `worker_pid`
field reports the shared service PID for an active seat. MCP configuration now
forwards `XDG_RUNTIME_DIR` alongside `NIRI_SOCKET`.

Verified on 2026-10-10:

- `devenv shell -- cargo build --bin dojo-agent` succeeded.
- `devenv shell -- cargo test --lib --bin dojo-agent` passed 34 tests, including
  strict bounded protocol parsing and non-renewing status/error requests.
- Raw Wayland lifecycle passed at `/tmp/dojo-agent-nested.w4KpOa`.
- GPUI lifecycle passed at `/tmp/dojo-agent-nested.dpdMan`.
- Both lifecycle scenarios verify simultaneous startup, one shared PID, distinct
  seats, independent deadlines, execution error and adapter crash isolation,
  survival after the launching adapter exits, service crash/recovery, idempotent release after expiry, held-input
  and cursor cleanup, and socket removal after the last session releases.
- Installed T3 Electron runtime in an isolated app/profile passed at
  `/tmp/dojo-electron-coordination.g73awd15`, including real click, keyboard,
  scrolling, human takeover, retained cyan cursor and release cleanup. The MCP
  executable was unlinked before startup to check binary replacement.
- Codex app-server discovery passed at `/tmp/dojo-shared-service-discovery.log`,
  without a model call or an input seat.
- No test service remains running. The host compositor and Dojo shell were not
  restarted. Existing provider sessions must reload to use the rebuilt adapter.

See [the lifecycle and protocol guide](../../../docs/agent-input.md#shared-service-lifecycle).
