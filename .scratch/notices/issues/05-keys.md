# Give the volume, brightness and screenshot keys feedback

Status: ready-for-agent
Type: task
Blocked by: 01

Add `XF86AudioRaiseVolume/LowerVolume/Mute` binds and route the existing brightness binds and the screenshot bind through `dojo-notify` in `home-manager.nix`. `dojo-notify volume|brightness` sets the level and sends the progress notice of that kind; the screenshot bind notifies only when `utsushot` succeeds. Changes made elsewhere send nothing.
