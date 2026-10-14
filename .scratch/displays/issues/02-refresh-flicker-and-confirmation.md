# Fix flickering controls and stalled scaling confirmation

Status: resolved
Type: task

The user reports flickering text and buttons, ignored Apply/Keep clicks, a frozen
confirmation countdown, and a gap above the monitor panel.

## Comments

The previous verification exercised the controller and inspected a screenshot,
but did not click through the actual scaling workflow. Reproduce through the
panel in an isolated compositor and retain the screenshot and persistence
evidence.

The background-refresh regression failed because every poll set `busy`, disabled
buttons, and changed the Apply label. Refresh and timer ticks now leave the
controls available. A second regression failed because the confirmation
countdown appeared during blocking apply readback. Apply now has a separate
five-second deadline, then starts the full confirmation period after readback.
The panel uses the shared top-bar height instead of an extra ten-pixel offset.

Evidence is under `/tmp/dojo-display-bugs/`. The two failing regressions are in
`red-refresh.log` and `red-countdown.log`; final backend checks pass. The isolated
GUI probe clicked Scale, Apply and Keep, verified saved scaling, and captured 40
idle frames with pixel-identical heading and control regions. Recheck the final
release after the countdown change before deployment.

## Answer

Fixed both reproduced state transitions and aligned the panel with the shell
bar. The background refresh test no longer disables controls. The delayed
readback test keeps the confirmation countdown hidden while applying and then
asserts the full confirmation period remains available.

Final checks passed: 15 backend regressions, five UI tests, a live physical-output
scaling controller roundtrip with restoration, and actual Scale/Apply/Keep clicks
inside an isolated compositor. The final GUI probe waited three seconds and
asserted the countdown pixels changed before clicking Keep; saved JSON and live
output state both contained scale 1.25. Forty further frames showed identical
heading and control pixels through background polls. The isolated shell and
compositor were stopped after verification.

Installed `e4e9b31fbccce11a-mmap128k` on venus and restarted the shell. The service
doctor confirms a mapped frame; Niri's configuration validates. Final evidence
is in `final-gui-roundtrip.log`, `final-idle.log`, `final-regressions.log`, and
`deployed-doctor.log` under `/tmp/dojo-display-bugs/`.
