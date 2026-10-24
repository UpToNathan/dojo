# Machine system configuration

These files are the maintained sources for custom files installed under `/etc`
and `/usr/local` on the host. Run `scripts/install-system-config.sh` from the
repository root to install them as root-owned copies; existing files are preserved
as timestamped backups before replacement. They are copies, never symlinks into the
checkout, because root reads and runs them (PAM stacks, units, tmpfiles, udev rules,
`update-nix-gl`), and a link would let any process of the checkout's owner rewrite
them. Re-run the installer after editing one. Hosts installed before this change have
symlinks; the next run backs them up and replaces them.

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
authentication before the normal system-login stack. `nix/nix.conf` contains public
Cachix and cache-host signing keys, not secrets (see `docs/nix-cache.md`).

`pam.d/dojo-login` is the display manager's password login stack. Stock PAM
logins list `pam_gnome_keyring.so` as optional with a leading `-`, so it is
skipped unless the module exists, and they reach `system-local-login`, whose
`sufficient pam_fprintd.so` ends the auth phase on a fingerprint before the
keyring sees a password. The tracked copy authenticates with the password only.
The Arch `gnome-keyring` package provides `/usr/lib/security/pam_gnome_keyring.so`,
and the module only unlocks a keyring named `login`.
`scripts/enable-keyring-login-unlock.sh` installs the package and renames an
existing default keyring to `login`, keeping a backup;
`scripts/install-system-config.sh` installs the stack. The keyring password has
to equal the login password, and a fingerprint login or autologin cannot unlock
it because no password is entered. The Home Manager daemon stays and hands over
to the one PAM starts.

ly was the display manager until the login screen replaced it. The tracked
`pam.d/ly` is gone and nothing here installs, enables or removes the `ly`
package; see "Login screen" below for how to go back.

## Login screen

`dojo-dm` replaces ly (ADR 0003): a root service on `tty1` that runs the login
screen, authenticates through PAM and starts the chosen session. What
is installed where:

| Source | Installed as | By |
| --- | --- | --- |
| `pam.d/dojo-login` | `/etc/pam.d/dojo-login`, password only, unlocks the keyring | `install-system-config.sh` |
| `pam.d/dojo-login-fingerprint` | `/etc/pam.d/dojo-login-fingerprint`, fingerprint only, no keyring, no faillock | `install-system-config.sh` |
| `pam.d/dojo-greeter` | `/etc/pam.d/dojo-greeter`, the greeter's own session (class greeter) | `install-system-config.sh` |
| `sysusers.d/dojo-greeter.conf` | `/etc/sysusers.d/`, the `dojo-greeter` system user | `install-system-config.sh`, which runs `systemd-sysusers` |
| `tmpfiles.d/dojo-dm.conf` | `/etc/tmpfiles.d/`, `/var/lib/dojo-dm/users` (root and `dojo-greeter` only) for look snapshots | `install-system-config.sh` |
| `systemd/system/dojo-dm.service` | `/etc/systemd/system/` | `install-system-config.sh` |
| `systemd/system/getty@tty2.service.d/dojo-rescue.conf` | `/etc/systemd/system/getty@tty2.service.d/` | `install-system-config.sh` |
| `dojo-dm/greeter.kdl` | `/usr/local/lib/dojo-dm/<release>/greeter.kdl` | `scripts/install-dojo-dm.sh` |
| `dojo-dm`, `dojo-dm-pam`, `dojo-greeter`, the Niri fork | `/usr/local/lib/dojo-dm/<release>/` plus a `current` link | `scripts/install-dojo-dm.sh` |

All of these are root-owned copies, never symlinks into this checkout or a
home directory, because root and the greeter execute them. A release
directory holds the four binaries, `run` (a three-line wrapper that sources
`environment.sh` and execs `dojo-dm`), `environment.sh` and `greeter.kdl`.
`environment.sh` is written from the installed Niri fork's and desktop's
`environment.sh` (library paths, which must be `/nix/store` paths) and
`roots/` holds GC roots for those store paths, so nothing depends on devenv at
runtime. `dojo-greeter` joins no device groups: its greeter session lets logind
give its compositor the GPU and input devices. If a host proves otherwise, add
the group in `sysusers.d/dojo-greeter.conf`.

