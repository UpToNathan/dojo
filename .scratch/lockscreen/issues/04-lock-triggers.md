# Lock from the menu, a keybind, idle and before suspend

Status: needs-info
Blocked by: 02

Make the lock screen reachable. Add a Lock item to the system menu (`.scratch/shell-controls/design.md` removed it deliberately because no locker existed), a Niri keybind in `home-manager.nix`, handling for `loginctl lock-session` so stasis and other tools can lock, and lock-before-suspend through a logind `PrepareForSleep` inhibitor. Keep Niri's existing `LockedHint` handling.

Run the locker as a user service with `Restart=on-failure` so a crashed locker is restarted while the compositor keeps the session locked. Start it on demand, not at login.

Opening the lock closes the launcher, tray menus and other temporary shell surfaces. This matches the launcher research, which recommends hiding on lock.

## Acceptance

- Each trigger locks, and a second trigger while locked does nothing harmful.
- Suspend does not complete until the lock is taken, and resuming shows the lock screen and not the desktop.
- The stasis idle action is explicit in `home-manager.nix` rather than inherited from defaults.
- Killing the locker restarts it and the session stays locked throughout.
- Shell surfaces cannot appear above the lock screen.

## Progress

Built: `dojo-lock.service`, a lock keybind (Super+Alt+L), an explicit stasis idle lock, a sleep bridge that locks before suspend, installer steps for the new binaries and root-owned PAM files. Not exercised: stasis idle, real suspend, a real `loginctl lock-session`. Installing needs the user's sudo password.

## Comments
