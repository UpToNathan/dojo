# Independent Niri agent seat

Type: task
Status: resolved

Implement the compositor and IPC requirements in [the spec](../spec.md). Keep controller lifetime tied to its socket. Preserve human layout activation and input. Cover lock and disconnect cleanup.

## Answer

Implemented controller-owned Smithay seats, independent keyboard and pointer routing, rendered software cursors, and lock/reset/disconnect cleanup. The experimental gate preserves upstream protocol exposure in normal mode. Niri's full library suite passes: 209 tests. Live raw Wayland and GPUI runs confirm independent input and unchanged human layout focus.
