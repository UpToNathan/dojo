# Privileged configuration inventory

`system-config/inventory/privileged-files.json` records configuration metadata under `/etc` and `/usr/local`. It contains file paths, numeric owners, modes, file types, package ownership, and maintained repository sources. It contains no file contents, content hashes, external symlink targets, timestamps, or host identity.

`scripts/capture-privileged-config.py` refreshes the snapshot. `--output PATH` selects another destination. `--check` compares current metadata with the snapshot, exits 1 on drift, and writes nothing. The script uses Python's standard library and the installed `pacman` command. It changes no system files and invokes no sudo commands.

The capture reads `pacman -Ql` for package ownership and `pacman -Qii` for backup-file statuses. Unmodified package-owned files are omitted because packages recreate them. Modified, missing, and unreadable package backup files remain visible. Unowned local files remain visible even when a package created them indirectly.

The script reads `config_links` and `config_copies` from `scripts/install-system-config.sh`. Those arrays remain the source of truth for maintained targets. Repository-managed entries include their source and installation method. Symlink entries also report whether the source matches the current checkout. Copy entries record metadata but do not compare contents.

`system-config/bin/configure-firewalld` installs and enables firewalld, sets the default zone to `drop`, and assigns `tailscale0` to the trusted zone. The inventory annotates those two configuration actions when the corresponding files exist. Other settings in those files, custom zones, and custom rules require separate review. The helper does not recreate arbitrary firewall customizations.

## Exclusions and inspection gaps

Private directories appear as aggregate paths. The capture never lists Wi-Fi connection names, SSH key names, VPN profiles, or credential filenames. Known account databases, host identity, disk mappings, keys, and credential files are excluded. Maintained installer targets inside an excluded directory remain visible as metadata.

Generated certificate stores, graphics state, package signing state, the active Chromium theme policy, resolver backups, lock files, and timestamped local backups are excluded. `excluded` reports the categories encountered. The exclusion rules live in the capture script.

The scanner does not follow directory symlinks. It records each symlink's metadata without traversing its target. Unreadable directories and missing maintained files appear in `inspection_gaps`. A successful capture can contain gaps; its console output reports their count. Running as the normal user preserves those gaps instead of claiming a complete root inspection.

## Restore and future ISO gaps

The snapshot identifies work still needed to reproduce the host. It is not a restore archive. Package manifests recreate package files, and `scripts/install-system-config.sh` installs the maintained privileged sources with backups.

The remaining gaps include:

- Reviewed source files for modified package backups and local files that belong in the repository.
- Separate provisioning for private credentials, account databases, disks, and host identity.
- Root inspection of the directories in `inspection_gaps`.
- Content verification for installed copies and local files. Metadata drift checks cannot detect edits with unchanged metadata.
- Configuration outside `/etc` and `/usr/local`, including boot configuration, root's tools, firmware, and application state.
- A clean-machine restore test, bootable image definition, disk layout, and bootloader setup.

`scripts/test-capture-privileged-config.py` verifies classification, installer parsing, missing-file reporting, and exclusion of fixture secrets and Wi-Fi names.
