# Notify when the agent pauses

Status: resolved
Type: task
Blocked by: 01

In `src/bin/dojo-agent.rs`, at the `InputPaused` sites, send one notice per pause using `dojo::notify`.

## Result

`src/bin/dojo-agent.rs` has a `PauseNotice` flag held in `DesktopSession` (the place that holds the controller). At both `InputPaused` sites (`DesktopSession::action` and the capture-action path in `service.rs`) `announce()` posts one Normal notice (kind `agent`, "Agent paused", "You took the keyboard and pointer, so the agent stopped. It carries on when you hand them back.") through `dojo::notify::post`, and repeated paused results do nothing more. The flag clears when an action goes through, a capture action succeeds, the controller is released, or an input failure drops it. Under `cfg(test)` nothing is posted, so the existing paused-input test does not notify for real.

Verified: `cargo test --bin dojo-agent` (43 pass, including `one_notice_per_pause` and the notice's fields) and `cargo check --bins`.
