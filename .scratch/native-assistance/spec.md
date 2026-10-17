# Native application assistance

## Goal

Use the useful CUA components already pinned in Dojo for native application
context, outcome verification, and failure diagnosis. Keep Pi as the run owner
and browser-control as the Helium interface.

## Requirements

- Preserve application-scoped AT-SPI reads for clients without an exact binding.
- Bind a native window to its accessibility root only through metadata attached
  to that exact Wayland surface, with process-generation and bus-owner checks.
- Recheck the native window and its binding after traversal. A disappeared or
  changed target must not return apparently current evidence.
- Use exact bound trees for native element verification. Incomplete or unbound
  observations cannot establish absence in the requested window.
- Keep semantic mutations disabled until their native admission is implemented.
- Export Driver, native, browser-control, perception, and bounded-step calls from
  canonical Pi history. Errors must not imply that no effect occurred.
- Keep exported screenshots omitted by default and preserve redaction limits.
- Reuse the installed perception worker for inaccessible controls; do not add a
  second inference service or unpinned model artifacts.

## Verification

Exercise ownership, target changes, two windows from one process, incomplete
trees, cancellation, and exported call/result evidence. Any live desktop test
uses the existing isolated nested Niri fixture on inactive `dojo-tests`.
