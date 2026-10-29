# Notify on Bluetooth devices

Status: resolved
Type: task
Blocked by: 01

Diff devices by id in `src/bluetooth_ui.rs`: connected, disconnected, and battery at or below 15% with re-arming above 20%. Pure diff with tests; first snapshot silent.

## Result

`DeviceWatch` in `src/bluetooth.rs` diffs devices by id between snapshots. A device connecting or disconnecting sends Low "NAME connected" / "NAME disconnected", kind `bluetooth-<address>` so two devices do not replace each other. A connected device's battery falling to 15% or below sends Normal "NAME battery low" (kind `bluetooth-battery-<address>`) once per fall; it re-arms above 20% or when the device disconnects. Silent: the first snapshot with a ready adapter (the baseline), a device first seen already connected or already low, a device that vanishes, and snapshots while the Bluetooth service is missing. `src/bluetooth_ui.rs` holds the watch in `BluetoothUi`, observes the initial snapshot, and posts through `dojo::notify::post` where `ui.snapshot` is assigned.

Tests: `cargo test --bin dojo -- wifi bluetooth` (all existing and new pass), `cargo check --bins`.
