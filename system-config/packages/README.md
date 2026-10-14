# Arch package baseline

`official-explicit.txt` lists the desired explicit packages available from the
configured pacman repositories. `foreign-explicit.txt` lists the desired explicit
packages that pacman cannot obtain from those repositories. Foreign packages need
their original package source or an AUR build before installation.

`installed.tsv` records every observed installed package and its version.
`dependencies.txt` records packages installed as dependencies. `foreign.tsv`
records all observed foreign packages and their versions, including dependencies.
These inventory files describe the captured machine. They are not version locks
or additional packages to install explicitly on a fresh system.

Run `python3 scripts/capture-system-packages.py` from the repository to accept
the machine's current package and enabled service baseline. The command queries
pacman and systemd without sudo, then atomically replaces each changed manifest.
Run `python3 scripts/capture-system-packages.py --check` to check for drift without
writes. Exit code 1 means drift. Exit code 2 means the capture failed.

For a fresh Arch installation, install the packages in `official-explicit.txt`
with `pacman -Syu --needed`, preserving a full system upgrade. Let pacman resolve
dependencies. Install any foreign explicit packages from their recorded sources.
Restore repository configuration and Home Manager before enabling the units
listed in [the service baseline](../services/README.md).
