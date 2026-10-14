# Targeted desktop observation

Extend Dojo's existing desktop_screenshot with bounded region and positioned
window capture and reduced image scale. Keep output-only calls valid. Use grim
without changing focus, clipboard, display configuration, or agent seat lifetime.
Return actual PNG dimensions, logical origin, and per-axis mapping. Reject
unavailable geometry rather than guessing. Preserve current timeout and image
budgets. Check capture geometry again before returning.

Window capture observes the visible desktop, including occlusion. The current
Niri compositor reports floating positions but omits tiled positions. Tiled
window capture is unavailable until that geometry exists. No compositor restart
or fork change is part of this unit.

Verify parser/geometry/mapping behavior and actual nested MCP crops, scaling,
window selection, click mapping, no focus/clipboard changes, and rejected inputs.

The linked UniiChat gist is for discussion only. No memory system, prompt,
provider, global agent configuration, or unrelated runtime is changed.
