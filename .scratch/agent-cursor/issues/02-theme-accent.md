# Use the theme primary accent for agent cursors

Type: task
Status: resolved

## Implementation

Export the semantic primary accent as Niri cursor.agent-color. Decode the
embedded glyph's alpha coverage and color it at runtime, preserving its size
and hotspot. Recolor live agent buffers when Niri reloads the theme.
The default accent matches Dojo's fallback primary, #cba6f7.

Build Niri and the theme exporter through devenv. Install the compiled release
for the next login. Keep the running desktop alive.

## Result

Niri release, Dojo and dojo-theme builds succeeded through devenv. The updated
Niri configuration validates, including the generated agent-color #7aa2f7.
The theme publisher retained the selected generation, appearance and target
list, checked ownership of the existing generated file, and saved a backup.
Installed compositor: /home/nathan/.local/share/dojo/niri/7936a75870756c69.

The existing cursor color assertion and Niri export expectations were updated.
The nested Electron fixture now checks its explicit #7aa2f7 cursor color.
No test suite or live rendering check was run for this change. The active
session remains on the prior compositor; logout/login activates this build.
