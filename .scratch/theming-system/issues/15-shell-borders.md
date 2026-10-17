# Add subtle shell and menu borders

Status: resolved
Type: task

Draw a 1 px outline at 10% opacity using the current theme's border color
around the workspace opening and the free edges of shell-attached menus.
Keep the rounded attachment curves and existing shadows.

## Comments

The shared menu painters reuse the fill contour for the outline and omit
attachment edges. The frame outline sits inside the workspace opening.
All callers, including the launcher, theme picker, and quick terminal, pass
the current border token.

Built the desktop and verification helpers in release mode. Inspected power
and Wi-Fi captures in Kanagawa Dragon and Catppuccin Latte. Both sessions
passed all eight fullscreen edge comparisons, frame restoration, and a click
through the workspace opening. Evidence: `/tmp/dojo-agent-nested.vz9r7z` and
`/tmp/dojo-agent-nested.yu2AVL`. The verification driver accepts release fixture
and pixel-diff paths through environment variables.

Reduced the shell and menu outlines to 10% opacity at the user's request.
