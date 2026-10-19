# Install it and retire ly

Status: ready-for-agent
Blocked by: 04, 05

Installer for the root-owned release under `/usr/local/lib/dojo-dm`, the `dojo-greeter` user (`sysusers.d`), `dojo-login` PAM files, systemd units for `tty1`, the `tty2` rescue getty, and removal of `ly` from `bootstrap-host`, `system-config` (`pam.d/ly`, the inventory, package lists) and `scripts/enable-keyring-login-unlock.sh`, which targets `dojo-login` instead. Follow `system-config/README.md`. Fixes go in the repo and are applied by its scripts.

## Acceptance

- A fresh host reaches the login screen from `bootstrap-host` alone; `ly` is not installed or enabled.
- The installer is idempotent and keeps timestamped backups of replaced files.
- `dojo-update` still verifies the installation.

## Comments
