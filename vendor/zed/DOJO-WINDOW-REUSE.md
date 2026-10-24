# Dojo patch: shared pipelines and hide/show for layer surfaces

Dojo carries two changes on top of the GPUI fork's window code (the same base as the
lock-screen and AT-SPI patches, Zed `cc94985f7042574f569e0b6c7b6bdd27975a4579`). They cut the
cost of opening a Dojo window; measurements are in
`docs/research/dojo-launcher-performance.md`. Neither touches the session-lock window kind.

## Shared render pipelines (`crates/gpui_wgpu`)

- `wgpu_renderer.rs`: `WgpuRendererCore::new` asks `WgpuContext::shared_pipelines()` for the bind
  group layouts and `WgpuPipelines` instead of building them. `SharedPipelines` holds a small
  `KeyedCache` keyed by `PipelineKey` (target format, alpha mode, path sample count, dual-source
  blending, WebGL instance data). `WgpuPipelines` and `WgpuBindGroupLayouts` derive `Clone`
  (wgpu handles are reference counted).
- `wgpu_context.rs`: `WgpuContext` owns the cache in an `Arc<Mutex<..>>`, shared by its clones. A
  device recovery replaces the context, so the cache never outlives its device.
- `wgpu_renderer.rs`: `WgpuRenderer::restore_surface` creates a surface for a window after
  `unconfigure_surface` released the old one, using the size, alpha mode and present mode the
  renderer last had.
- Tests: `keyed_cache_builds_each_key_once`, `pipeline_keys_differ_in_every_field` and
  `renderers_on_one_device_share_pipelines`, in `wgpu_renderer.rs`.

## Hide and show (`crates/gpui`, `crates/gpui_linux`)

- `gpui/src/platform.rs`: `PlatformWindow::hide` and `show`, defaulting to `false` ("cannot be
  hidden"). `gpui/src/window.rs`: `Window::hide` and `Window::show`, the latter also refreshing the
  window. `gpui/src/platform/test/window.rs` and `app/test_context.rs` implement and expose them
  for tests (`TestAppContext::window_is_hidden`).
- `gpui_linux/src/linux/wayland/window.rs`, layer-shell windows only (other kinds return
  `false`, and so does a compositor with layer shell version 1):
  - `hide` sets keyboard interactivity to none, attaches a null buffer and commits, releases the
    swapchain and path textures (`unconfigure_surface`), parks the frame loop in the new
    `FrameLoop::Hidden` and reports `WindowVisibility::Hidden` from the executor (it can run inside
    a frame that holds the callbacks).
  - `show` restores the surface, sends the layer, size, anchor, margin, exclusive zone and
    keyboard interactivity again, commits without a buffer and waits for the new configure, whose
    handling starts the frame loop as for a new window. Niri resets a layer surface to the
    background layer on unmap, so the layer must be sent again or the surface lands under the
    windows without keyboard focus. If a compositor configures the surface while it is hidden, the
    configure is acknowledged and `show` draws without waiting.
  - A hidden window ignores `draw`, because a buffer on an unmapped layer surface before its new
    configure is a protocol error, and acknowledges configures without laying anything out.
  - `WaylandLayerSurfaceState` keeps its `LayerShellOptions` for `show`, and
    `apply_layer_options` is the creation code, moved so both paths share it.

The launcher is the only user (`src/launcher.rs`). Wayland behaviour is checked against Niri by
`tools/agent-input/run-launcher-reuse.py`, which needs the nested session described in the README.
