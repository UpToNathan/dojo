# 01 — Port Niri's Smithay virtual keyboard and pointer onto Dojo's agent seat

Type: task
Status: needs-triage
Blocked by: none
Effort: large

## Brief

Upstream Niri `7da6ae44` deletes `src/protocols/virtual_pointer.rs` and uses Smithay's virtual
keyboard and pointer; `fc9e8c42` is the Smithay bump it needs (it also brings fcitx5/wtype fixes and
tablet proximity-out pressure 0). Smithay also changed `input_intercept()` to take a source
(`254f8873`) and added grab-bypassing virtual keyboard events (`9786228e`). Dojo's agent seat filters
virtual input per seat in `vendor/niri/src/handlers/mod.rs` (`seat.owns()`, `led_state_changed`) and
`src/input/mod.rs` (`human_keyboard_activity()`); `docs/agent-input.md` describes the contract.

Port the filtering onto the new handlers, keeping the claims in `docs/agent-input.md` true. Then take
`12de51d6`'s original LED approach (Dojo currently has a hand port that runs after the num-lock
restore) and drop the hand port if it becomes redundant. Held-back conflicts found in review:
`handlers/mod.rs`, `input/mod.rs`, `protocols/mod.rs` (adjacent `pub mod` lines).

## Acceptance

- Agent seat input still cannot reach the human seat and the reverse; the existing agent-input
  tests pass and gain coverage for virtual keyboard events.
- `cargo test -p niri -p niri-ipc -p niri-config` passes, including layout tests.
- Live check with `dojo-tests` (never the user's workspace) after a relogin.
- `provenance/README.md` lists the commits applied and no longer lists them as held back.
