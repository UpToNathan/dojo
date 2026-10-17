# Switch the desktop to Dojo's Niri fork

Build the compositor with its default desktop features, including D-Bus,
systemd and screencasting:

```sh
devenv shell -- cargo build --release --locked --manifest-path vendor/niri/Cargo.toml
devenv shell -- scripts/install-niri-fork.sh
```

Optional pins, each an immutable root-owned Nix-store path, are exported into the installation's
`environment.sh`: `DOJO_BROWSER_CONTROL_BROKER_EXE`, `DOJO_BROWSER_CONTROL_MCP_EXE`, `DOJO_SEMANTIC_AGENT_EXE`
and `DOJO_PERCEPTION_INSTALLATION` (the OCR worker, produced by `scripts/stage-perception-installation.mjs`; see
[the task corpus notes](task-corpus.md)). Without the last, `parse_visual_regions` reports the worker as not installed.

The installer validates your existing Niri configuration and installs a versioned
binary under `$XDG_DATA_HOME/dojo/niri`, normally `~/.local/share/dojo/niri`.
It retains Nix roots for its runtime libraries and atomically selects that
version through `current`. A previous installed fork remains selected by
`previous` when you upgrade.

The wrapper `~/.local/bin/dojo-niri-session` enables `DOJO_AGENT_INPUT=1` and
starts the installed binary with `--session`. A user service drop-in overrides
the existing UWSM `niri-session` instance's compositor command. Home Manager
retains these wrapper and drop-in sources. Installation reloads service
definitions without restarting the running compositor.

Save your work, log out, then log into your usual Niri session. That login starts
the fork. Reopen T3 Code so it inherits the new `NIRI_SOCKET` and picks up the
desktop tools. Existing Wayland clients belong to the old compositor connection,
so changing compositor requires a new login.

Confirm the running executable and next-start service command:

```sh
readlink -f /proc/$(pgrep -x niri)/exe
systemctl --user cat 'wayland-wm@niri\x2dsession.service'
python3 tools/agent-input/check-codex.py
```

Before activating a newly built fork, verify its release binary in nested mode:

```sh
devenv shell -- env NIRI_BINARY="$PWD/vendor/niri/target/release/niri" tools/agent-input/run-nested.sh --mcp --gpui --recorder "$PWD/target/debug/examples/agent_seats"
devenv shell -- env NIRI_BINARY="$PWD/vendor/niri/target/release/niri" tools/agent-input/run-nested.sh --fullscreen
```

## Re-pin the semantic agent

The compositor's `dojo-agent` is an immutable, root-owned copy in the Nix store,
selected by `DOJO_SEMANTIC_AGENT_EXE`. Source changes to `dojo-agent` take effect
only after it is re-pinned. Build it in release, add it to the store, and reinstall
with the existing pins carried over:

```sh
devenv shell -- cargo build --release --bin dojo-agent
pin=$(nix-store --add target/release/dojo-agent)
devenv shell -- env DOJO_SEMANTIC_AGENT_EXE="$pin" \
  DOJO_PERCEPTION_INSTALLATION="$(. ~/.local/share/dojo/niri/current/environment.sh; echo "$DOJO_PERCEPTION_INSTALLATION")" \
  scripts/install-niri-fork.sh
systemctl --user restart dojo-voice.service
```

`devenv shell` exports the browser-control pins. Without
`DOJO_PERCEPTION_INSTALLATION` the new installation drops the OCR worker. The
installer keeps the previous installation for rollback: point `current` back at
`previous`. Older store copies lose their roots when installations are pruned;
`nix-collect-garbage` then reclaims them.

## Return to upstream Niri

Remove the Dojo drop-in and reload user service definitions:

```sh
rm -- "$HOME/.config/systemd/user/wayland-wm@niri\x2dsession.service.d/90-dojo-niri.conf"
systemctl --user daemon-reload
```

Log out and back in. UWSM uses its original Niri command. The upstream package
and system login entry are unchanged. To keep this rollback across future Home
Manager activations, remove the corresponding `xdg.configFile` entry from
`home-manager.nix` too.

The experimental session currently hides virtual-keyboard and input-method
globals. Clipboard/IME and client support limits are described in
[the agent input guide](agent-input.md#experimental-scope).
