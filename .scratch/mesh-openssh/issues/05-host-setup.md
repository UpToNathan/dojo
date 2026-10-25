# Host setup: root via sudo and sshd

Type: task
Status: resolved
Blocked by: none

Through the repository's system-config scripts: optionally, for a node that has a YubiKey, install `pam_u2f` and `libfido2` and set up `sudo` to require a touch, with a recovery path documented (no node requires it); and harden `sshd` (public keys only, listening only where intended). Jupiter's `sshd` currently listens on all interfaces with no `PasswordAuthentication` setting, so OpenSSH's default (passwords allowed) applies. Verify from another machine that passwords are refused after the change. Each change is installed with a timestamped backup, per the system-config README.

## Resolution

`system-config/ssh/sshd_config.d/10-dojo-mesh.conf` with `scripts/install-mesh-sshd.sh` (backup, `sshd -t`, rollback, reload, optional `--listen`, refuses with no authorized key), `scripts/check-mesh-sshd.sh` for the probe from another machine, and optional `scripts/enable-sudo-yubikey.sh` (`register`, `enable`, `disable`; recovery documented in the script). All three are tested by `scripts/test-mesh-host-setup.py` against fakes. Applying and probing on Jupiter is a separate step recorded below when it happens; `pam_u2f` was not exercised against a real key (none exists here).
