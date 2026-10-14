# Replace Nirimon with native monitor settings

Status: resolved
Type: task

Implement the [monitor settings spec](../spec.md). A graphical arrangement view
is required; an external replacement with a different shell style was rejected.

## Comments

Nirimon reports failure executing the Niri CLI while applying output positions.
The replacement uses the vendored typed socket protocol. wdisplays was tested as
an alternative and dropped after the user rejected its interface.

## Answer

Added a native shell panel with draggable monitor arrangement, edge snapping,
advertised resolution and refresh menus, scale controls, enabled state, and
keyboard positioning. The frame monitor button and `dojo-desktop --displays`
open it. Apply previews for 15 seconds; Keep persists and reloads Niri, while
Revert, close, timeout, or failed application restores the previous layout.
Home Manager owns the optional saved-output include. Nirimon's package and
input pin are removed, and the Home Manager configuration is deployed on venus.

Validation passed: 63 library and 105 shell tests in the initial full run,
13 final display backend tests, five final UI tests, and the control command
delivery test. The ignored live Niri regression verified preview, revert,
Keep with reload/readback, and byte-for-byte restoration of prior saved files.
The live session had HDMI active and the laptop output disabled during the
probe. Fractional-scale geometry and disconnected-output rollback have separate
regressions.

Installed release `54592e5d593f7e94-mmap128k` and restarted the Dojo service.
The service doctor confirms one shell process with a mapped frame. Niri's
configuration validates. Visually inspected the native panel screenshot;
automated pointer interactions remain unverified because the desktop capture
tool could not load its Wayland library. Captured with grim instead.
Evidence is in `/tmp/opencode/dojo-verification/displays/`.

User instructions are in [monitor settings](../../../docs/displays.md).
