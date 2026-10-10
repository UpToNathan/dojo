# Keep the frame behind fullscreen applications

Type: task
Status: resolved

Implement [the spec](../spec.md). Reproduce with `tools/agent-input/run-nested.sh --fullscreen`, verify fullscreen edge pixels match the frameless baseline and that leaving fullscreen restores the frame.

## Answer

The frame's Overlay layer placed it above fullscreen clients. Changed it to Top and retained its negative exclusive zone and separate edge reservations. The live regression failed before the change with nonzero differences at all eight fullscreen edge crops (`/tmp/dojo-agent-nested.FItMYY`). It passes after the change with eight zero differences and 9604 restored-frame pixels after leaving fullscreen (`/tmp/dojo-agent-nested.PgUpo3`).

Reloaded the actual desktop shell and visually checked fullscreen T3 Code covers the output (`/tmp/dojo-host-fullscreen-fixed.png`). Both existing window widths stayed at 1538 pixels across the reload. All 42 shell tests pass, with one preexisting ignored test. Updated verify-dojo's layer expectations and added the isolated pixel regression.
