# Add subtle theme-aware shell and menu shadows

Status: resolved
Type: task

Give the workspace opening a restrained edge shadow and shell-attached menus
a soft drop shadow. Adapt the tint and opacity to the current background.

## Comments

The shared shell-extension painter adds a 16 px blurred shadow on free edges.
Attachment edges are masked so the menus still join the shell frame. The
workspace opening uses a weaker 10 px inset shadow with its existing corner
radius. Both use a faint background tint and lower opacity for lighter fills.
These are paint primitives and do not alter input regions or reservations.

Built the release and inspected power/Wi-Fi captures in isolated Kanagawa Dragon
and Catppuccin Latte sessions. Both appearances passed the eight fullscreen edge
comparisons, frame restoration, and an actual underlying-window click. Evidence:
`/tmp/dojo-agent-nested.aajobY` and `/tmp/dojo-agent-nested.BrCNhn`. The verification
driver's optional `--menu-screenshots` flag retains the menu captures for reruns.
