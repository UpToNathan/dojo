---
status: proposed
---

# Notices go to the notification service

Dojo reports things that happen on the machine (a battery running low, a Wi-Fi network connecting, a theme failing to apply) as **notices**. Each one could be drawn by the shell on a surface of its own, or handed to whatever owns `org.freedesktop.Notifications`.

Dojo hands them over. A notice is one `Notify` call on the session bus from a small shared module, `dojo::notify`, and a command, `dojo-notify`, for things that are not Rust (the updater, Niri binds, the mesh helpers). Hokum draws them like any other app's, themed from the same palette, with the same Do not disturb, the same per-app rules and the same stacking.

## Consequences

- No second notification surface in the shell, and nothing for the frame to reserve room for.
- Dojo's notices obey the user's notification settings, including muting `Dojo` or letting it through Do not disturb.
- A machine with no notification service loses the notices, and Dojo carries on. Sending never blocks the shell and never fails an operation.
- A notice reaches the service whole or not at all: Dojo does not keep a history, queue or retry; the service owns that.
- A notice's kind rides in the `x-canonical-private-synchronous` hint, which Hokum and the other services that follow the convention use to replace a card in place.
- Notices are edges, not levels: each source reports a change, never a state it is merely in, and says nothing at startup about what was already true.
