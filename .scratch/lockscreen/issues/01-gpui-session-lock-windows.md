# Add session-lock windows to the vendored GPUI

Status: resolved

Land the spike patch in `vendor/zed` and harden it, so a GPUI app can take `ext-session-lock-v1` and open one window per output. Start from [the spike patch](../spikes/gpui-session-lock.patch) and read [the grounding notes](../grounding.md) first. Follow [ADR 0001](../../../docs/adr/0001-lock-screen-is-a-separate-gpui-program.md).

The patch adds `WindowKind::SessionLock`, a matching Wayland surface state, a lazily created shared lock object, `cx.on_session_lock_event` and `cx.unlock_session()`. It skips the initial commit for lock windows because committing before the first `ack_configure` makes Niri disconnect the client.

## Acceptance

- The spike example locks, renders, takes keyboard text input and unlocks in the nested Niri on one and on two outputs. Never run it against the live session.
- A lost Wayland connection ends the process. It must not spin the CPU.
- A lock object is not left orphaned when the first lock window fails to open.
- Output hotplug while locked opens or closes a lock window for that output.
- Existing GPUI layer-shell and window tests still pass. Build and test through `devenv shell --`, with no private target directory and nothing left in `/tmp`.
- Record how much of the patch is Zed-upstream-shaped, so carrying it stays cheap.

## Progress

Built and committed (vendor/zed): session-lock windows, shared lock object, a dead-connection fix that stopped a CPU spin in every GPUI Wayland app, no orphaned lock objects, duplicate-surface refusal, hotplug, and an error on non-Wayland. Proven in a nested Niri and a headless two-output harness. Not tested: a real input method.

## Comments
