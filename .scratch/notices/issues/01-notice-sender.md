# Add the notice sender

Status: resolved
Type: task

Build the shared sender in [the spec](../spec.md): `dojo::notify` (`Notice`, `send`, one reused session-bus connection, fallback to the bus path) and the `dojo-notify` command with `send`, `volume` and `brightness`. Pure builders and edge-detection helpers are testable without a bus; the call is tested against a private bus. Home Manager installs the command.

## Result

`crates/dojo-notify` (standalone crate, its own lockfile, like `dojo-mesh`): `Notice`, `Urgency`, `send` (blocking, one reused connection, reconnects once, falls back to `/run/user/$UID/bus`), `send_to` (explicit bus, for tests) and `post` (worker thread, ordered, never blocks the caller). The root crate re-exports it as `dojo::notify`. `dojo-notify` runs `send`, `volume up|down|mute [--step N]` and `brightness up|down [--device D] [--step N]`. `scripts/install-helpers.sh` builds and installs it and links it into `~/.local/bin`.

Validation: 7 tests in the crate, including delivery to a fake `org.freedesktop.Notifications` service on a private `dbus-daemon` (app name, icon, urgency, replace key, value, timeout, connection reuse) and an error, not a hang, when nobody owns the name.
