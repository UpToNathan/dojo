# Bounded screenshot extension design

Keep the public MCP tool as `desktop_screenshot` and retain the required `output: String` field, so every existing call remains valid. Add optional, `deny_unknown_fields` request fields for `region: Option<PixelRegion>`, `window_id: Option<u64>`, and `scale: Option<f64>`. This is smaller and more compatible than a tagged target: output selection remains common to all modes, and omitted fields preserve current behavior. Validate that region and window are mutually exclusive; scale defaults to 1.0 and is bounded (for example, 0.25–2.0) before invoking grim. Also enforce output pixel and PNG byte budgets after capture, as today.

Suggested request/module shape:

```rust
struct PixelRegion { x: u32, y: u32, width: u32, height: u32 }
struct DesktopScreenshot { output: String, region: Option<PixelRegion>,
    window_id: Option<u64>, scale: Option<f64> }
enum CaptureTarget { Output, Region(PixelRegion), Window(u64) }
fn capture(socket: Option<&Path>, request: &DesktopScreenshot) -> Result<Vec<ContentBlock>>;
fn resolve_window_rect(output: &LogicalOutput, workspaces: &[Workspace],
    windows: &[Window], id: u64) -> Result<PixelRegion>;
```

Keep output capture as `grim -o NAME -s SCALE -`; region mode adds grim’s `-g 'X,Y WIDTHxHEIGHT'` geometry. The existing `-o` keeps coordinates output-relative, including when the output has negative desktop coordinates. Define integer region coordinates in output-relative logical pixels, clip the requested rectangle against `[0,width) × [0,height)`, and reject an empty intersection. Convert to grim geometry using the clipped integer rectangle; report the clipped rectangle and actual PNG-to-logical X/Y factors, since rounding and scale can make them differ. Existing responses continue reporting output dimensions/mapping unchanged.

For window mode, query Niri `Windows` and `Workspaces` over the same IPC socket and use only the window’s geometry on its active workspace. `WindowLayout` provides fractional `tile_pos_in_workspace_view`, `tile_size`, and `window_offset_in_tile`; use the visual tile rectangle, rounding outward to integer bounds, and intersect it with the requested output. Require the workspace to be active on that output; error for missing/stale ids, unavailable geometry, or no visible intersection. This handles negative desktop origins by subtracting the output’s logical origin before clipping, and partially off-output windows by clipping. Do not switch focus or workspace. Grim remains clipboard/focus-neutral; Niri’s screenshot request changes the clipboard, and `grim -T` takes a foreign toplevel handle rather than Niri’s window id.

Pure tests should cover region clipping at each edge, empty/overflowing rectangles, negative global origin conversion, partial window clipping, inactive workspace and missing geometry errors, outward rounding, scale bounds, and target mutual exclusion. No live compositor is needed for these geometry tests.
