# Notify on Wi-Fi changes

Status: ready-for-agent
Type: task
Blocked by: 01

Diff `active_connection` in `src/wifi_ui.rs` and send the Wi-Fi notices in the spec. Pure diff function with tests: first snapshot silent, network to network is one notice, Ethernet by label.
