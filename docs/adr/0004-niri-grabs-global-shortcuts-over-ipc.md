---
status: accepted
---

# Niri grabs global shortcuts for IPC clients at runtime

The shell implements the GlobalShortcuts portal backend, and Niri does the key capture. An IPC connection sends Niri the keys it wants (`Request::GlobalShortcuts`); Niri matches them after configured and hardcoded binds, consumes both the press and the release, and reports them as `ShortcutActivated` and `ShortcutDeactivated` on the event stream. Grabs belong to the connection and end when it closes, as an agent seat does. This is a patch in `vendor/niri` (`src/global_shortcuts.rs`, about 250 lines with tests), carried through every upstream sync, in the same way as `spawn-on-release`.

## Considered options

- **Generate KDL binds that `spawn` and `spawn-on-release` a `dojo` command into the control socket.** Rejected: it needs no compositor change, but it rewrites a user-owned `config.kdl` and reloads it, cannot report a conflict when the bind is made, costs a process spawn on each edge, and leaves stale binds if the shell dies. It remains the fallback if the IPC patch becomes too costly to carry.
- **Read `/dev/input`.** Rejected: the app can only watch keys, so the focused window still receives them, and it needs the `input` group.

## Consequences

- Configured binds always win. A key already bound in `config.kdl`, a hardcoded key (VT switch, power) or a key grabbed by another connection is refused, and the reply names it.
- Grabs are inert while the session is locked and while a focused client inhibits shortcuts, like any bind.
- The event stream is a broadcast, so every IPC client sees every `ShortcutActivated`. IPC clients can already run any action, so this exposes nothing new, but a future IPC permission model must treat these events as sensitive.
- Without the patched Niri the backend starts, but `NiriLink::connect` fails on the first request and no shortcut works.
