# 05 — CUA-derived desktop actions and motion under Dojo coordination

Type: task
Status: needs-triage
Blocked by: 02, 03
Effort: large; split by action family before implementation

## Brief

Adopt typed target/delivery contracts for coordinate click, double-click, move, scroll-at-point, key/hotkey, text, button down/up and in-window drag. Keep evdev low-level compatibility but do not force models to speak it. Every effect validates target/permission/coordination; partial effects release held input and report the completed prefix. Implement honest capability discovery before advertising action families.

Reuse CUA key pacing, sanitization and Linux input logic only where independent-seat delivery is preserved. Specify Unicode/layout/composition, terminal-safe typing and cancellation. Clipboard operations require explicit separate effects/approval and privacy policy, not hidden text insertion fallback. Cross-window drag-and-drop and window-frame/layout actions need separate compositor policy decisions and tests before enabling.

Import cursor-motion/effects crate and reconcile first arrival versus visual settling, click timing and velocity-preserving redirection. Preserve independent human focus/cursor and zero idle frames; update the motion spec for deliberate algorithm changes. Do not import a second global overlay/input controller.

## Acceptance

- Model-facing normalization tests for key aliases, finite coordinates, scroll units and bounded drag paths.
- Live native/primary-seat-only clients with two agents; human takeover mid-chord/text/drag releases state.
- Reset/lock/disconnect/output loss cancel movement and held input; no automatic action replay.
- Unicode and terminal cases state supported/refused semantics explicitly.
- Motion arrival before effects and redirected velocity meet updated acceptance contract.

## Comments

### Inactive-workspace acceptance

The complete native MCP/Pi/capture acceptance passed on inactive `dojo-tests`, with evidence at `/tmp/dojo-capture-live.JpTLYF`. The existing typing timeout was reproduced and traced to the nested EGL surface's default swap interval: each native call waited for the hidden host's throttled frame callback. The Winit fork now explicitly sets swap interval zero on its current draw surface, matching Smithay's requested non-vsync configuration. No action deadlines or takeover guards changed. Typing, holds, drag, actual target delivery, geometry/takeover interruption and disconnect cleanup passed alongside real contained perception and MCP cancellation/recovery. Temporary key timing logs were removed. Full action-family acceptance, Unicode, clipboard and cross-window drag remain open.
