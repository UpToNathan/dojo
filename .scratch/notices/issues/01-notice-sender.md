# Add the notice sender

Status: ready-for-agent
Type: task

Build the shared sender in [the spec](../spec.md): `dojo::notify` (`Notice`, `send`, one reused session-bus connection, fallback to the bus path) and the `dojo-notify` command with `send`, `volume` and `brightness`. Pure builders and edge-detection helpers are testable without a bus; the call is tested against a private bus. Home Manager installs the command.
