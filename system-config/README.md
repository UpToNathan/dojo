# Machine system configuration

These files are the maintained sources for custom files installed under `/etc`
and `/usr/local` on venus. Run `scripts/install-system-config.sh` from the
repository root to make the system paths symlink to these files; existing files
are preserved as timestamped backups before replacement.

The Nix graphics updater is the source for generated runtime state such as
`/etc/environment.d/99-nix-gl.conf` and `/etc/opengl-driver`; those generated
files stay outside version control. `pam.d/system-local-login` adds fingerprint
authentication before the normal system-login stack. `nix/nix.conf` contains a
public Cachix signing key, not a secret.
