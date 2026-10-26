---
status: proposed
---

# Dojo ships its own display manager instead of using ly or greetd

`dojo-dm` is a root service on the login virtual terminal. It runs the login screen as the lock screen's model in a throwaway Niri owned by an unprivileged `dojo-greeter` user, authenticates through PAM itself, opens the logind session and starts the chosen session, then takes the VT back when the session ends. It replaces `ly`. The login screen has to match the lock screen exactly, with the typed user's theme, greeting and farewell. greetd would still need our own greeter, compositor and handoff, and would add a second daemon whose protocol allows one PAM conversation at a time, which blocks the fingerprint race of ADR 0002.

## Considered options

- **greetd with a Dojo greeter in cage.** Rejected: a new compositor dependency and a visible handoff, and its single conversation per greeter rules out racing fingerprint and password.
- **Boot into a locked session (autologin).** Rejected: the login password never reaches PAM, so the GNOME keyring stays locked, and a session starts before anyone has authenticated.
- **Keep ly.** Rejected: a text-mode login cannot show the lock screen or play farewells and greetings.

## Consequences

- The password reaches the greeter process, as with any graphical login. The greeter user must be unprivileged, and everything it executes (Niri fork, `dojo-greeter`, their libraries) is a root-owned copy under `/usr/local/lib/dojo-dm`, never a path in a user's home.
- The display manager never reads a home directory before authentication. It reads each user's files after dropping to that user, copies a bounded look snapshot into `/var/lib/dojo-dm` with the wallpaper already decoded to raw pixels, and the greeter draws only from snapshots and runs no image decoder.
- The login screen's Niri has no monitor settings of its own. The look snapshot carries the user's saved monitor layout (`displays.json`), and the display manager renders it into a root-owned file that the greeter's Niri configuration includes, so rotation, position and scale match the user's session and the lock screen.
- A fingerprint login cannot unlock the GNOME keyring because no password is entered. Password login keeps unlocking it through `pam_gnome_keyring` in the login stack.
- `ly` leaves the repo's setup. `tty2` becomes a rescue `getty`; a greeter that cannot start must leave the machine with a working login console.
