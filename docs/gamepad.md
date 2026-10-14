# DualSense controls

Connect a PS5 DualSense or DualSense Edge over USB while Dojo is running. The
controller service reconnects after unplugging and ignores button states
reported at connection.

| Control | Action |
| --- | --- |
| Controller touchpad | System mouse input |
| Left stick | Focus windows/columns above, below, left, or right |
| Right stick up/down | Scroll through launcher items |
| D-pad left | Launch `$BROWSER` |
| D-pad up | Launch `$EDITOR` inside `$TERMINAL` |
| D-pad right | Launch `$TERMINAL` |
| D-pad down | Open or focus T3 Code |
| Square | Open Dojo's app launcher |
| Circle | Back / Escape (dismiss the launcher) |
| Cross | Select / confirm the launcher selection |
| Triangle | Cycle through open windows; if there are no open windows, cycle through monitors when more than one is connected |

The right stick's horizontal axis and all other controller buttons are unassigned. Application commands are read from
the controller service environment, including quoted arguments. The terminal
must support `-e` to run the editor.

Home Manager imports `gamepad-home-manager.nix` to install the user service.
To activate it without deploying the rest of Home Manager, run
`bash scripts/install-gamepad.sh`. The installer backs up an existing unit.
Use `systemctl --user stop dojo-gamepad.service` to stop it, or
`systemctl --user disable --now dojo-gamepad.service` to disable login startup.
Inspect logs with `journalctl --user -u dojo-gamepad.service`.

The adapter reads Linux joystick events and asks the kernel for their axis and
button mappings. It discovers USB Sony vendor/product IDs in sysfs instead of
assuming a fixed device number. No extra packages, root service, uinput access,
or compositor changes are required. Run `python3 scripts/test-dojo-gamepad.py`
to check the control mappings and focus behavior.
