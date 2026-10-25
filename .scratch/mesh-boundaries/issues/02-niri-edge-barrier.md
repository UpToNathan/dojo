# Edge barriers in the Niri fork

Type: task
Status: needs-triage
Blocked by: 01

Teach the Niri fork configurable edge barriers: report over IPC when the pointer pushes past a barrier (with the crossing point). While input is away, hold the local cursor at the edge, hide it, keep reporting relative motion, and route keys away from local clients until released. The lock screen and a lost IPC connection release it. Builds on the interactive move code in `vendor/niri/src/layout/mod.rs` and `src/input/move_grab.rs`.
