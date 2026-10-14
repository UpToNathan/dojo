# Native monitor settings

Replace Nirimon with a graphical monitor editor inside the Dojo shell. The user
rejected wdisplays because its appearance and interaction feel separate from Dojo.
Monitor arrangement is the main control. Also provide resolution, refresh rate,
scale, enabled state, and saved settings.

Use the vendored Niri IPC directly. Stage edits in the graphical view. Apply
enters a 15-second reversible preview; Keep saves, while timeout, Revert, close,
or failed application restores the original layout. Verify live compositor state
after applying and after reloading saved settings. Do not turn off every monitor.

The maintained Home Manager configuration includes a separate mutable fragment
at `~/.local/state/dojo/displays/niri.kdl`. Dojo owns that fragment and retains
saved configurations for disconnected monitors. Home Manager owns the include.

## Shape and synthesis

Compared a full resizable settings window with a shell-attached popover. Select
the popover to match Dojo's frame, Bluetooth, theme colours, and rounded surfaces.
Keep the graphical map prominent and explicit Apply/Keep/Revert controls from the
settings-window design. The IPC controller owns preview rollback independently
of the view; the view owns only uncommitted edits.

`dojo::displays` owns typed settings, geometry, IPC, read-back, preview, and saved
files. `displays_ui` owns rendering and input. A frame monitor button and
`dojo-desktop --displays` open it. Remove the Nirimon package and input pin.

## Verification

Test logical geometry, drag mapping and snapping, overlap/all-off rejection,
mode/rate selection, safe serialization, missing outputs, partial apply failures,
read-back, saved settings, and timeout rollback. Validate with the live laptop
and HDMI monitors, preserving their original configuration after probes.

A controller thread keeps rollback independent of the view, but cannot survive
termination of the entire shell process. Saving remains explicit so unfinished
previews cannot become login configuration.
