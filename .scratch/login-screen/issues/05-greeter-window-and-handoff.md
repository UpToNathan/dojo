# Greeter program, greeting and handoff

Status: resolved
Blocked by: 01, 03

The `dojo-greeter` mode of the lock program: lock windows in the greeter Niri, the protocol client behind the model's effects, the look swap, the greeting, and a last frame that is the closed state the shell's login opening starts from. Greeting and farewell settings come from the snapshot.

## Acceptance

- Entering `nathan` shows nathan's palette, wallpaper and name.
- A successful login fades to the frame colour and then hands over (the greeting is the shell's, after the swap); a refused or dead `dojo-dm` leaves the greeter restartable.
- A screenshot of the login screen and the lock screen differ only in the two chips and the hint.

## Progress

Built on branch `greeter`. Unit and socketpair tests pass; a nested-Niri smoke against a fake display manager passed (`tools/agent-input/run-greeter.py`). Nothing has run on a real boot.

- [x] `dojo-greeter` is the lock program in login mode. The lock edge moved into `dojo::lock::edge` behind a `Host`; `dojo-lock` and `dojo-greeter` are the two hosts. The window kind, frames, timer and keys are shared.
- [x] `dm::client` connects to `/run/dojo-dm/greeter.sock` (`DOJO_DM_SOCKET` and `DOJO_DM_STATE` in `fake-pam-stack` builds only), sends `Hello`, reads `Init` and reads events on a thread. A server that is not root, a datagram that does not decode, a second `Init` and a closed socket end the greeter non-zero.
- [x] `dm::greeter::Bridge` maps effects to requests (`Spawn` to `Begin`, `Answer`, `Kill` to `Cancel`, `Unlock` to `StartSession`, `Power`, `SwapLook` to `SelectUser`) and events to the model's. A reply for a user who has since changed is dropped. A name outside the protocol's login rules never leaves the greeter: it prompts and refuses, as any unknown name does.
- [x] Looks come from `snapshot::load(/var/lib/dojo-dm, login)` on a loader thread, with the default theme and no greeting for a user with no snapshot. A home directory is never read.
- [x] Handover: the model waits in a still closed frame (0.00% CPU) for `SessionStarted`, then exits 0. `SessionFailed` asks for the password again with the reason as the hint.
- [x] Tab, Ctrl+Tab and chip clicks edit the user and cycle the session; the power chip and Ctrl+Alt+S/H/R/P play the occasion's farewell, then send `Power`.
- [x] The session chip restarts the live conversations, because `Begin` carries the session and `StartSession` does not.
- [ ] A real boot: the greeter in its own Niri on `tty1`, and the last frame handing over to the shell's login opening.


Joint run (branch `login-joint`): `dojo-dm` stops the greeter before it starts the session and never says `SessionStarted` to it, so the greeter now treats a socket that closes after `StartSession` as the hand-over and exits 0 (`Bridge::handed_over`). A `SessionFailed` that arrives after the lock landed, or before it, shows as the hint (`LoginState::refused` holds one until the lock lands); one reason already on the screen stands.

## Comments
