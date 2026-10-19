# The dojo-dm service

Status: resolved
Blocked by: 02, 03

The root service: take the VT, start the greeter Niri as `dojo-greeter`, read Wayland session entries, run the session with the right environment, wait, close, snapshot, repeat. Includes the failed-greeter limit and the `tty2` fallback.

## Acceptance

- A nested run logs in against a fake PAM stack and a recording session command, and returns to the greeter when it exits.
- Environment variables match the Niri (UWSM) desktop entry's expectations; `XDG_VTNR` and the PAM tty are set.
- `dojo-dm` never starts through devenv or a path under a home directory.

## Progress

Built `dojo-dm` (`src/bin/dojo-dm.rs`, `src/dm/service.rs`): one `poll(2)` loop over the signal descriptor, the greeter socket, the login helpers and the greeter's process. Decisions live in pure cores with tests: `sessions` (desktop entries from a root-owned directory), `memory` (last user and session), `registry` (attempts by generation, first success wins and kills the rest), `cycle` (greeter, session, greeter; three failed greeter starts in 30 s exit 69) and `look` (the snapshot child's pipe format). The greeter starts through `dojo-dm-pam dojo-greeter`; `dojo-greeter` is accepted only by that service and refused by both login services. On `StartSession` the service snapshots the user's look, stops the greeter, waits for its logind session to go, and only then sends the helper `Start`. `tests/dm_service.rs` runs the real `dojo-dm` against a fake PAM stack, a recording session and a stand-in greeter command, as the normal user.

Not verified without a real boot: the VT switch and the greeter's last frame surviving the handover, the greeter session and seat grant under real logind, the DRM master being free for the user's Niri, fprintd and logind D-Bus calls, the identity drop as root, and `dojo-dm` stop while a session runs. The list is in the report of this workstream.

## Comments
