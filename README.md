# Dojo

## Desktop configuration

This repository is the source for the desktop's devenv inputs and definition,
Home Manager module, Neovim and Rush configs, wallpaper, and custom PAM, Nix,
1Password, systemd, tmpfiles, and udev configuration. `~/.config/devenv` keeps
symlinks to maintained files here; its `.devenv` directory is local generated
cache/state and is not part of the source configuration.

Run `devenv shell` or `devenv eval ...` from this directory. Run
`scripts/install-system-config.sh` to link the maintained system files into
place; it preserves replaced files as timestamped backups. See
`system-config/README.md` for tracked inputs versus generated runtime state.

## Project

Dojo is a Niri desktop shell implemented with GPUI. Build and test it with
`cargo check` and `cargo test` inside devenv.
