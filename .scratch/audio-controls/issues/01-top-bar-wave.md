# Add and activate the audio wave

Status: resolved

Implement the accepted experience in [the spec](../spec.md). Read system audio on a worker, subscribe to change events, and keep subprocess work off GPUI's render thread. Verify parsing, drag limits, mute animation, then drive the installed control in the live Niri session and compare its result with system volume.

## Result

Implemented in `src/audio.rs` and `src/frame.rs`, installed with the release installer, and activated in the existing Dojo systemd service. The wave expands leftward, animates staggered crests, flattens into a continuous muted line, and follows default-output changes through an event subscription. Commands run on a worker and coalesce rapid drag/scroll input.

Validation: `devenv shell -- cargo test --bin dojo` passed 55 tests with one existing ignored test. The release build and autostart check passed. `scripts/check-audio-wave.py` verified real mute/unmute, drag, scroll, release outside the control within the top bar, and external volume updates; it restored the original 80% unmuted state. Evidence and screenshots are in `/tmp/opencode/dojo-verification/audio-wave/`. The external-change screenshot displays 25 and the muted screenshot displays a flat line. All existing column widths match the snapshot before activation.

The fixed-subtraction layout helper expects fullscreen columns to shrink and fails on the existing fullscreen browser column. Before/after snapshots confirm it remains unchanged, as required by Dojo's existing fullscreen behavior. Agent seats do not support client pointer grabs, so the live release check stays within the top bar rather than crossing into the workspace.

## Comments

The first drag check exposed a full-desktop input-region expansion that let the human pointer take ownership of Dojo during independent-seat dragging. Removed the expansion; the original live interaction script then passed. Bluetooth and output selection remain deferred.

### Hover text regression

The reported 100% label wrapped while its width animated from 0 to 28 pixels, moving the number upward and drawing a zero below the frame. The native GPUI renderer reproduced a 58-pixel text height in a 22-pixel control at 2% reveal. `frame::tests::audio_percentage_stays_on_one_line_while_revealing` now checks fixed text dimensions and baseline throughout expansion and contraction. The number keeps a fixed 28-pixel layout with no wrapping; only its outer reveal width changes, and its opacity stays zero until the full number has room.
