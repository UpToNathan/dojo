# 03 — Evaluate cua's AT-SPI wake and p2p changes for Dojo's accessibility crate

Type: research
Status: needs-triage
Blocked by: none
Effort: medium

## Brief

cua `e7af6a61b` calls `app.get_attributes()` once per pid before the first tree traversal, which
raises a Chromium or Electron tree from 4 nodes to 238. Dojo currently treats those apps as
OCR/vision only (see `docs/native-accessibility.md`). `e3ad8319a` moves the driver to `atspi_connection`
with `p2p` off to stop accessibility-bus deadlocks; Dojo's own crate uses zbus peer-to-peer
connections (`crates/dojo-accessibility`), so check whether it can hit the same deadlock.

Try the once-per-pid wake in `dojo-accessibility` against a real Chromium and a real Electron app
(through the surface annotation path), measure node counts and traversal time, and decide whether
Chromium/Electron move out of the OCR-only list. Check for the deadlock with the same repro cua's
commit describes.

## Acceptance

- A written result (node counts, timings, deadlock check) and a decision; if adopted, a test and the
  `native-accessibility` docs updated.
