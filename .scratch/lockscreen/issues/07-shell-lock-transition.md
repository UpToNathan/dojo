# Session transition for lock and unlock

Status: resolved
Blocked by: 05

Implement the session transition in [the spec](../spec.md) for the lock and unlock occasions, using the shared pure modules `session::transition` and `session::message` from [the design](../design.md). The shell closes its frame over the workspace and shows the farewell, starts the locker as the closing begins, and the locker takes the lock when the closing ends or a watchdog of about 700 ms expires. The locker's first frame is the closed state the shell ended on, drawn by the same code. On a successful unlock the ring completes, the greeting from [greetings.md](../greetings.md) fades in for about a second, and the opening grows back into the desktop frame, which is the shell's own resting frame, before the lock is released. Reduced motion cross-fades.

Share the geometry and drawing between the shell and the locker rather than copying them, so the frames cannot drift. Keep that logic independent of GPUI where it can be.

## Acceptance

- The shell's closed state and the locker's first frame match pixel for pixel on one and two outputs, at fractional scale as well.
- A stalled shell cannot delay the lock past the watchdog.
- A key press or touch during the farewell or greeting skips ahead and never cancels or re-locks.
- The transition runs only for its duration and then parks; nothing animates afterwards.
- The shell's own windows are fully hidden by the end of the closing.
- With greetings and farewells switched off the frame closes and opens with no text.
- Lock-before-suspend does not hold the machine awake for the animation.

## Progress

Built: the shell closes over the workspace on an overlay-layer window with the farewell, starts the locker as the closing begins, and the locker plays the greeting and opening on unlock. Not run: the real lock handoff and the logind `LockedHint` watch end to end.

## Comments
