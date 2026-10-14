# Preserve semantic foreground and background pairs in the shell

Status: resolved

## Problem

`dojo.json` and `ThemeColors` discard `primary_foreground`, `selection_foreground`, `surface`, and `surface_raised`. Picker controls guess black or white labels locally.

## Acceptance

- Carry the existing authored pairs and surfaces through export and runtime loading.
- Accept existing six-role fallback config and existing generations with an explicit fallback policy.
- Replace picker label guesses with the correct paired foreground. Audit launcher and tray selection usage.
- Preserve exact authored colors; avoid assigning new meanings to existing stored fields.
- Check export-to-runtime behavior with a palette whose paired foreground is neither black nor white.
- Run build/tests from devenv and the project verify-dojo skill for native input/frame behavior after UI changes.

## Resolution

Nord's pale selected controls came from dropping paired foreground roles at
the Dojo export/runtime boundary. Export and ThemeColors now preserve both
foreground pairs and both surface roles. Picker controls, launcher results
and selected search text, and hovered tray items consume selection_foreground.
Accent-filled picker controls consume primary_foreground. Local picker guesses
were removed. Existing six-role generations/config retain an explicit contrast
fallback; authored foregrounds are preserved exactly when present.

Checks: 94 shell tests and 27 theme library tests passed in devenv. The new
41-preset export/runtime sweep asserts exact authored pairs/surfaces and 4.5:1
for adapted pairs; existing authored palettes retain their own contrast. A
legacy Nord regression asserts dark text on its white highlight. Release
7990899c78dbb576-mmap128k was installed and the desktop service restarted.

Live evidence: `/tmp/opencode/dojo-verification/nord-pairs-20261012/HDMI-A-1.png`
and `layers.txt`. Temporarily supplied Nord's six original shell roles, opened
the real picker, inspected readable selected controls, closed it, and restored
the original runtime file. One desktop process remains active with frame and
reservation layers on both outputs. Pointer pass-through was not exercised;
this change does not alter input regions or frame geometry.
