# Enabled service baseline

`system-enabled.txt` records enabled system unit files. `user-enabled.txt` records
enabled unit files for the current user. Both lists include services, sockets,
and targets. They describe enablement, not whether a unit is currently running.

`scripts/capture-system-packages.py` refreshes both lists together with the Arch
package inventory. Its `--check` option reports drift without changing files.

Restore the Arch packages, run `scripts/install-system-config.sh`, and deploy the
Home Manager configuration before enabling these units. Use system-level
`systemctl enable` for `system-enabled.txt` and `systemctl --user enable` for
`user-enabled.txt`. Template names use their packaged `DefaultInstance`, such as
`getty@.service` with `tty1`, or need an explicitly chosen instance.
Check missing units against the package and configuration installation steps.
