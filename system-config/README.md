# Machine system configuration

These files are the maintained sources for custom files installed under `/etc`
and `/usr/local` on venus. Run `scripts/install-system-config.sh` from the
repository root to make the system paths symlink to these files; existing files
are preserved as timestamped backups before replacement.

That installer also copies `bin/dojo-chromium-theme-policy` and its narrowly
scoped sudoers rule into place. Dojo uses the helper to update only Chromium's
managed browser-color policy, which Helium watches at runtime. The helper and
policy file are not symlinked into the user-writable checkout. The installer
creates the root-owned Chromium policy directory so Helium's FHS wrapper can
mount it at launch.

Chromium reads every regular file in `policies/managed`, including files named
`.backup-*`. The helper stores cleared policies in the sibling
`policies/dojo-backups` directory and migrates only matching root-owned Dojo
backups left by older helpers. `scripts/install-system-config.sh
--chromium-theme-only` installs the helper without changing the active browser-color policy.
Theme Apply updates that policy using the selected background color. It leaves
other managed policies intact.

The Nix graphics updater is the source for generated runtime state such as
`/etc/environment.d/99-nix-gl.conf` and `/etc/opengl-driver`; those generated
files stay outside version control. `pam.d/system-local-login` adds fingerprint
authentication before the normal system-login stack. `nix/nix.conf` contains a
public Cachix signing key, not a secret.

The [package baseline](packages/README.md) records every installed pacman package,
the explicit packages to restore, and foreign-package provenance gaps.
The [service baseline](services/README.md) records enabled system and user units.
`packages/required.txt` adds requirements of maintained setup that might not yet
be installed, including firewalld. The existing system installer invokes the
maintained firewalld setup helper.

The [privileged configuration inventory](../docs/system-inventory.md) records
installed paths, ownership, modes, and their maintained sources without storing
credentials or generated host state. `scripts/restore-system-baseline` previews
package installation, maintained configuration setup, service enablement, and
Home Manager deployment. See [the updater guide](../docs/system-update.md) for
execution and verification. These manifests are a restore baseline and a list
of remaining gaps for a future ISO, not a complete machine image.

The system-config installer also runs `bin/configure-firewalld`. It enables
firewalld with the `drop` zone as the default, closing unsolicited inbound
traffic on regular interfaces, and assigns `tailscale0` to the trusted zone so
tailnet connections remain available. On Arch based systems the setup installs
the `firewalld` package when needed.
