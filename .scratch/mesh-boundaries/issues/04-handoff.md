# Window handoff

Type: task
Status: needs-triage
Blocked by: 01

Bind keys that send the focused window to a peer: across a boundary in a direction, or chosen in a picker (skipped with one online peer). Identify the window's application, ask its adapter for the window's state, recreate it on the peer near the crossing point, then close the local window; move remote input with it when it is configured toward that peer. Without an adapter, relaunch from the desktop entry and say so. A file the peer lacks refuses the handoff and offers to copy it first. Every handoff is audited.
