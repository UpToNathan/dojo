# Dojo lock screen

Give Dojo its own lock screen. When the session locks, the desktop frame closes in on its centre and becomes the lock screen: the user's themed wallpaper with a clock inside a tick ring (the dial). Fingerprint and password are both live from the first frame and the first to succeed unlocks. Unlocking reverses the lock transition back into the desktop frame. Nothing on screen is a dialog and there is no Unlock button. The first keypress shrinks the dial and grows a password pill, and a touch on the sensor fills the ring.

See [the glossary](../../GLOSSARY.md) for the terms, [ADR 0001](../../docs/adr/0001-lock-screen-is-a-separate-gpui-program.md) for the process model and [ADR 0002](../../docs/adr/0002-unlock-methods-race-in-separate-pam-conversations.md) for authentication. Measurements and spike results are in [grounding.md](grounding.md).

## Experience

The resting lock screen shows the dial, the date, and a one-line hint ("Touch the sensor, or just start typing"). Small chips show the user and network/battery in the top corners, and a lock-age chip plus "Agent input stopped while locked" at the bottom. Nothing else appears: no notifications, media controls or previews.

States: resting, fingerprint reading (ticks fill clockwise, the PAM message is the hint), typing (dial shrinks, pill appears with one dot per character, hint says the sensor still works), wrong (pill shakes and turns red, ring flashes; colour only under reduced motion), password pause (pill dashed with a countdown), and unlock. Colours come from the active `palette.json`, read once at start. The wallpaper behind the dial is pre-blurred when the theme is applied; the locker never blurs live.

Only the active output takes keyboard input, because the compositor focuses one lock surface. Password state is shared across outputs so every screen shows the same dots. Other outputs show a dim clock and take over when the pointer moves there.

## Session transition

One movement serves every occasion: lock, unlock, login, logout, suspend, hibernate, restart and shutdown. The desktop frame closes in, with the workspace opening shrinking and rounding into the dial while the top-bar clock travels to the centre and enlarges. Opening runs the same movement in reverse. Reduced motion cross-fades. Closing occasions show a farewell, opening occasions a greeting, from [greetings.md](greetings.md).

**The shell draws every closing.** Its frame is a layer surface above the windows, so closing the opening hides the workspace with no compositor change. The shell then performs the action: start the locker, log out, suspend, hibernate, restart or shut down. Closing for a lock starts the locker 160 ms before the closing ends (measured at 140 ms from start to lock), so the compositor's swap to the lock screen lands on a finished closing. The closing's clock starts at the cover's first presented frame, not when its windows were requested, and progress is linear in time: every part eases within its own window. The first thing that moves is the frame itself, closing over the top bar's controls and the side panel's icons (these are drawn by the live frame windows under the cover, so the cover fades in over them rather than appearing at once). Then the opening shrinks and rounds with the live desktop still inside it, the desktop gives way to the dial's disc as the opening ends, and only then does the wallpaper fade in around the disc, because the cover can paint an opaque wallpaper only over a covered opening. The cover holds the closed frame until the locker's `locked-at` marker appears in `$XDG_RUNTIME_DIR/dojo/` (or logind's `LockedHint` turns true, which the locker sets itself on `Manager.GetUser(uid).Display`), then drops at once. A watchdog starts the locker once more 700 ms after the closing ends, and a 5 s cap always removes the cover. A shell that is stalled never delays the lock past the watchdog. Lock-before-suspend skips the animation or shortens it so the machine is not held awake.

**The unlock opening is the shell's, drawn over the live desktop.** When the lock is confirmed the shell does not dismiss its closing windows: it parks one per output on the closed state (same disc, wallpaper and clock as the locker's last frame), requests no frames and runs no timers, and writes its pid to `$XDG_RUNTIME_DIR/dojo/overlay-ready` (removed when the overlay is dismissed or the shell exits, and cleared at shell start). The compositor hides the parked windows for as long as the lock lasts. The locker's first frame is still the closed state the closing ended on, with the ring, date, hint and chips fading in over 250 ms from `Locked`, and the farewell follows. A locker that restarts under a held lock, and reduced motion, show them at once.

On a successful unlock the locker asks the host whether the overlay is live (the file exists, its pid is alive and `/proc/<pid>/exe` ends with `dojo`; the decision is a model event, `ShellOverlay`). If so, the greeting is shorter (fade in 150 ms, hold 450 ms, fade out 150 ms), the ring, date, hint and chips fade out with it, and the locker leaves the closed disc and clock, deletes `locked-at`, clears `LockedHint` and unlocks. It plays no opening of its own. If there is no live overlay (shell absent, fallback start, stale file) it plays the greeting (hold 650 ms) and its own opening as before.

