# Agent input

## Notes

Implement independent agent input in the pinned Niri and GPUI forks. See [the spec](spec.md).

## Decisions-so-far

- [Niri seats](issues/01-niri-seat.md): persistent controller sockets own real seats and software cursors; human layout activation stays independent.
- [GPUI seats](issues/02-gpui-seats.md): keep primary-seat services and route additional devices, focus and input per seat.
- [Integration](issues/03-integration.md): local dependencies, Dojo controller and nested fixtures are implemented; 209 Niri tests, 29 Dojo tests and live scenarios pass.

## Fog

XWayland and additional-seat clipboard, text-input-v3, gestures, popup grabs and window management remain outside this experimental implementation.
