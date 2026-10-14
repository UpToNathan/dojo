# Dojo software updater and system baseline

Provide a `dojo-update` terminal command first. Update the Arch host,
Home Manager/devenv packages, and independently installed tools through their
owning installers. Default to versions at least three days old. Pi, OpenCode,
Codex, Claude, and T3 Code bypass the buffer.

Record every installed pacman package, install reasons, foreign packages, enabled
system/user units, and the metadata/source ownership of privileged configuration.
Connect restoration to the existing system installer and the independently
implemented firewalld setup. Keep credentials, host identity, and generated
runtime state out of source. Establish a baseline for a future ISO without
claiming a complete bootable image.

## Interface and ownership

- `scripts/dojo-update [--dry-run] [--only GROUPS] [--skip GROUPS] [--buffer-days N] [--keep-pins]`
  owns ordering, dependent-step failure handling, summary, and one-run locking.
- `refresh-update-inputs.py` owns YAML commit selection and atomic source changes.
- `update-cargo-tools.py` owns installed Cargo metadata and publication-age selection.
- `arch_packages.py` owns replacement of official mirrors with dated snapshots.
- The capture scripts own package/service and privileged-file manifests.
- `restore-system-baseline [--apply] [--buffer-days N]` previews by default and
  reuses maintained install sources when explicitly executed.

## Design synthesis

Compared a Topgrade configuration/wrapper with a custom command. The custom
command hides the repo's pin-refresh, lock-refresh, and machine-activation chain
behind one operation. Topgrade discovery would still need that custom chain and
adds a dependency without solving release-age selection for it.

Retained native update commands, interactive prompts, independent-step summaries,
and visible unsupported sources from the Topgrade design. An independent review
agreed on the custom design and identified Cargo API pagination for correction.

## Acceptance

Dry-run executes no updater and writes no files. Buffered selectors reject newer
versions. Failed pin/lock refresh blocks Home Manager activation. The restore
preview includes maintained firewalld requirements and the existing installer.
Captured inventory is deterministic and has a read-only drift check. Unreadable
directories, unmanaged configuration, and unsupported update sources are visible.

Verification lives in the six `scripts/test-*` updater/baseline test files and
the procedures in `docs/system-update.md`.
