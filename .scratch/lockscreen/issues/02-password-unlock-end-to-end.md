# Lock and unlock with a password end to end

Status: resolved
Blocked by: 01

Build the `dojo-lock` program as the thinnest vertical slice: take the session lock, show the clock and a password pill on every output, and unlock through the password PAM stack. Follow [the spec](../spec.md) and [ADR 0002](../../../docs/adr/0002-unlock-methods-race-in-separate-pam-conversations.md). Reuse [the harness](../spikes/pam/bin/dojo_pam_race.py) for the worker shape, rewritten as Rust with the PAM boundary kept separate from GPUI so the logic can be unit tested.

The PAM service files go in `system-config` and are installed with `scripts/install-system-config.sh`. Read [system-config/README.md](../../../system-config/README.md) first. Use [the recommended service files](../spikes/pam/recommended/) as the starting point.

## Acceptance

- Typing a password and pressing Enter unlocks in the nested Niri; a wrong password shows the wrong state and stays locked. The password never reaches logs, arguments or the environment.
- The PAM worker is a separate process and is killed on unlock or exit.
- Password state is shared across outputs and only the active output takes input.
- The locker pins its Vulkan driver and exits when its connection is lost.
- Agent keys are rejected while locked, and `ChangeVt` still escapes.
- Killing the locker leaves the session locked and a new locker can take over.
- Pure logic (state machine, PAM message handling) has unit tests. Host-bus checks stay behind `#[ignore]` as in `src/system_menu.rs`.

## Progress

Built: `dojo-lock`, the `dojo-lock-pam` helper, the pure reducer and scene. Password unlock, wrong password, the pause, takeover after a SIGKILL, rejected agent keys and exit 75 on a lost connection were exercised in a nested Niri with stand-in PAM stacks. Not run: the real `pam_unix` stack inside the locker (the real password path passed in the PAM spike).

## Comments
