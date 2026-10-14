# Dojo Wayland capture

Native standard `ext-image-copy-capture-v1` output capture derived from CUA at
`88aff7e5424e9ed45b93ded85eb4111472ca5190`. See provenance.json and LICENSE.md.

`capture_output(&CaptureRequest)` takes explicit absolute Wayland and Niri IPC
socket paths, an exact output name, deadline, cancellation flag, and limits.
Both sockets must have the same Linux SO_PEERCRED PID, UID and GID. The Niri
connection remains alive until capture completes. This checks process identity;
it does not authenticate an executable or prove the caller's supplied IPC socket
is Niri. The service must supply its configured trusted Niri socket.

The returned PNG is in displayed orientation. Width and height describe that PNG.
The original protocol transform, monotonic presentation time, and compositor PID
are returned as evidence. The caller owns logical crop/resample mapping and
before/after identity/geometry checks. This crate neither captures an isolated
window nor proves geometry and image came from the same compositor scene.
The primary hardware cursor is not requested. Agent cursor overlays may be part
of the compositor scene. Capture uses the compositor's ScreenCapture policy.

Output naming requires wl_output version 4. Unsupported protocol, missing or
ambiguous output, removal, stopped session, malformed constraints, format error,
disconnect, timeout and cancellation return errors. There is no automatic fallback.
Socket connection, discovery and dispatch use nonblocking sockets and poll slices
of at most 20 ms. Pixel conversion checks cancellation/deadline every row. PNG output checks them
on every encoder write and checks its byte budget before extending the output.
Encoder work between writes is bounded by image limits. Default allocation limits
are 32768 pixels per axis, 32 Mi pixels, and 128 MiB per buffer/encoded image.
The oriented decoded image adds one bounded allocation beyond the SHM buffer.

## Changes from upstream

- Explicit output selection and Wayland/Niri socket peer match.
- Nonblocking bounded event dispatch instead of blocking roundtrip deadlines.
- Cancellation and output removal/session-stop handling.
- Require complete protocol constraints and frame metadata.
- Select only Xrgb8888 or Argb8888. Preserve padded row handling, force opaque
  XRGB, and unpremultiply ARGB before PNG encoding.
- Normalize all eight buffer transforms. Rotation direction follows Niri's
  inverse output render transform and Smithay's buffer-to-logical coordinates.
- Check configured budgets and signed protocol integer bounds before allocation.
- RAII tempfile/mmap ownership and connection teardown on every error path.
- No cleanup roundtrip, persistent worker, image store, or input operation.

Run `devenv shell -- cargo test --manifest-path crates/dojo-capture-wayland/Cargo.toml`.
Tests use Rust socket fixtures and literal padded/alpha/non-square transform images.
Live multi-output/scale/rotation parity with grim remains an integration gate.
