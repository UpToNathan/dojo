# Graphics resource changes

Implemented in `vendor/zed/crates/gpui_wgpu/src/wgpu_renderer.rs` and `src/theme_picker.rs`. The current worktree changes are preserved. The parent handled the vendored README review marker required by vendor/zed/AGENTS.md.

Path intermediate and MSAA textures now allocate only for scenes containing paths. Existing size checks, antialiasing selection, resource invalidation, and device recovery remain authoritative. A path-free scene does not resize previously allocated path scratch textures. The next path scene resizes them as required.

Picker Analysis now owns an orientation-correct, BGRA RenderImage thumbnail alongside the original compressed bytes. Thumbnail bounds derive from the preview width, its 300-pixel maximum height, and window scale, with a 4096-pixel edge ceiling. Header dimension validation still runs before native pixel decode and retains the existing 40-million-pixel and 16384-edge input caps. Thumbnail decode runs in the existing background analysis task. The UI passes the owned RenderImage directly to img, bypassing the global path asset cache. Replacement explicitly drops the old image from its owning window's atlas. Toggle, Escape, and outside-click close paths do the same. A window-scoped entity-release callback covers release while the window remains available. Once the window is gone its renderer teardown owns GPU resource release. Existing revision checks discard stale analysis and derivation results. Close increments the revision too.

Apply and Derive keep exact captured original bytes. Derivation reuses the thumbnail identity rather than decoding or uploading a second preview. JPEG, PNG, and static WebP preserve their dimensions, orientation, crop behavior, and colors. Animated WebP retains animation delays and colors within a limit of 32 frames and 16 MiB of retained thumbnail pixels. Larger animations use a still first-frame picker preview. Each animated thumbnail frame has a 2048-pixel edge ceiling. Frame decode is incremental, so completed native-size frames do not accumulate. Apply retains the original wallpaper animation bytes.

The focused GPU test `headless_path_resources_are_lazy_and_survive_scene_changes` uses the actual headless WGPU renderer. It renders blue raster and red path scenes, reads their pixels, checks actual scratch-texture creation and dimensions, alternates scene types across a resize, invalidates resources, and verifies path rendering recreates them. Initial path-free rendering must leave both path scratch textures absent.

Picker tests cover unchanged original bytes and bounded thumbnail pixels for PNG, JPEG, and WebP. They inspect BGRA colors and actual RenderImage dimensions. A JPEG fixture carries EXIF orientation 6, so the thumbnail must rotate to the expected shape. Another test rejects an excessive-width input and successfully decodes the adjacent valid width. An actual animated WebP fixture checks two differently colored frames and their delays, then verifies the first-frame fallback when a 33-frame input exceeds the budget. A stream of large frames checks the total-pixel fallback.

`rustfmt --edition 2024 src/theme_picker.rs vendor/zed/crates/gpui_wgpu/src/wgpu_renderer.rs` and `git diff --check` pass. The actual headless renderer resource/pixel test passed through the isolated authoritative-manifest runner. The focused animation fixture passed after its initial failure exposed unwanted thumbnail upscaling, which is now prevented by native-dimension bounds. Full commands and evidence are in [gpu-test-result.md](gpu-test-result.md). The parent owns final root test coverage and live verification.

No live UI, service, host configuration, or launcher change was made. No performance or battery savings are claimed. Live verification still needs repeated picker replacements/close with atlas counters, fullscreen/input checks, output resize and scale changes, and GPU resource capture.

Parent live evidence: five resident picker opens/closes at current scale1.75
passed, with maximum4KiB closed GPU growth and final allocation equal to baseline.
Preview image visually inspected. Evidence /tmp/dojo-optimization/live/picker-lifecycle.json.
Within-picker replacement and output scale changes remain outside this live check.
