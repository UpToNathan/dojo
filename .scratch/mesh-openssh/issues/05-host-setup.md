# Host setup: root via sudo and sshd

Type: task
Status: resolved
Blocked by: none

Through the repository's system-config scripts: optionally, for a node that has a YubiKey, install `pam_u2f` and `libfido2` and set up `sudo` to require a touch, with a recovery path documented (no node requires it); and harden `sshd` (public keys only, listening only where intended). Jupiter's `sshd` currently listens on all interfaces with no `PasswordAuthentication` setting, so OpenSSH's default (passwords allowed) applies. Verify from another machine that passwords are refused after the change. Each change is installed with a timestamped backup, per the system-config README.

## Resolution

`system-config/ssh/sshd_config.d/10-dojo-mesh.conf` with `scripts/install-mesh-sshd.sh` (backup, `sshd -t`, rollback, reload, optional `--listen`, refuses with no authorized key), `scripts/check-mesh-sshd.sh` for the probe from another machine, and optional `scripts/enable-sudo-yubikey.sh` (`register`, `enable`, `disable`; recovery documented in the script). All three are tested by `scripts/test-mesh-host-setup.py` against fakes. `pam_u2f` was not exercised against a real key (none exists here).

## Applied on Jupiter (2026-10-25)

`scripts/install-mesh-sshd.sh --user nathan --allow-no-keys` ran as root over Tailscale SSH: the drop-in was installed and `sshd` reloaded with its sessions kept. `sshd -T` now reports `PasswordAuthentication no`, `KbdInteractiveAuthentication no`, `PermitRootLogin no` and `AuthenticationMethods publickey`. Jupiter has no `~/.ssh/authorized_keys` yet, so until the roster's `authorize` runs there, `sshd` admits nobody; Tailscale SSH is unaffected.

The remote probe (`check-mesh-sshd.sh`) could not be run against Jupiter's `sshd`, and this is worth knowing: Jupiter's `firewalld` default zone is `drop` on its Wi-Fi interface, so the LAN address times out, and Tailscale answers port 22 on the tailnet address itself (it is Tailscale SSH, not `sshd`). A roster route to Jupiter therefore needs either an open port in `firewalld` for `sshd`, or `sshd` on another port reached over the tailnet address (`install-mesh-sshd.sh --port N`, then a route `100.x.y.z:N`). The configuration check with `sshd -T` stands in for the probe until a route exists.
