# Preserve semantic foreground and background pairs in the shell

Status: ready-for-agent

## Problem

`dojo.json` and `ThemeColors` discard `primary_foreground`, `selection_foreground`, `surface`, and `surface_raised`. Picker controls guess black or white labels locally.

## Acceptance

- Carry the existing authored pairs and surfaces through export and runtime loading.
- Accept existing six-role fallback config and existing generations with an explicit fallback policy.
- Replace picker label guesses with the correct paired foreground. Audit launcher and tray selection usage.
- Preserve exact authored colors; avoid assigning new meanings to existing stored fields.
- Check export-to-runtime behavior with a palette whose paired foreground is neither black nor white.
- Run build/tests from devenv and the project verify-dojo skill for native input/frame behavior after UI changes.
