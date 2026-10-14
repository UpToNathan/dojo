# Keep a scaled and aligned monitor arrangement

Status: resolved
Type: task

Increasing scale, dragging the monitor box to align its edge, and pressing Apply
changes the outputs but never offers Keep and eventually restores the old layout.

## Comments

The scale-only GUI test missed the adjacent-monitor geometry. The regression
`scaled_drag_snaps_to_niris_rounded_placement_geometry` fails on the previous
implementation: it snaps a 2880-wide laptop at 175% scaling to 1645 instead of
1646. Niri's placement code uses `output_size(...).to_i32_round()`, while its
IPC dimensions truncate. A one-pixel overlap causes automatic placement, which
fails Dojo's exact position readback and rolls back.

Snapping and overlap validation now use Niri's rounded placement geometry.
The regression also rejects the formerly accepted one-pixel overlap. Verify
the combined scaled-and-aligned arrangement on the active laptop and HDMI
outputs, including Keep and restoration, before deploying.

## Answer

The corrected geometry passes the formerly failing drag/snap regression and
rejects the one-pixel overlap. Sixteen backend tests and five UI tests pass.
The live combined regression scales the laptop to 175%, snaps the HDMI monitor
to its edge, previews, reverts, previews again, Keeps, verifies reload/readback,
and restores the original runtime layout and saved file bytes. That full
roundtrip passed on the active laptop and HDMI outputs.

Installed release `177cfde7d16002ed-mmap128k` and restarted Dojo on venus.
The service doctor confirms one process and a mapped frame. Evidence under
`/tmp/dojo-display-bugs/` includes `red-scaled-drag.log`,
`scaled-alignment-live.log`, `scaled-alignment-release.log`, and
`scaled-alignment-doctor.log`.
