# Farewell for every shutdown, restart, hibernate and suspend trigger

Status: ready-for-agent

Make the closing play whatever starts the occasion. Extend `dojo-lock-session-bridge` to hold a shutdown delay inhibitor, add `dojo --farewell <occasion>` (the closing with no action), bind the power key in Niri and stop logind handling it, route stasis's suspend step and the lid through the closing, and keep the shell's own actions from closing twice. The login screen gets power shortcuts that play the same closing.

## Acceptance

- Recording-runner runs show one closing per trigger and no second closing for the shell's own actions.
- If the shell is absent or stalled the inhibitor is released and the machine still powers off or sleeps within logind's delay.
- Real power actions are never run by agents; add them to the human checklist.

## Progress

Built: `dojo --farewell <occasion>` (a farewell-only closing, idempotent while any closing is on screen, replying once the timeline has played), the bridge's shutdown inhibitor and farewell calls, `Departure` with `suspend-then-hibernate` for `--leave`, the logind power-key drop-in, the Niri `XF86PowerOff` bind with `disable-power-key-handling`, and stasis's suspend step through `--leave`. A lock asked for during a suspend or hibernate closing now starts the locker, which the menu's own suspend needed. Not done: the login screen's power shortcuts (greeter workstream). Real power actions were never run; the triggers to check by hand are listed in `09-human-checklist.md`.

## Comments
