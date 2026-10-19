# Machine system configuration

These files are the maintained sources for custom files installed under `/etc`
and `/usr/local` on the host. Run `scripts/install-system-config.sh` from the
repository root to make the system paths symlink to these files; existing files
are preserved as timestamped backups before replacement.

That installer also copies `bin/dojo-chromium-theme-policy` and its narrowly
scoped sudoers rule into place; the installer fills the rule's `@USER@` with the
account that runs it. Dojo uses the helper to update only Chromium's
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

`pam.d/ly` is the ly login stack. Stock ly lists `pam_gnome_keyring.so` as
optional with a leading `-`, so it is skipped unless the module exists, and it
reaches `system-local-login`, whose `sufficient pam_fprintd.so` ends the auth
phase on a fingerprint before the keyring sees a password. The tracked copy
authenticates with the password only. The Arch `gnome-keyring` package provides
`/usr/lib/security/pam_gnome_keyring.so`, and the module only unlocks a keyring
named `login`. `scripts/enable-keyring-login-unlock.sh` installs the package and
renames an existing default keyring to `login`, keeping a backup;
`scripts/install-system-config.sh` links the ly stack. The keyring password has to
equal the login password, and a fingerprint login or autologin cannot unlock it
because no password is entered. The Home Manager daemon stays and hands over to
the one PAM starts.

## Battery

`scripts/install-system-config.sh` installs `laptop/` only where `/sys/class/power_supply/BAT*`
exists, as root-owned copies (udev rules run programs, so they must not follow a user-writable
checkout). It follows the Arch Wiki's Power management and Laptop pages, with power-profiles-daemon
as the only power tool (the wiki says not to combine it with TLP):

| File | What it does |
| --- | --- |
| `sysctl.d/90-dojo-power.conf` | NMI watchdog off; disk writeback every 60 s instead of 5 |
| `NetworkManager/conf.d/90-dojo-wifi-powersave.conf` | Wi-Fi power saving on (`wifi.powersave=3`) |
| `udev/rules.d/90-dojo-power.rules` | `power-saver` on battery, `balanced` on mains; hibernate at 5% |
| `tmpfiles.d/90-dojo-battery.conf` | charge between 75% and 80% (ThinkPad thresholds) |
| `systemd/logind.conf.d/90-dojo-lid.conf` | lid close suspends then hibernates on battery |
| `systemd/sleep.conf.d/90-dojo-hibernate.conf` | hibernate after 60 minutes of suspend |

The laptop here only supports s2idle sleep, which keeps draining the battery, so the lid action
ends in hibernation (the resume swapfile is configured; `CanSuspendThenHibernate` answers yes).
The charge limit trades about a fifth of the capacity for a longer battery life span: write 100
to `charge_control_end_threshold` to charge fully for a trip, or edit the tmpfiles file.

The idle plan lives in `home-manager.nix` (stasis): on battery the screen dims at 2 minutes, locks at
5, turns off at 5.5 and suspends-then-hibernates at 15; on AC it locks at 5 and turns off at 10.

Considered and not adopted: PCI runtime power management for every device (the only real devices
left on are the MediaTek MT7925 Wi-Fi, known to misbehave under runtime PM, the NVMe, and the iGPU;
the rest are bridges with no driver), changing the ASPM policy (unmeasured, and the same Wi-Fi chip
is sensitive to it), USB autosuspend (already on for all but one device), TLP, and forcing GPU
power levels (conflicts with power-profiles-daemon).

## Lock screen

`pam.d/dojo-lock-fingerprint` and `pam.d/dojo-lock-password` are the two PAM
services `dojo-lock` runs (ADR 0002). `scripts/install-system-config.sh` installs
them as root-owned mode 0644 copies, not symlinks, because a symlink into this
user-writable checkout would let any process running as the user rewrite how the
session unlocks. Re-run the installer after editing them. Refresh the inventory
afterwards with `dojo-update --only inventory`; it has no entries for them yet.

Install the locker with the desktop release:

```sh
devenv shell -- cargo build --release --bin dojo --bin dojo-lock --bin dojo-lock-pam
devenv shell -- scripts/install-dojo-release.sh
devenv machines deploy "$(cat /etc/hostname)"
```

The release installer puts `dojo-lock` and `dojo-lock-pam` beside `dojo`, links
`~/.local/bin/dojo-lock`, and refuses to install if `dojo-lock-pam` cannot load
libpam under the host loader (`scripts/check-dojo-lock-helper.py`). Home Manager
adds `dojo-lock.service` (on demand, never enabled at login; override the Vulkan
pin with a drop-in setting `VK_DRIVER_FILES`), `dojo-lock-session-bridge.service`
(`loginctl lock-session` and a sleep delay inhibitor, because stasis can do
neither), the `Super+Alt+L` bind, a stasis `lock_screen` step after 300 s idle,
and a fallback Niri `background-color`.

Test in the nested compositor (`tools/agent-input/run-nested.sh`), not the live
session. Keep a second TTY ready: Ctrl+Alt+F3, log in, then
`systemctl --user restart dojo-lock` starts a fresh locker that takes the lock
over; unlock it with your password. `loginctl unlock-session` does nothing,
because no program listens for it. Killing the locker leaves the session locked.

Not exercised for real: suspend, lock-before-suspend, the stasis idle step, the
login background colour, and installing the PAM files.

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
