# Chromium ignores the independent agent seat

Type: bug
Status: resolved

## Symptom

The agent cursor can open Dojo's GPUI launcher but cannot toggle T3 Code's sidebar.
Niri and the MCP tool report the click as handled even though the sidebar stays
expanded. This violates the spec's general mapped-application input contract.

## Reproduction and evidence

The host screenshot loop at `/tmp/dojo-t3-click-repro.py` focuses T3 for the agent,
waits for seat binding, moves to the sidebar toggle, clicks and compares a stable
sidebar crop. It fails with `changed_pixels: 0` and `T3 sidebar stayed expanded
after agent click`. Evidence is `/tmp/dojo-t3-repro-before.png` and
`/tmp/dojo-t3-repro-after.png`.

An isolated fixture at `/tmp/dojo-electron-seat-repro.py` uses the exact installed
T3 Electron executable with a separate application and profile in a nested Niri.
Electron 44.4.2 / Chromium 152.0.7977.130 announces two seats to its connections,
but binds only primary seat 42. Agent seat 46 is announced and never bound. A
full-window button receives no click. Evidence is
`/tmp/dojo-electron-seat-evidence/{electron.log,fixture.log,niri.log}`. All fixture
processes were stopped; the user's T3 and compositor were left running.

The same-version upstream implementation returns immediately from
`WaylandSeat::Instantiate` when `connection->seat_` already exists:
[Chromium 152.0.7977.130 wayland_seat.cc](https://github.com/chromium/chromium/blob/152.0.7977.130/ui/ozone/platform/wayland/host/wayland_seat.cc#L34).

## Cause

Chromium keeps one Wayland seat per connection and ignores additional globals.
No pointer or keyboard resource exists for the agent seat on that client, so
Smithay cannot send the agent's input to it. GPUI's fork binds every seat, which
explains why the same compositor path works in Dojo and the GPUI fixtures.

At reproduction, the compositor checked its internal pointer focus before a press but
does not check whether the target client bound that pointer. Consequently
`Handled` acknowledges internal dispatch even when no client can receive it.

## Required follow-up

- Reject presses, typing and scrolling clearly when the target client has no
  corresponding agent input resource. Test against a client that ignores
  additional seats, alongside the clients that bind them.
- Design compatibility for applications that accept only one seat, or change
  their input implementation. Preserve the human pointer position and keyboard
  focus. Routing through the human seat without those guarantees would violate
  the existing contract.
- Verify actual application state changes. An IPC acknowledgement is not a
  delivery assertion.

## Resolution

The [shared pointer coordination layer](../../shared-pointer/spec.md) delivers
agent events through the primary resources already bound by single-seat clients.
It preserves the human compositor pointer position and keyboard layout focus.
Human interaction cancels conflicting held agent input, restores human protocol
focus and modifiers, and leaves the separate agent cursor visible. Missing input
resources produce an error rather than a handled response.

The maintained fixture is `tools/agent-input/run-electron.py`. It runs an isolated
copy of T3's installed Electron binary, confirms that only the primary seat was
bound, and checks page events for clicks, typing, modifiers, scrolling and held
input release. It also checks ownership events, stale-action rejection, worker
retention, recovery and cursor visibility. The user's T3 is not restarted.

The final release fixture passed on October 10, 2026. Evidence is
`/tmp/dojo-electron-coordination.gt9a9b5m`, including protocol and page event logs,
ownership notifications and screenshots. The takeover screenshot retains 632
cyan cursor pixels while the human cursor operates the same app. The final
native and GPUI checks also passed; see
[the coordination issue](../../shared-pointer/issues/01-coordination.md).
