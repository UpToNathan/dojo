# Animate the independent agent cursor

Type: task
Status: resolved

Implement [the spec](../spec.md). Use finite trajectories, keep input coordinates aligned with the visible cursor and verify in a nested compositor.

## Answer

Added finite quintic trajectories in Niri with travel time between 120 and 550 ms,
scaled by distance. Movement ends with zero velocity and acceleration, and
redirection preserves current position and velocity. The first appearance
copies the human pointer's location without changing human input state. Niri's
animation frame scheduling stops when movement finishes.

Raw Move returns AgentInputMoving. Wait completes at arrival; button and scroll
requests received during movement also wait before dispatch. Dojo's controller
waits before returning from move_pointer and accepts Handled from older forks.
Reset, lock, output removal and seat destruction cancel trajectories and pending
arrival requests. A one-shot deadline completes an outstanding wait if an output
stops presenting frames.

Verification:

- All 214 Niri library tests pass, including interpolation, velocity continuity,
  deferred click delivery, human pointer isolation and reset cancellation.
- The controller serialization test and both MCP adapter tests pass.
- The default-feature release build succeeds.
- Raw nested motion scenario at scale 1.75 passes, including actual intermediate
  events, arrival before click, redirection, reset and disconnect during movement.
  Evidence: /tmp/dojo-agent-nested.zEHnlz. Rendered cursor inspected in the first
  run's motion-mid screenshot at /tmp/dojo-agent-nested.YxYOuN.
- MCP/GPUI lifecycle scenario at scale 1.75 passes, including idle shutdown,
  cursor removal, restart and crash cleanup. Evidence: /tmp/dojo-agent-nested.PcpvHY.

Installed release a90195c8b2343d02 for the next UWSM Niri login; retained previous
932a187dbbffda45. The active compositor remains PID 1112 on fdaa18e74a348416.
