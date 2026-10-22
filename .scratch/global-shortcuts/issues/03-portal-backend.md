# Portal backend end to end, without a dialog

Type: task
Status: resolved
Blocked by: 01, 02

The shell owns `org.freedesktop.impl.portal.desktop.dojo` and implements
`org.freedesktop.impl.portal.GlobalShortcuts` v2 per [the spec](../spec.md). For now
`BindShortcuts` grants each shortcut's `preferred_trigger` when it is free, and
otherwise returns it unbound. No panel yet.

- Hold one Niri IPC connection and push the union of all sessions' grabs to it; turn
  its events into `Activated` / `Deactivated` for the right session.
- Handle `Close`, callers vanishing from the bus, Niri restarting and the shell
  reconnecting.
- Resolve the `app_id` question in the spec's open questions and fix the store key.

Tests: a private-bus fake frontend drives every method and signal; the nested Niri run
from the spec's verification section passes with a test client.

## Answer

Implemented in `src/shortcuts/{compositor,portal}.rs` and tested on a private bus with a fake Niri socket, and against the real xdg-desktop-portal 1.22.1 frontend (`real_frontend_routes_an_app_to_this_backend`, ignored by default). The `app_id` question is answered in the spec.
