# Farewell for every shutdown, restart, hibernate and suspend trigger

Status: ready-for-agent

Make the closing play whatever starts the occasion. Extend `dojo-lock-session-bridge` to hold a shutdown delay inhibitor, add `dojo --farewell <occasion>` (the closing with no action), bind the power key in Niri and stop logind handling it, route stasis's suspend step and the lid through the closing, and keep the shell's own actions from closing twice. The login screen gets power shortcuts that play the same closing.

## Acceptance

- Recording-runner runs show one closing per trigger and no second closing for the shell's own actions.
- If the shell is absent or stalled the inhibitor is released and the machine still powers off or sleeps within logind's delay.
- Real power actions are never run by agents; add them to the human checklist.

## Comments
