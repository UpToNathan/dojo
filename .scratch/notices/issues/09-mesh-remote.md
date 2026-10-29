# Notify when remote input starts and stops

Status: ready-for-agent
Type: task
Blocked by: 01

In `crates/dojo-mesh/src/remote_cli.rs`, send notices on the operator's side (`remote`) and on the peer (`inject`, over ssh) by running `dojo-notify` as a subprocess, ignoring its absence. `dojo-mesh` gets no D-Bus dependency.
