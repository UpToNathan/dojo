# Top bar audio wave

Add a seven-stroke audio wave to the left of the existing right corner controls in Dojo's 24-pixel desktop frame. Its amplitude represents default output volume. Cap the wave at 11 pixels, matching the small corner icons, and render the volume text at 12 pixels. Keep it still at rest. Hover expands the control leftward and reveals the percentage without moving the existing icons. Animate volume and mute changes with slightly staggered strokes.

Dragging right increases volume; dragging left decreases it. A click without a drag toggles mute and preserves the previous volume. Scrolling makes small volume adjustments. Clamp adjustments to 0–100%. Follow external volume, mute and default-output changes. Unavailable audio must appear disabled rather than displaying a made-up volume.

Install and activate the change in the existing desktop session. Bluetooth and output selection are deferred.
