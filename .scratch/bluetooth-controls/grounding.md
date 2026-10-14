# Dojo top-bar and host-service grounding

## Overview

The top bar is drawn by each display's full-screen transparent GPUI layer-shell frame in `src/frame.rs`. It shares that surface with the desktop surround, while Niri edge reservations are separate transparent windows. Tray UI state and hit-testing live in each `FrameView`; the StatusNotifier D-Bus host and its async/blocking bridge live in `src/tray.rs`. Process-wide desktop services are initialized from the GPUI application startup in `src/shell.rs`.

Bluetooth belongs beside the existing audio service as a host/backend module with GPUI global state and a narrow command interface. Keep BlueZ discovery/control and event listening outside `FrameView`; the view should observe state and translate user clicks into commands. Startup of a Bluetooth service should be independent of frame enablement if Bluetooth state is expected to exist while the frame is disabled; `ShellController::enable` is specifically where the tray host is started/stopped.

## How the current flow works

`main` declares modules; the `application().run` closure in `src/shell.rs` initializes global theme/speech/host/audio state before creating `ShellController`. It calls `frame::initialize_host_state(cx)` and `audio::initialize(cx)` once for the GPUI app. The audio module (`src/audio.rs`) is a useful nearby pattern: `AudioState` is a GPUI global; worker threads own external command/event I/O; an async GPUI task transfers updates back through `cx.update_global`; each `FrameView` observes the global and calls `cx.notify()` when it changes.

When enabled, `ShellController::enable` starts `TrayService`, obtains a cloneable `TrayHandle`, and passes it to every active display's `DisplayFrame::open`. On display changes, `reconcile_active_displays` reuses valid frames or opens replacements with the same service handle. Disable/quit shuts down the tray service and closes frames. `TrayService` owns a named worker thread; `run_host` registers both freedesktop and KDE StatusNotifier watcher interfaces on the session bus, tracks item registration and owner changes, reads item/menu properties, and handles queued activate/menu commands. `TrayHandle` exposes snapshots plus bounded change notifications. Each `FrameView` subscribes and refreshes icons/menu data on changes.

In `FrameView::render`, non-passive tray items become buttons; left click selects activate/show-menu/context-menu based on item metadata, right click opens a menu when available, and middle click requests secondary activation. Menu requests go through `TrayHandle`; D-Bus menu actions are queued back to the tray worker. Menu layout, submenu hover, and rendered bounds are local to that view.

The top bar is placed at the upper right in the full-screen frame (`TOP_BAR_HEIGHT`, `RIGHT_INSET` from `src/geometry.rs`). `window.set_input_region` is the actual Wayland hit-test gate: normally the frame accepts input only in the top bar and launcher button bounds; with a tray menu open it accepts input across the whole viewport so outside clicks can reach its dismissal handler. The handler is installed on the shell for left/right/middle mouse-down and closes the menu if the point is outside the main and submenu bounds. Speech bubbles temporarily compose their own blur/input bounds with the frame regions.

## Where things live

- `src/frame.rs`: `DisplayFrame::open`, `FrameView::new`, top-bar layout and controls, input regions, menu event handling and outside dismissal.
- `src/shell.rs`: `ShellController::enable/disable`, display reconciliation, and the GPUI application initialization point.
- `src/tray.rs`: `TrayItem` / `TrayHandle` / `TrayService`, D-Bus host, worker commands, signals, and menu snapshots.
- `src/audio.rs`: current example of a process-wide host service exposed to the top bar as GPUI global state.
- `src/geometry.rs`: top-bar dimensions and edge insets.

## Gotchas and constraints

- The top bar is not a separate window. A new control must be laid out in `FrameView` and fit the dynamically sized `top_bar_width`; its click behavior must remain inside the top-bar input region.
- A visual element outside the configured input regions is not interactive. When a menu is open the whole viewport becomes interactive, so dismissal depends on accurate main-menu **and** submenu bounds. Preserve propagation behavior for controls such as the audio drag handler, which calls `cx.stop_propagation()`.
- There is one `FrameView` per display. Shared device state should be a GPUI global (or shared backend handle) observed by all views, not copied into per-display backend workers.
- Tray startup is deliberately tolerant: failure logs and continues with an empty default handle. Bluetooth availability should likewise have an explicit unavailable state so missing BlueZ does not prevent shell startup.
- Current workspace edits include user-owned audio-wave changes in `src/frame.rs` and `src/audio.rs` (plus related untracked audio work). Preserve those changes while touching top-bar code. `git status` also shows edits in `src/shell.rs`, so integrate there without reverting existing work.
- The host note for this task says Arch currently has `hci0`, but BlueZ/the service is absent and sudo needs a password. Do not treat hardware presence as evidence that a BlueZ D-Bus API can be exercised in this environment; architecture can be grounded from source, but backend behavior needs a host with BlueZ running.
- `GLOSSARY.md` defines the desktop frame/reserved edge vocabulary. No `docs/adr/` directory is present in this checkout.