Install on a fresh host with `scripts/bootstrap-host all`. On a host that
already runs ly, once the three binaries build:

```sh
scripts/install-system-config.sh
devenv shell -- cargo build --release --bin dojo-dm --bin dojo-dm-pam --bin dojo-greeter
devenv shell -- scripts/install-dojo-dm.sh
scripts/bootstrap-host login
```

`install-dojo-dm.sh` is idempotent. It refuses to install if `dojo-dm-pam
--check` fails under the host loader, backs up a replaced release or `current`
link with a timestamp, keeps `previous` pointing at the last release, and prunes
to the newest four. `bootstrap-host login` enables `dojo-dm.service` (which is
`display-manager.service`) and `getty@tty2.service`, and disables `ly@tty2` and
`getty@tty1`. It starts nothing: the login screen replaces the console at the
next boot, so the running session is untouched. Refresh the inventory
afterwards with `dojo-update --only inventory`.

`pam.d/dojo-login` and `dojo-login-fingerprint` both end in `session include login`.
Neither has been exercised against the real PAM stack or logind yet.

**Rescue path.** `tty2` is a plain `getty`. If the greeter breaks, `dojo-dm`
stops after three starts in 30 seconds, `tty1` stays blank and Ctrl+Alt+F2 is a
working text login. From there: `journalctl -u dojo-dm -b`, then fix and
`sudo systemctl restart dojo-dm`, or roll back (below).

**Keyring.** A password login unlocks the GNOME keyring through
`pam_gnome_keyring` in `dojo-login`. A fingerprint login cannot: no password is
entered, so the keyring stays locked and the first thing that needs a secret
asks for the login password. That is why the fingerprint stack has no keyring
line.

**Fingerprint.** The login screen offers the sensor only for a user whom `dojo-dm`
reports as having a reader and an enrolled print (its answer to `SelectUser`, from
fprintd). Without either, as on Jupiter, the screen is password only: no sensor state in the
dial and no fingerprint worker. The answer is per user, so typing another name arms or
releases the sensor. A fingerprint login leaves the keyring locked (see Keyring above) and
the screen says nothing about it.

**Greeter keys.** Tab or a click on the user chip edits the username, Ctrl+Tab or a click on
the session chip picks the next session, and the power chip (bottom left) or Ctrl+Alt with
S, H, R or P suspends, hibernates, restarts or shuts down after the farewell. A `fake-pam-stack`
build of `dojo-greeter` reads `DOJO_DM_SOCKET` and `DOJO_DM_STATE` for a fake display manager.

**Roll back to ly.** The `ly` package stays installed until you remove it.
Restore its PAM file from history and swap the units:

```sh
sudo rm -f /etc/pam.d/ly   # a link into this checkout that no longer resolves
git show "$(git log -n1 --format=%H -- system-config/pam.d/ly)^:system-config/pam.d/ly" | sudo tee /etc/pam.d/ly >/dev/null
sudo systemctl disable dojo-dm.service getty@tty2.service
sudo systemctl enable ly@tty2.service getty@tty1.service
```

Once the login screen has proved
itself, `sudo pacman -Rns ly` removes the package; until then the next
`capture-system-packages.py` run lists it again.

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

## Power key and farewell

`systemd/logind.conf.d/90-dojo-power-key.conf` makes logind ignore a short press of the power key
and power off on a long one. Niri binds the key to `dojo --leave shutdown`, so the shell plays its
closing with the farewell and then shuts down. Niri hardcodes the power key to suspend ahead of any
bind, so `home-manager.nix` also sets `disable-power-key-handling`. Deploy the Home Manager
configuration before the next boot after installing the drop-in, or a short press does nothing
until then. Where there is no Niri (a TTY, the login screen until it has its own shortcut) a short
press does nothing and the long press is the way off.

The lid stays with logind (`laptop/systemd/logind.conf.d/90-dojo-lid.conf`): the closing cannot be
seen once the lid is shut. The session bridge (`scripts/dojo-lock-session-bridge`) holds sleep and
shutdown delay inhibitors and asks the shell for the farewell on `PrepareForShutdown` and
`PrepareForSleep`, which covers `systemctl poweroff`, `reboot`, `suspend` and `hibernate` and the
lid's sleep. Stasis's suspend step runs `dojo --leave suspend-then-hibernate`.

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
