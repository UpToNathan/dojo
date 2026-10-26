# T3 Code handoff adapter and `t3-code` capability

Type: task
Status: open
Blocked by: 01

Add the T3 adapter in `crates/dojo-mesh/src/adapter.rs` (thread choice, `T3Open` handoff, source kept) and the receiving side in `handoff.rs` (open the link with the inherited environment cleared; refuse when T3 or the environment is missing). Publish the `t3-code` capability with `environment-id-file`. Tests for the thread ordering, the handoff plan and refusals; build and test the whole main crate, not just `dojo-mesh`.
