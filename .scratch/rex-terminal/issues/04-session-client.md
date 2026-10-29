# Whole-session client in dojo-rex

Status: ready-for-agent

`dojo_rex::Session`: find-or-create a session by label, follow `session.view` through `session_view_changed`, stream every visible block of the active window, and expose tab, split, focus, zoom and close calls. `list_sessions` with directory, program and status for the picker. A most-recently-used list in `$XDG_STATE_HOME/dojo/terminal/recent`. Layout geometry is pure and tested.
