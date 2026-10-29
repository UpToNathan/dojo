- **Wayland overlays take clicks while idle.** A layer-shell overlay reads input from its own
  queue on winit's connection, and winit turns its loop only for its own events, so a click
  on an overlay that wasn't animating waited for an unrelated wake (a timer, the pointer
  leaving) and often landed late or never. A watcher thread now wakes the loop when the
  compositor writes to the display, at most once per read.
