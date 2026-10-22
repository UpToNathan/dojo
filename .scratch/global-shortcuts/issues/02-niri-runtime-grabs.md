# Niri runtime shortcut grabs over IPC

Type: task
Status: resolved

In `vendor/niri` and `niri-ipc`, following [the spec](../spec.md).

- A request that replaces the connection's set of grabs, a reply listing refused keys,
  and `ShortcutActivated` / `ShortcutDeactivated` events on that connection's stream.
- Match after config binds and hardcoded binds; consume press and release; pair them by
  key code; no repeat. Inert while locked or inhibited.
- Drop a connection's grabs when its socket closes, as the agent seat is.
- Record the decision in `docs/adr/0004-...` (carrying an IPC patch in the Niri fork)
  and update `.scratch/upstream-sync-*` notes if the sync procedure changes.

Tests: matching, conflict refusal, release after modifier release, lock, inhibitor,
disconnect cleanup. Run the fork's full lib suite.

## Answer

Implemented in `vendor/niri/src/global_shortcuts.rs` with `niri-ipc` types, the server request and the key path in `input/mod.rs`. Recorded as ADR 0004. Niri's lib suite passes (278 tests), including end-to-end key tests on the test input backend for press, release after modifier release, leakage to the client, disconnect and configured-bind conflicts.
