# Shortcut model and persistence

Type: task
Status: resolved

Pure Rust, independent of zbus, GPUI and Niri, following [the spec](../spec.md).

- Parse and format the XDG shortcuts trigger syntax (`CTRL+ALT+a`, `LOGO+space`,
  `SHIFT+F5`) and convert to and from Niri key syntax. Reject modifier-only and
  unknown keys at this boundary, with a readable reason.
- Session registry: sessions by handle, per-session shortcuts, grant and revoke.
- Conflict resolution between sessions, and against a list of reserved keys supplied
  by the caller.
- Per-app persisted assignments in Dojo's state directory, read and written atomically,
  with a corrupt file treated as empty and reported.

Tests cover round trips, every rejected input, conflicts and the corrupt-file path.

## Answer

Implemented in `src/shortcuts/{trigger,registry,store}.rs`: XDG and Niri trigger syntax and GPUI key names, the session registry with per-app persisted choices and conflict handling, and an atomically written store that sets a corrupt file aside. Covered by unit tests.
