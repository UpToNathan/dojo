# GPUI concurrent Wayland seats

Type: task
Status: resolved

Implement GPUI's seat registry, device routing and focus requirements in [the spec](../spec.md). Use the pinned subtree. Preserve existing single-seat behavior and remove only the departing seat.

## Answer

Implemented per-seat devices, keyboard composition/repeat/modifiers, pointer/scroll routing, focus membership and removal cleanup in the pinned GPUI subtree. Live nested GPUI scenarios pass with both dynamically created and preexisting agent seats. Clipboard, primary selection and text-input-v3 remain primary-seat services; same-window widget state remains shared.
