# Dojo login screen and display manager

Replace `ly` with a display manager that looks like the lock screen. Pick a session (Niri through UWSM), type a username, and the screen becomes that user's lock screen: their palette, pre-blurred wallpaper, name, greeting and farewell. Type a password and the greeting plays, then the desktop frame opens as it does on unlock. Shutdown, hibernate, restart and suspend play the farewell closing whatever triggers them.

Terms are in [the glossary](../../GLOSSARY.md). Decisions: [ADR 0003](../../docs/adr/0003-dojo-ships-its-own-display-manager.md), building on [ADR 0001](../../docs/adr/0001-lock-screen-is-a-separate-gpui-program.md) and [ADR 0002](../../docs/adr/0002-unlock-methods-race-in-separate-pam-conversations.md). The lock screen it copies is in [the lock spec](../lockscreen/spec.md).

## Experience

The login screen is the resting lock screen with two differences. The top-left chip is the user (typed or the last user, prefilled) and a second chip names the session; the hint reads "Type your password". The first key press goes to the password pill, as on the lock screen. Clicking the user chip, or Tab, edits the username; Enter commits it and the look cross-fades to that user's snapshot. A name with no snapshot gets the default theme and a neutral hint. The session chip cycles through the Wayland sessions (the last choice is the default); only `wayland-sessions` entries are offered. Any username may be typed. Wrong password, password pause, reduced motion and the power policy match the lock screen. After a login screen settles it dims and the display turns off on the same timeline.

Fingerprint is offered when the typed user has an enrolled print and a reader exists, and races the password as ADR 0002 describes; otherwise the screen is password only, so a machine without a reader (Jupiter) simply never shows the sensor state. The dial shows the reader state only when it is live. A fingerprint login leaves the GNOME keyring locked; the login password unlocks it.

## Process

`dojo-dm` runs as root on `tty1` (`Conflicts=getty@tty1.service`, `Alias=display-manager.service`). `tty2` is a rescue `getty`. It starts the greeter as `dojo-greeter`: our Niri fork with a greeter config whose only client is `dojo-greeter` (the lock program's login mode), in a PAM session of class `greeter` so logind grants seat access. The greeter talks to `dojo-dm` over a seqpacket socket in `/run/dojo-dm` whose peer must be `dojo-greeter`.

A login attempt is one PAM conversation per method in a root helper, `dojo-dm-pam`, with the service names `dojo-login` (the old `ly` stack) and `dojo-login-fingerprint`. The winning helper becomes the session leader: `pam_setcred`, `pam_open_session`, then fork, drop to the user, set the environment (`XDG_SESSION_TYPE`, `XDG_SESSION_DESKTOP`, `XDG_SEAT`, `XDG_VTNR`, `DesktopNames`) and run the session's `Exec` through the user's login shell. It closes the PAM session when the session ends, and `dojo-dm` starts the greeter again. The greeter's last frame is the closed state, so the shell's login opening continues from it.

Everything the greeter and `dojo-dm` execute is a root-owned copy under `/usr/local/lib/dojo-dm/<release>/` with an `environment.sh`, never a path in a home directory.

## Look snapshots

At session start, session end and `dojo-dm` stop, a child that has dropped to the user reads `palette.json`, `lock-wallpaper.png`, the `[session]` settings and the last greeting (no symlinks, size caps) and hands the bytes to the parent, which writes `/var/lib/dojo-dm/users/<login>/` atomically. The greeter reads only these. A theme change shows at the next login screen after a session ends.

## Power occasions

The shell already plays the closing for shutdown, hibernate, restart, suspend and logout when they start from its menu. Other triggers bypass it: the power key, the lid, stasis's suspend-then-hibernate and `systemctl poweroff`. The logind bridge holds a delay inhibitor for shutdown as well as sleep, and on `PrepareForShutdown(true)` or `PrepareForSleep(true)` asks the shell to play the closing (`dojo --farewell <occasion>`, no action of its own), then releases it. The shell's own actions must not trigger a second closing. The login screen offers the same occasions from a corner chip and keyboard shortcuts and plays them in its own scene. `logind.conf` stops handling the power key so Niri's bind does.

## Open decisions

- **Greeter user and cleanup:** whether `dojo-greeter` is created by the installer through `sysusers.d` (recommended).
- **Failed greeter:** after 3 failed starts in 30 s, `dojo-dm` stops and leaves `tty2` as the console.
- **Autologin:** not offered.

## Verification

Run the greeter and `dojo-dm` against a fake PAM stack and a nested Niri (`tools/agent-input/run-nested.sh`), never the live VT, and never run real power actions. A human checklist covers real login, logout, shutdown, restart and hibernate with `tty2` as the way back.
