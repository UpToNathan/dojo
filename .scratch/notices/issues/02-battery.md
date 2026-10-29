# Notify on battery and power changes

Status: resolved
Type: task
Blocked by: 01

Diff the host-state `PowerStatus` in `src/frame.rs` (where `cx.set_global(next)` is called) and send the battery notices in the spec. Edge-detect in a pure function with tests: first reading is silent, each threshold fires once per fall and re-arms, plug, unplug and full.

## Result

`BatteryWatch` in `src/power.rs` (pure, no GPUI): `observe(&PowerStatus) -> Vec<Notice>`, fed each new reading. The first reading with a battery only sets the baseline (a battery already below a threshold counts as having fired it); readings with no battery send nothing and keep the baseline. Discharging at or below 20% sends Normal "Battery low"; at or below 10%, Critical "Battery critically low" with a body naming the charge left (jumping past both sends only the critical one). Each fires once per fall and re-arms above 25% or while charging. Discharging to Charging sends Low "Charging"; Charging or Full to Discharging sends Low "On battery"; Charging to Full sends Low "Fully charged". All are kind `battery`. Unknown status strings and a missing capacity send nothing.

Wired in `src/frame.rs`'s host-state loop: the loop seeds the watch with the startup reading, then posts what `observe` returns for each fresh power read through `dojo::notify::post` (non-blocking), before the `HostState` update.

Validation: `devenv shell -- cargo test --lib power` (23 passed, 13 of them new `BatteryWatch` tests: first reading, fall through 20 then 10 once each, jump past both, re-arm above 25% and after charging, no re-arm between 20 and 25, plug, unplug, full, no battery, odd statuses) and `devenv shell -- cargo check --bins` (clean; the warnings shown were already there).
