# The dojo-dm service

Status: ready-for-agent
Blocked by: 02, 03

The root service: take the VT, start the greeter Niri as `dojo-greeter`, read Wayland session entries, run the session with the right environment, wait, close, snapshot, repeat. Includes the failed-greeter limit and the `tty2` fallback.

## Acceptance

- A nested run logs in against a fake PAM stack and a recording session command, and returns to the greeter when it exits.
- Environment variables match the Niri (UWSM) desktop entry's expectations; `XDG_VTNR` and the PAM tty are set.
- `dojo-dm` never starts through devenv or a path under a home directory.

## Comments
