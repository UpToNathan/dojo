# Nested MCP tests changed the host activity indicator

The apparent persistent repaint is explained by test environment contamination. No graphics/resource production fix is needed for this observation. The parent confirmed that sending Settled to the resident shell stopped GPU work and that the previous screenshot showed the Working indicator.

Independent source evidence identifies the boundary. `src/bin/dojo-agent.rs:327-331` sends Phase::Working before a validated desktop_action runs. The successful action returns without clearing that phase in this method. `shell_event` sends through dojo::control::send. `src/control.rs:199-206` chooses the speech UI socket from XDG_RUNTIME_DIR/dojo.sock, independently of the Niri IPC socket passed through --socket.

`tools/agent-input/run-mcp.py:22-25` starts the MCP process with --socket for the nested Niri compositor but inherits the environment. `tools/agent-input/run-nested.sh:49` overrides WAYLAND_DISPLAY and NIRI_SOCKET for the driver, while inheriting the host XDG_RUNTIME_DIR. Thus the fixture controls nested Niri and sends activity events to the host shell. `src/speech.rs:143-150` deliberately animates Working, and `src/frame.rs:829-831` requests its next animation frame. The host CPU/GPU work therefore follows the requested phase, rather than a leaked picker image or expiry timer.

The parent's remedy isolates MCP runtime and shared worker sockets while preserving an absolute path to the nested Wayland socket. That prevents verification tools from changing host activity state.

Read-only observations were `/tmp/dojo-optimization/repaint-threads-confirm.json` and `/tmp/dojo-optimization/idle-repaint-confirm.json`. The passive GPU assertion was already green when the parent changed the phase, so the overlapping observations cannot establish an independent before/after performance comparison. They make no battery or speedup claim. Probing stopped once the parent identified the contamination. No debugger, host restart, scale change, or graphics production edit was performed during this diagnosis.

`.scratch/efficiency-optimization/check-idle-repaint.py` is a rerunnable passive assertion after a caller establishes Settled phase and no input. It preserves process identity and de-duplicates DRM clients before checking the engine counter delta. Its result alone cannot distinguish a valid Working animation from an unwanted idle repaint, so the phase precondition is required.
