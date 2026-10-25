# Host setup: root via sudo and sshd

Type: task
Status: needs-triage
Blocked by: none

Through the repository's system-config scripts: optionally, for a node that has a YubiKey, install `pam_u2f` and `libfido2` and set up `sudo` to require a touch, with a recovery path documented (no node requires it); and harden `sshd` (public keys only, listening only where intended). Jupiter's `sshd` currently listens on all interfaces with no `PasswordAuthentication` setting, so OpenSSH's default (passwords allowed) applies. Verify from another machine that passwords are refused after the change. Each change is installed with a timestamped backup, per the system-config README.
