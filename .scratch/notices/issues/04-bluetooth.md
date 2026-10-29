# Notify on Bluetooth devices

Status: ready-for-agent
Type: task
Blocked by: 01

Diff devices by id in `src/bluetooth_ui.rs`: connected, disconnected, and battery at or below 15% with re-arming above 20%. Pure diff with tests; first snapshot silent.
