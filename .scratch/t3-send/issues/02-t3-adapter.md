# T3 Code handoff adapter and `t3-code` capability

Type: task
Status: resolved
Blocked by: 01

Add the T3 adapter in `crates/dojo-mesh/src/adapter.rs` (thread choice, `T3Open` handoff, source kept) and the receiving side in `handoff.rs` (open the link with the inherited environment cleared; refuse when T3 or the environment is missing). Publish the `t3-code` capability with `environment-id-file`. Tests for the thread ordering, the handoff plan and refusals; build and test the whole main crate, not just `dojo-mesh`.

## Answer

`adapter.rs` captures T3 Code windows (app id `com.t3tools.T3Code…`): the environment id from `userdata/environment-id` and the five or so most recent threads, newest by last message, read read-only from `userdata/statev2.sqlite` with the Node inside the app (`node:sqlite`; `t3 thread list` needs a project and the CLI cannot list projects). `Handoff::T3Thread` is opened on the peer with `env -u ELECTRON_RUN_AS_NODE xdg-open t3code://threads/…` and refused when the peer has no T3 Code or the ids are not plain. Nothing closes at the source. The panel's **T** key picks another thread and `dojo-mesh send --thread ID` sends it. The `t3-code` capability needs no code: `[capability.t3-code]` with `environment-id-file` in `mesh.toml`.
