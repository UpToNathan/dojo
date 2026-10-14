# Arrange your monitors

Click the monitor icon in Dojo's top bar, or run `dojo-desktop --displays`.
The editor uses the shell's current theme.

1. Drag the monitor boxes into the arrangement you want. Nearby edges snap together.
2. Select a monitor to change its resolution, refresh rate, scale, or enabled state.
3. Click Apply to preview the layout for 15 seconds.
4. Click Keep to save it. Click Revert to restore the previous layout.

Closing the editor or letting the preview expire also restores the previous
layout. Dojo rejects overlapping monitors and arrangements with every monitor
disabled. Resolution and refresh menus contain the modes advertised by the
selected monitor.

Dojo saves confirmed settings in `~/.local/state/dojo/displays/niri.kdl` and
`niri.json`. Home Manager maintains the optional Niri include. Saved settings for
disconnected monitors remain available when they reconnect. Keep reloads Niri's
configuration and verifies the resulting layout before accepting the save.

The preview timer runs independently of the editor window, inside the shell
process. If the entire shell is killed during a preview, the timer stops; the
unconfirmed layout remains temporary and is never saved for the next login.
