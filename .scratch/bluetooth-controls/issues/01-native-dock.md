# Build and activate the native magnetic dock

Status: needs-info

Implement [the spec](../spec.md). Backend/UI candidates and judge are in `/tmp/dojo-bluetooth-design-{luna,sol,verdict}.md`. Choose the dedicated keyboard-capable panel, single BlueZ actor and authoritative snapshots. User selected device tiles with explicit controls and subtle settling motion after rejecting spatial drag-to-connect prototypes.

## Progress

The native dock, BlueZ controller, private D-Bus tests, and setup script are implemented and installed. Backend tests verify service appearance, power changes, discovery ownership, connect/disconnect and failures, confirmation/PIN/passkey replies, rejection, cancellation on dock close, daemon restart, stale device IDs and worker shutdown. No actual hardware pairing is claimed because BlueZ is absent on the host.

The first live icon click exposed a GPUI reentrancy crash: `toggle` opened the window inside `update_global`, which temporarily removed `BluetoothUi` while the new renderer tried to read it. The new native window test reproduced the exact panic before the fix. Window opening/closing now occurs after the short global update has returned; the same test passes. Release `465cbcf082327db7-mmap128k` is installed with the audio hover regression fix. Dojo is running with frames on both outputs. Existing column widths were recovered from the pre-update snapshot after the crash.

Remaining host step: `bash scripts/setup-bluetooth.sh`, which requires the user's sudo password. The dock observes BlueZ starting and will update without a shell restart. Native icon automation paused on human input ownership, so repeated live interaction and real hardware connection checks remain pending. Evidence is under `/tmp/opencode/dojo-verification/audio-hover/`.
