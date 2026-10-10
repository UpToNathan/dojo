# Build and nested integration

Type: task
Status: resolved

Wire Dojo to the local GPUI subtree, provide the Niri development dependencies and a repeatable controller demonstration, and verify the behaviors in [the spec](../spec.md) without replacing the running desktop.

## Answer

Dojo now imports the pinned GPUI subtree and local Niri IPC, exposes `AgentController`, and includes repeatable nested raw Wayland and GPUI fixtures. Final `cargo check` and 29 Dojo library tests pass from devenv. Niri's 209 library tests also pass. Live runs verify independent focus, modifiers, repeat, scrolling, reset, lock/unlock and disconnect cleanup. Each final scenario detected 660 cursor pixels and exact restoration of the cursor region after removal.

Evidence: `/tmp/dojo-agent-nested.bKLRNA/events.jsonl` (raw), `/tmp/dojo-agent-nested.aispI8/events.jsonl` (GPUI dynamic seat), and `/tmp/dojo-agent-nested.29Fz09/events.jsonl` (GPUI preexisting seat). Commands and limitations are in [the guide](../../../docs/agent-input.md).
