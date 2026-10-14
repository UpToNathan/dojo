# Restore Derive with the approved refined recipe

Status: resolved

Restore all 23 derived modes in the picker and theme CLI. Preview, generation, and
Apply must use the same refined recipe and revision. Keep static preset colors and
older stored generations unchanged. Publish the refined semantic status pairs to
supported external formats. Remove the hard-coded Astro picker fallback.

The picker-to-generation regression checks every mode in both appearances and
verifies exported T3 warning/error pairs and its existing 61-role contract.

## Comments

Restored the picker and CLI with refined revision 2. The library suite passed
51 tests with one unrelated ignored integration test, all 16 picker tests passed,
and the wallpaper identity test passed. The release is deployed and Derive is
visible in the running picker. Preview and Apply match for every mode and both
appearances; T3 retains its 61-role format and receives the correct status fills.
