# Human checklist for the real login

Status: ready-for-human
Blocked by: 06, 07, 08

`scripts/check-login-screen-by-hand.sh` walks through real login, wrong password, password pause, logout, restart, hibernate, shutdown and the `tty2` rescue, with the way back at each step.

## Power triggers (from 08)

Each one should show the closing and farewell once, then power off, restart or sleep. Check that the screen never shows the closing twice.

- Short press of the power key (needs the logind drop-in installed and a reboot, and the Home Manager switch).
- `systemctl poweroff`, `systemctl reboot`, `systemctl suspend` and `systemctl hibernate` from a terminal.
- System menu: shut down, restart, suspend, hibernate.
- Lid close on battery (suspend-then-hibernate) and on power (suspend); resume shows the lock screen.
- Stasis's idle suspend step.
- Kill the shell (`pkill dojo`) and run `systemctl poweroff`: it must still power off within about 5 seconds.
- Hold the power key for the long press with the shell hung: it must power off.

## Progress

The script runs read-only preflight checks, states the way back through `tty2` first, then records yes, no or skip for each step in `~/.local/state/dojo/login-checklist.log`. It performs no power action itself.

## Comments
