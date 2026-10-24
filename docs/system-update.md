# Update Dojo software

Preview the update plan from this checkout:

```sh
devenv shell -- dojo-update --dry-run
```

Run it with `devenv shell -- dojo-update`. Home Manager also installs `dojo-update`
on PATH after the next `devenv machines deploy "$(cat /etc/hostname)"`.
The runner keeps package-manager prompts attached to the terminal and requests
sudo only through the system package manager.

The default minimum version age is three days. Pi, OpenCode, Codex, Claude,
and T3 Code use their newest upstream versions without this buffer.
Use `--buffer-days 7` to change the default or `--buffer-days 0` to disable it.

`--dry-run` prints commands without fetching versions, acquiring locks, writing
files, or running updaters. Individual helper scripts have read-only dry runs
that do fetch version metadata. A preview does not prove a remote version exists.

## Select update groups

```sh
dojo-update --list
dojo-update --only nix,codex
dojo-update --skip t3code
dojo-update --keep-pins --only nix
dojo-update --buffer-days 0 --only nvim
dojo-update --only dojo
dojo-update --only inventory
```

`--only` selects groups, and `--skip` excludes them. Groups run in a fixed order.
Failed steps return a nonzero exit status, block dependent steps, and appear in
the final summary. Independent groups continue. A per-user lock prevents two
update runs from changing the same installation together.

The Dojo build/install group is opt-in. It compiles the current checkout and uses
the existing release installer. Activate the result with the documented desktop
restart. The updater does not pull this checkout or replace the vendored Niri fork.

## Version selection

- Arch upgrades from one complete snapshot of the official repositories, chosen
  with `--arch-source`. The default, `archive`, is an
  [Arch Linux Archive](https://wiki.archlinux.org/title/Arch_Linux_Archive)
  snapshot from the UTC date before the cutoff, which guarantees at least
  `--buffer-days` full days of delay and has signed databases, but it can download
  slowly. `--arch-source omarchy-stable` is faster: it uses Omarchy's stable channel
  (`https://stable-mirror.omarchy.org`, pinned in `scripts/arch_packages.py`, so no
  Omarchy install is needed). It runs about two days behind current when measured,
  with no published guarantee, and `--buffer-days` does not apply to it. Its
  databases are unsigned. Packages are still checked against the Arch keyring, but
  the mirror could replay older signed packages or withhold updates, so use it only
  if you trust Omarchy. Either way the temporary pacman configuration preserves
  signing settings and repository order, replaces all official mirrors, and
  performs a full system upgrade. Unsupported custom repositories fail the step
  instead of mixing snapshots with current mirrors. Already-installed newer
  packages are not downgraded. The machine's mirror configuration is unchanged,
  and `--buffer-days 0` uses it directly.
- Nix input declarations use default-branch GitHub commits at or before the
  cutoff, except the exempt input names. The helper atomically changes only
  revision strings in `devenv.yaml` and preserves its formatting. Backups live in
  `$XDG_STATE_HOME/dojo-update`, normally `~/.local/state/dojo-update`.
  `devenv update` then refreshes `devenv.lock`, and
  `devenv machines deploy "$(cat /etc/hostname)"` builds and reviews the Home Manager activation.
  A failed input or lock refresh blocks activation. `--keep-pins` retains the
  currently declared revisions rather than selecting new ones.
- Global npm packages use npm's [before setting](https://docs.npmjs.com/cli/v11/using-npm/config/#before).
  The cutoff applies to version resolution for that update invocation.
- Cargo binaries use crates.io publication timestamps. The helper chooses the
  highest eligible stable, non-yanked version and preserves recorded features
  and selected binaries. It does not downgrade newer installed versions.
  Unsupported sources or incomplete install metadata require manual handling.
- Mise tool updates use `--minimum-release-age` and retain previous installs.
- Codex, Claude, and T3 Code run their native `update` commands without an age filter.
  T3 may ask to restart its background service. Skip its group during a session
  where that restart is inconvenient.

Git commit age measures the source snapshot, not every transitive dependency's
release date. Fixed source hashes retain their declared versions. Inspect the
resulting `devenv.yaml` and `devenv.lock` diff before committing it. A failed build
keeps those source changes available for inspection; it does not roll them back.

## Manual and held sources

Helium, Rift, and other source/hash derivations require reviewed version and hash
changes. The privileged 1Password CLI installation, AUR packages, packslip-owned
mise binary, other standalone tools, and the Niri fork remain manual sources.
Lazy's Neovim plugin updates are held while a buffer is enabled because Lazy has
no release-age filter. The summary lists these limits instead of reporting them
as successful updates.

The exact-version installer recipes for the Cargo and npm tools in `devenv.nix`
remain unchanged. Running those recipes later can reinstall their declared
versions. Project dependency lockfiles and firmware are outside this command.

## Keep the system baseline current

The default `inventory` group captures the installed pacman package and enabled
service lists, then refreshes the privileged configuration metadata inventory.
It records the actual state even when an earlier update failed. To check drift
without accepting it, run:

```sh
python3 scripts/capture-system-packages.py --check
python3 scripts/capture-privileged-config.py --check
```

Review these changes with the input-lock diff. See
[the system inventory](system-inventory.md) for exclusions and restore gaps.

## Preview restoration

```sh
python3 scripts/restore-system-baseline
```

The preview installs the official explicit package baseline and additional
maintained setup requirements, applies `scripts/install-system-config.sh`,
deploys Home Manager, and enables recorded system units. The installer includes
the maintained firewalld setup. `--apply` executes this plan and stops on failure.
Run it as the desktop user on an existing Arch host with Nix and devenv installed.

Restoration uses the same three-day Arch snapshot policy by default. Pass
`--buffer-days 0` to use current signed repositories. Foreign packages, user-service enablement, secrets,
boot/storage configuration, Dojo/Niri builds, and unmanaged configuration require
the follow-up described in the inventory. No bootable ISO is defined yet.

## Verify the updater

Run the behavior tests from devenv:

```sh
devenv shell -- python3 scripts/test-dojo-update.py
devenv shell -- python3 scripts/test-refresh-update-inputs.py
devenv shell -- python3 scripts/test-update-cargo-tools.py
devenv shell -- python3 scripts/test-capture-system-packages.py
devenv shell -- python3 scripts/test-capture-privileged-config.py
devenv shell -- python3 scripts/test-restore-system-baseline.py
```

Tests use fixture metadata and mocked remote responses/installers. They do not
update the host. Live helper dry runs can verify remote version selection without
installing software.
