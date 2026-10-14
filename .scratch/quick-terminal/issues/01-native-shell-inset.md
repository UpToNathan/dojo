# Nest quick-access Kitty in the desktop frame

Status: resolved

Implement [the spec](../spec.md), keeping the existing persistent quick-access session behavior and shortcut. Verify the actual terminal placement, keyboard ownership, hide/reopen behavior, and desktop column widths in the live session.

Corrected the panel margins for Niri's reserved edges and disabled Kitty's additional window padding and margins for this instance. The shell supplies the content inset. Placement tests pass, and live screenshots on eDP-1 and HDMI-A-1 show the terminal contained within the rounded outline. Hide/reopen retains the same Kitty process, with keyboard exclusivity on its visible overlay and no input region on the shell extension. Saved column widths were restored after activation. Evidence is under `/tmp/opencode/dojo-verification/quick-terminal/`. Physical keyboard input was not exercised during verification.
