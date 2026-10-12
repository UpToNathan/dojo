# Design judgment

**Recommendation: use B as the base, with two narrow grafts from A.** B fits the current ownership boundary: `TrayService` already owns the bus connection, command receiver, shared snapshots, shutdown channel, and join handle; `TrayHandle` is already the UI-facing API. B can replace the 25 ms command polling and 750 ms rebuild in `run_host` without forcing a new public runtime or migrating all host services at once. Its incremental tray-first step also matches the stated implementation sequence.

| Criterion | A | B | Judgment |
|---|---:|---:|---|
| Idle resource precision | 4 | 5 | Both remove permanent view polling and gate graphics allocations. B names concrete empty-tray behavior, no path scratch for raster scenes, bounded picker previews, and deadline sources. A is equally sound in principle but less explicit about wake/signal mechanics. |
| Smallest maintainable interface | 2 | 5 | B preserves `TrayHandle` and keeps transport private. A adds `DesktopEvents`, `DesktopRuntime`, per-output subscriptions, `TrayCommands`, and a cross-domain runtime before proving they are needed. |
| Cancellation/shutdown | 3 | 4 | A says cancel task tree and join blocking workers, but leaves ownership/task interaction abstract. B gives an interruptible bridge, owner wake, cancellation before join, match cleanup, and bounded D-Bus calls. |
| Protocol/provider correctness | 3 | 5 | B explicitly handles SNI `NewIcon`/`NewStatus`, invalidated properties, owner replacement generations, dbusmenu signals, stale proxies, and fallback repair. A's general signals plus sweep do not spell out these compatibility traps. |
| Practicality against current code | 2 | 5 | B maps directly onto the existing blocking host thread and its mailbox-like command/snapshot responsibilities. A's async actor requires broader proxy, registration, and lifecycle redesign. |

Graft from A: publish tray changes as a **latest-value/coalescing snapshot** so multiple outputs do not queue stale full states; keep each output's image cache and window-local resources independent. Also retain A's one shared host-observation owner as a later step, after the tray boundary works. Do not introduce its generic `DesktopRuntime` now.

Main lifetime pitfalls for B: a signal stream or match must not retain the connection/mailbox after shutdown; cancellation must wake a blocked stream and happen before joins; owner changes must invalidate every proxy, icon, and menu snapshot so delayed invalidations cannot revive old state. Never hold mailbox/publication locks over D-Bus calls. `TrayHandle` outliving `TrayService` should degrade to a closed-command error rather than block. Preserve window-local `Window::drop_image` ownership and reject late picker decode completions after replacement/close. For voice, keep the warmed worker owned by the coordinator across bubble expiry; socket unlink must remain restricted to the lock holder.

B is the stronger proposal, though its zbus 5.19 executor and async-drop details should be verified against the repository's pinned API before implementation. Keep that bridge private and replaceable if connection-level matches cannot reliably cover provider restarts.
