# Install T3 Code from the repository

T3 Code is a Dojo dependency (its Codex provider hosts the Dojo desktop tools, and Dojo publishes its theme), but only its `t3` wrapper is tracked. The app itself was extracted by hand into `~/.local/opt/t3code`, so a fresh machine has the wrapper and no app, and `dojo-update` skips it there. This makes the install reproducible.

## Requirements

- `scripts/install-t3code.py` installs T3 Code from its GitHub releases (`pingdotgg/t3code`) into `~/.local/opt/t3code`, the directory the Home Manager wrapper and the desktop entries already expect.
- It follows a channel (`nightly` by default, as the installed app does; `stable` and `preview` too) or one exact `--version`. The channel is decided from the release tag, never from where a file was found.
- It downloads the AppImage for this machine's architecture and refuses to continue unless its SHA-256 matches the digest GitHub reports for the asset, and the release's `SHA256SUMS` when that lists the file. It extracts with the AppImage's own `--appimage-extract`; nothing is mounted or run as the app.
- It records what it installed (tag, asset, checksum) beside the app, so a later run with nothing newer changes nothing, and `--force` reinstalls.
- Replacing is atomic: the new tree is built next to the old one and swapped in by rename, a failed install leaves the old app as it was, and the old tree is removed once the new one is in place. A running T3 keeps working from the old files until it is restarted; the installer says so and never stops it.
- It writes the visible desktop entry with absolute `Exec` and `Icon` paths. The hidden entry that registers the `t3code://` handler belongs to the app and is left alone.
- No machine names or personal choices are in the script. `--prefix` and `--applications-dir` exist for tests and for other layouts.
- `dojo-update` and `scripts/bootstrap-host user` call it, so a machine without the app gets it and a machine with it follows the channel.

## Out of scope

- Patching T3 or building it from source; any change to what T3 does once installed.
- The `t3` wrapper (still Home Manager's) and T3's own data directory.
