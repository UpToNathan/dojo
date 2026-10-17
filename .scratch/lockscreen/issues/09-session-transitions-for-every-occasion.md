# Session transitions for login, logout, suspend, hibernate, restart and shutdown

Status: resolved
Blocked by: 07

Reuse the session transition for the remaining occasions in [the spec](../spec.md). A resident animator in the shell plays the closing with the farewell for logout, suspend, hibernate, restart and shutdown, then runs the action. Add `dojo --leave <occasion>` as the control command used by the system menu and the power keys, and wire the existing system-menu power actions through it. For login, the compositor background is the frame colour, and the shell's first frame is the closed state, then it greets and opens, which also hides shell start-up time.

The action runs through an injectable runner so tests and the nested Niri record it instead of powering the machine off. Real power actions are not run by agents. Keep the existing capability checks from logind and the Niri IPC logout, and keep the existing menu behaviour when the shell is unavailable.

## Acceptance

- Each occasion plays its closing and farewell, then the recorded action runs, checked in the nested Niri with the action runner recording.
- If the shell stalls or the animation is skipped, the action still runs after a bounded delay and is never lost.
- Suspend and hibernate skip or shorten the closing so the machine is not held awake, and resume shows the lock screen when a lock was in force.
- Login opens from the closed state and greets, and the screen before the shell appears is the frame colour.
- Greeting and farewell lines come from the pools in [greetings.md](../greetings.md) and never repeat the previous line for that occasion.
- Real shutdown, restart, hibernate and logout are verified by the human; prepare a checklist script for them.

## Progress

Built: `dojo --leave <occasion>`, closings for logout, suspend, hibernate, restart and shutdown through a recording runner under test, the system-menu wiring, and the login opening. A nested smoke run recorded the shutdown action once. Real power actions were never run; the human checklist is still to be written.

## Comments
