# Notify when a theme apply finishes

Status: resolved
Type: task
Blocked by: 01

Send the theme notices from the end of an apply the person started (picker, `dojo-theme apply|preset|select`), not `refresh_system`. Pure summary function over `ApplyReport` with tests.

## Result

`src/theme/notice.rs` holds the pure `theme::notice(&ApplyReport) -> Notice`: no problems gives Low, kind `theme`, "Theme applied"; any target `blocked`, `reload_failed` or `needs_setup` gives Normal "Theme applied with problems" with those targets named in order (each once, the first three, then "and N more"). `published`, `ready`, `reload_requested` and `needs_selection` are fine. The picker posts it (`dojo::notify::post`) after a preset apply and after a wallpaper-derived theme apply, before it updates its own message. `dojo-theme` sends it (`dojo::notify::send`) from `print_report`, which `apply`, `select`, `preset` and `procedural apply` share; stdout keeps only the JSON and a failed send is one line on stderr. `refresh_system`, `rollback` and the picker's wallpaper-only selection send nothing.

Tests: six unit tests in `theme::notice` (all fine, `needs_selection` fine, one blocked, several named, truncation, repeated target). `cargo test --lib theme` passes (78 passed) and `cargo check --bins` is clean of new warnings.
