# Give the volume, brightness and screenshot keys feedback

Status: resolved
Type: task
Blocked by: 01

Add `XF86AudioRaiseVolume/LowerVolume/Mute` binds and route the existing brightness binds and the screenshot bind through `dojo-notify` in `home-manager.nix`. `dojo-notify volume|brightness` sets the level and sends the progress notice of that kind; the screenshot bind notifies only when `utsushot` succeeds. Changes made elsewhere send nothing.

## Result

`home-manager.nix`: added `XF86AudioRaiseVolume`, `XF86AudioLowerVolume` and `XF86AudioMute` binds spawning `~/.local/bin/dojo-notify volume up|down|mute`; the screen brightness binds now spawn `dojo-notify brightness up|down` and the keyboard backlight binds `dojo-notify brightness up|down --device tpacpi::kbd_backlight`. The screenshot bind runs `sh -c "utsushot --copy && dojo-notify send --kind screenshot --urgency low --icon camera-photo-symbolic \"Screenshot copied\""`, so a cancelled or failed `utsushot` sends nothing. Every new or changed bind has a `hotkey-overlay-title`. No bind in this file sets `allow-when-locked`, so none was added.

Verified without activating: `devenv machines deploy "$(cat /etc/hostname)" </dev/null` built the Home Manager generation and stopped at "Deployment needs confirmation". The generated `home-files/.config/niri/config.kdl` holds all seven binds and the screenshot bind as valid KDL, with the inner quotes escaped.
