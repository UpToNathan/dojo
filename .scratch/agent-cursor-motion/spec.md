# Agent cursor movement

Make the independent cursor move continuously between requested positions. Use compositor frames and distance-dependent travel time, with acceleration and deceleration. Preserve position and velocity when a raw controller redirects a movement. The first appearance starts from the human pointer's position without changing human input state.

Dojo's blocking controller must finish movement before it returns, so subsequent clicks and scrolling reach the intended surface. Raw IPC can start a movement and wait separately. Reset, locking, disconnect and idle worker shutdown cancel movement and release its resources. Stop animation frames when movement ends; retain no background animation service.

Verify interpolation, arrival before clicks, input isolation, cancellation and worker lifecycle. Build and install the full fork for the next login while preserving the active desktop.
