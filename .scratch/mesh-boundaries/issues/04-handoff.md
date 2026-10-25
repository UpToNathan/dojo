# Window handoff

Type: task
Status: needs-triage
Blocked by: 02, 03

On a move released past a boundary: identify the window's application, ask its adapter for the window's state, recreate it on the peer near the crossing point, then close the local window, and move remote input with it. Without an adapter, relaunch from the desktop entry and say so. A file the peer lacks refuses the handoff and offers to copy it first. Every handoff is audited.
