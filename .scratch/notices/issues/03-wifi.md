# Notify on Wi-Fi changes

Status: resolved
Type: task
Blocked by: 01

Diff `active_connection` in `src/wifi_ui.rs` and send the Wi-Fi notices in the spec. Pure diff function with tests: first snapshot silent, network to network is one notice, Ethernet by label.

## Result

`connection_notice(old, new)` in `src/wifi.rs` is the pure diff over `active_connection`: connected gives Low "Connected to LABEL", disconnected gives Low "Disconnected from LABEL", a move between two connections is one "Connected to NEW", and no change gives nothing. Every notice has kind `wifi`, so the latest replaces the earlier. Ethernet and other kinds use their label like a network. `ConnectionWatch` keeps the last connection; its first settled snapshot (NetworkManager actually read, no observation error, not the "Checking Wi-Fi…" placeholder) is the baseline and sends nothing. `src/wifi_ui.rs` holds a `ConnectionWatch` in `WifiUi`, observes the initial snapshot, and posts through `dojo::notify::post` where `ui.snapshot` is assigned.

Tests: `cargo test --bin dojo -- wifi bluetooth` (all existing and new pass), `cargo check --bins`.
