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

The Nix graphics updater is the source for generated runtime state such as
`/etc/environment.d/99-nix-gl.conf` and `/etc/opengl-driver`; those generated
files stay outside version control. `pam.d/system-local-login` adds fingerprint
authentication before the normal system-login stack. `nix/nix.conf` contains a
public Cachix signing key, not a secret.