The shell watches `locked-at` (and logind's `LockedHint` as a second source). When the marker disappears while the overlay is parked, it opens the parked windows over the now-visible desktop: the same movement as the closing run backwards, about 450 ms, starting at the first presented frame. A short lead (45 ms) takes the progress from the closed state to the point where the wallpaper has gone, so the live desktop shows inside the disc at once; after it progress is linear and every part eases in its own window. The opening grows and rounds out to the desktop frame with the live windows inside it, the clock travels up and shrinks into the top-bar label, the surround releases and the top-bar controls and side-panel icons fade back in; the last frame is the shell's resting frame. Any key press ends it at once, never cancelling it. Reduced motion cross-fades, `transition = false` dismisses at once. A parked overlay has no time cap; one that is never followed by a confirmed lock is dismissed by the closing's 5 s cap.

**The shell draws the login opening.** The compositor background is the frame colour, so before the shell exists the screen already matches. The shell's first frame is the closed state, then it greets and opens. This also hides shell start-up time.

## Greeting

After a successful unlock, the dial's hint line gives way to one short greeting for about a second ("Welcome back, Alyssa", "Onwards, Alyssa"), then the session transition opens the desktop frame. Lines vary with time of day and how long the session was locked, never repeat the previous one, and are listed in [greetings.md](greetings.md). Any key press or touch skips to the transition and never cancels the unlock. It plays inside the unlock transition, so it costs nothing while locked, and it can be switched off.

## Farewell

When the session locks, a short farewell ("Securing the perimeter", "Goodbye, Alyssa") plays over the resting dial: the shell chooses the line and leaves it in `$XDG_RUNTIME_DIR/dojo/lock-handoff` before starting the locker, and the locker fades it in over 150 ms, holds 600 ms and fades it out over 150 ms from the moment it is locked. It never delays the lock, adds no frames afterwards, and any key skips it. Other closings keep their farewell in the shell. Lines and rules are in [greetings.md](greetings.md). It can be switched off separately from the greeting.

## Process and authentication

`dojo-lock` is a separate on-demand program that takes the session lock with `ext-session-lock-v1` and draws with GPUI (ADR 0001). It exits when the lock is released or its connection is lost. A user service supervises it so a crashed locker is restarted while the compositor keeps the session locked.

It runs two PAM conversations at once (ADR 0002): `dojo-lock-fingerprint` and `dojo-lock-password`, installed through `system-config`. Messages from PAM are shown as the hint line. The password never appears in logs, arguments or the environment.

## Triggers

Lock from the system menu, a Niri keybind, `loginctl lock-session` (so stasis and other tools can lock), and before suspend through logind. Opening the lock closes the launcher, tray menus and other temporary surfaces. Agent input is reset on lock by the fork and must stay rejected while locked.

## Power

The lock screen must not be a battery drain. It renders zero frames when nothing is changing and wakes at most once a minute for the clock. Animation runs only for the lock transition, a fingerprint read or a wrong password, never in a loop. After the lock screen settles it dims and the display turns off after a short timeout, waking on input. The fingerprint sensor is armed only while the display is on, unless a measurement shows always-armed is cheap. The locker starts on demand with its Vulkan driver pinned. Verify with frames and wakeups per second, GPU busy, and battery `power_now` for unlocked idle, locked and lit, locked and static, and locked with the display off.

## Open decisions

- **Display-off timeout:** recommend 30 s after the lock screen settles.
- **Fingerprint during a password pause:** the two-stack design leaves it working. Decide whether it should.
- **Fingerprint while the display is off:** recommend arming only when the display is on, accepting that a touch wakes the screen without unlocking.
- **Password dots reveal the password length:** recommend leaving this alone for a local lock screen.
- **Failures on the lock screen count toward the account-wide faillock tally** shared with ly and sudo. Recommend yes, since it is what `system-auth` does.

## Verification

Run in the nested Niri used by `tools/agent-input/run-nested.sh`, never the live session: lock reaches `locked` on every output, killing the locker keeps the session locked and a new locker takes over, agent keys are rejected, `ChangeVt` still escapes, and both unlock methods work. Real fingerprint and real-password checks need the human and are prepared under `spikes/pam/`.
