# Notify when remote input starts and stops

Status: resolved
Type: task
Blocked by: 01

In `crates/dojo-mesh/src/remote_cli.rs`, send notices on the operator's side (`remote`) and on the peer (`inject`, over ssh) by running `dojo-notify` as a subprocess, ignoring its absence. `dojo-mesh` gets no D-Bus dependency.

## Result

`crates/dojo-mesh/src/remote_cli.rs` gained `notify(kind, summary, body)`: it finds `dojo-notify` on PATH or at `~/.local/bin`, starts `dojo-notify send --kind remote-input --urgency low SUMMARY` in its own process group with stdio null, never waits, and ignores every error. No D-Bus dependency was added. Operator side (`remote`): "Controlling NAME" once the capture helper starts, "Stopped controlling NAME" after it is gone. Peer side (`inject`, both the real-hardware path and the older seat path): "NAME is controlling this machine" when input starts and "No longer controlled by NAME" when it ends.

Verified: `cargo test` in `crates/dojo-mesh`. New tests cover the four message builders, the PATH / `~/.local/bin` lookup, and the subprocess helper against a fake `dojo-notify` script that records its arguments (and a missing binary being an error, not a panic). Not tested: a real notice over a real ssh session.
