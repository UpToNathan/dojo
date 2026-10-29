# Notify on battery and power changes

Status: ready-for-agent
Type: task
Blocked by: 01

Diff the host-state `PowerStatus` in `src/frame.rs` (where `cx.set_global(next)` is called) and send the battery notices in the spec. Edge-detect in a pure function with tests: first reading is silent, each threshold fires once per fall and re-arms, plug, unplug and full.
