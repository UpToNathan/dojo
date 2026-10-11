# Shared desktop service

Route desktop actions through a local Unix socket service, started directly by the MCP adapter on demand. Systemd is not required. Use one service per Niri socket, with a process lock to serialize concurrent startup. Store the socket and lock in a private directory under XDG_RUNTIME_DIR. Retain the lock file across exits so all startups lock the same inode.

Each adapter connection owns a separate agent session and Niri controller. Session disconnect, release, execution error and idle expiry release only that session's held input and cursor. Context does not create a service or renew a session. Reset does not create an idle seat. The service exits after its last seat is removed and a short startup grace period has elapsed. Session idle timeouts are independent. Service failure releases all seats and no ambiguous action is replayed.

Keep the existing three MCP tools. Report the shared service PID in worker_pid for an active session. Provide a versioned JSON-lines socket protocol so another local client can connect without MCP. Validate handshake names, timeouts and action arguments at the service boundary; bound request size and socket permissions.

Verify simultaneous adapter startup, shared PID, distinct seats, independent idle deadlines, release and crash isolation, service crash/recovery, no wake on context, protocol rejection, and scale to zero with the real nested raw Wayland and GPUI fixtures.
