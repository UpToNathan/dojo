# T3 Code backend

`t3-home-manager.nix` exposes the installed T3 Code Electron bundle's CLI as
`t3`. The wrapper uses `~/.local/opt/t3code` and runs the bundled
`apps/server/dist/bin.mjs` entrypoint in Electron Node mode.

`scripts/install-t3code.py` installs the app itself into `~/.local/opt/t3code`
from the `pingdotgg/t3code` GitHub releases. It follows the `stable` channel
unless `--channel` or `--version` says otherwise, refuses a download whose
SHA-256 does not match the checksum GitHub publishes, and swaps the new tree in
by rename, so a failed install leaves the old app. `bootstrap-host user` and
`dojo-update` run it, so a new machine gets the app and an existing one follows
the channel. A running T3 keeps its old files until you restart it.

T3 Desktop owns the local backend. Keep **Settings → Connections → Local
environment** enabled. Dojo does not start a standalone server or disable the
local environment through Home Manager.

The desktop backend and Dojo theme publisher use `~/.t3/userdata`. Themes
are regular JSON files in `themes/`; Apply publishes `dojo.json` atomically
and selects it with the bundled CLI.

The installed desktop's theme lookup requires its primary local environment.
Disabling that environment while connecting to a standalone server lets
conversations work but leaves desktop theme publication disconnected from
the theme lookup.

Do not run a second backend against the same data directory. Existing
threads, providers, and settings remain in `~/.t3/userdata`.

Run `python3 scripts/restore-t3-desktop.py` to remove the previous standalone
autostart entry and enable the local backend. It saves timestamped backups
and leaves the running standalone process until logout because its service
cgroup can contain unrelated agent jobs. Log out and back in before reopening
T3 Desktop with the local backend.
