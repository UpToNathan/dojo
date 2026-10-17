# Remove Zed and Delta from the desktop

Status: resolved
Type: task

Remove the unused Zed editor and Zed Industries Delta app from the installed
desktop and maintained application integrations.

## Comments

Removed both Home Manager packages and Delta's input/lock references. Activated
the dev machine; neither executable nor desktop entry remains in the active profile.
Removed Zed exports, settings publication, tests for the deleted integration,
and Browser Control provisioning. Removed Delta's unsupported theme target.
Stored selections discard both retired targets on deserialization so old
snapshots do not reintroduce them during theme rollback.

The vendor/zed source remains because Dojo depends on its GPUI crates. User
application data and old Home Manager generations were not purged.

Validated the activated profile and launcher locations: no Zed/Delta executable,
desktop entry, or running process remains. Rebuilt and installed the desktop and
theme command, then reapplied the existing Kanagawa Dragon palette. Its selection
now contains 13 targets and no retired export files. Theme tests and Browser
Control config tests pass. The updated 41-preset sweep emits 17 formats per
preset and passes all 2,337 checked color pairs. The stored-selection migration
regression and desktop autostart check pass.
