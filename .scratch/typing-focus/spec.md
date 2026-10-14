# Preserve focus while typing

Focus follows mouse continues to select windows and outputs when keyboard input
is idle. While a primary-seat key is held, and for 750 ms after keyboard
activity, pointer motion must not change window, output, or layer keyboard
focus. Explicit clicks and keyboard focus commands still work immediately.
Resume on subsequent pointer motion; do not move focus merely because a timer
expires. Independent agent-seat typing must not suspend human mouse focus.

Implement in the maintained Niri fork, verify with compositor fixtures, and
install for the next session without terminating the operator's current login.
