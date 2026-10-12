# Shell efficiency audit

Status: ISSUES. Source audit only. No production edits, builds, benchmarks, UI sessions, or service changes were made.

Baseline HEAD is `7fb1e4ffa79843314624783fe8f302c734322caf`. Findings describe the dirty worktree read on 2026-10-11, including the current `TrayImageCache` fix. File line numbers refer to that worktree. The fixed tray image leak is excluded from open findings.

Scope covers the desktop frame, tray, shell theme polling, launcher, theme picker, and the relevant GPUI Wayland/WGPU resource paths. AGENTS.md, GLOSSARY.md, domain documentation, the fullscreen-frame spec, and the theming-system spec informed the audit. The unslop, codebase-design, technical-writing, and prove-it-works skills informed the report. All impact estimates below are source-based hypotheses until a live measurement establishes their size. Timer intervals describe code, not observed wakeup rates. Ranking prioritizes battery use while preserving capabilities.

## 1. Every output's desktop frame invalidates once per polling cycle

Priority P1. Confidence is high for the mechanism, unmeasured for CPU/GPU or battery impact.

`src/frame.rs:476-501` starts a detached loop for every `FrameView`. Each cycle starts `pgrep`, reads battery state, clones tray state, updates the image cache, and unconditionally calls `cx.notify()`. The timer is one second after the work finishes. `src/frame.rs:461-470` proves the process spawn. `src/power.rs:92-110` rescans the power_supply directory and reads type, capacity, and status files. Each display creates its own FrameView at `src/frame.rs:134-151`, so host-state reads multiply with output count.

The frame covers the complete output even though its center is transparent. `vendor/zed/crates/gpui_wgpu/src/wgpu_renderer.rs:1188-1210` renders and presents the acquired target. `:1509-1517` clears and stores the whole render attachment. A repeated notification therefore has a cost beyond the small tray controls, subject to compositor visibility and scheduling.

The smallest first correction is to compare the displayed state before notifying. Read shared host state once for all outputs and publish only changes. Battery changes can come from power events with a conservative fallback timer. Vigiland status can come from lifecycle notifications or a slower shared probe. Keep the current responsiveness for tray interactions.

Verify unchanged state causes no view invalidation, then exercise battery changes, vigiland start/stop, tray icon changes, and multiple outputs. Capture idle scheduler wakeups, process exec counts, GPU submissions, and full-output presents with equal output resolution, scale, refresh rate, brightness, and foreground workload. Measure energy separately before claiming savings.

## 2. Static speech bubbles keep the complete frame rendering

Priority P1. Confidence is high for unnecessary frame requests. Sustained pulse cost is an unmeasured design tradeoff.

`src/frame.rs:750-751` requests another animation frame whenever the speech phase is not Settled or any unexpired bubble exists. `src/speech.rs:101-118` makes entry animation finish after 0.35 seconds, with exit motion confined to the final 0.55 seconds. A settled bubble remains for 30 seconds at `src/speech.rs:97` and `:158-160`. Its long middle interval has constant opacity and geometry but still requests frames. Sent also has a constant pulse at `src/frame.rs:1200-1201`, yet falls into the animation condition. Listening, Transcribing, and Working deliberately pulse at `:1203`.

`vendor/zed/crates/gpui/src/window.rs:2729-2731` turns the request into next-frame view notification. GPUI may throttle inactive windows at `:1811-1824`; the exact running frame rate is unverified. The frame code also omits the reduced-motion check required for direct decorative frame requests by `:2724-2728`.

Schedule animation only while a bubble enters or exits. Use one deadline to wake for exit and expiry. Keep the active-phase pulse, but honor reduced motion and evaluate a lower animation rate on battery. Do not reduce responsiveness for pointer input or speech events.

Verify a settled bubble receives no animation requests through the middle interval, still exits at its deadline, and restarts correctly when speech extends its lifetime. Verify Sent remains visible without self-scheduled frames. Compare frame submissions during a conversation on high-refresh and ordinary displays, including reduced-motion mode.

## 3. The tray worker repeatedly wakes and rereads every item

Priority P1. Confidence is high for periodic work, unmeasured for cost.

`src/tray.rs:498-545` checks channels with `try_recv`, refreshes after 750 milliseconds, and sleeps for 25 milliseconds each iteration. The loop remains active with no registered items. `:563-565` refreshes each item serially. `:626-653` recreates the proxy and retrieves Id, Status, Title, IconName, IconPixmap, ItemIsMenu, and Menu for each active item. This obtains complete pixmap payloads even when nothing changed. Proxy caching and zbus internals may change the actual bus-call count, so no exact traffic count is claimed. `:483` gives each method a two-second timeout, and commands share this worker with refreshes.

Block on a combined command/shutdown/event receiver. Keep item proxies and respond to item change signals and bus name-owner events. Preserve a slow fallback refresh for providers with incomplete signals, and refresh immediately for registration. A smaller initial step can remove the 25-millisecond loop using a blocking command wait until the next refresh deadline. Avoid delaying menu actions behind a full sweep of slow providers.

Verify registration, removal, Passive/Active transitions, icon updates, menus, command latency, and shutdown. Include an empty tray and a provider that omits change signals. Measure wakeups and bus activity for unchanged trays and during real icon changes.

## 4. Path scratch textures are allocated even for scenes without paths

Priority P1 for capacity, P2 for battery until measured. Confidence is high for allocation requests, unmeasured for driver residency and energy.

`vendor/zed/crates/gpui_wgpu/src/wgpu_renderer.rs:1436-1437` unconditionally calls `ensure_intermediate_textures` for every frame. `:1383-1414` creates a full-target color intermediate and a same-size MSAA attachment. The sample count prefers 4, then 2, then 1 at `:2580-2583`. No `scene.paths` condition protects the allocation. Empty reservations render only `div().size_full()` at `src/frame.rs:1369-1373`. Wallpaper scenes use a raster image at `src/wallpaper.rs:15-22`. Both still go through this allocation path.

For a four-byte target format, the raw requested storage is width times height times four times one plus the MSAA sample count when multisampling is enabled. This is arithmetic from descriptors, not a VRAM measurement. Surface buffers, atlas textures, alignment, and driver behavior are additional factors.

Create the path textures only when a scene has paths to rasterize. Retain size-based reuse once allocated. Consider releasing them after sustained path-free usage only if measurements justify the extra lifecycle work. Preserve path antialiasing rather than lowering sample counts indiscriminately.

Verify path-free rendering never creates path attachments, then render actual paths, resize, and test device recovery. Compare GPU resource captures for blank reservation, wallpaper, launcher, and speech scenes. Check rendered pixels and all existing renderer tests before changing shared vendored code.

## 5. Wallpaper previews accumulate native-size decoded images in the global cache

Priority P1 for capacity, P2 for decode work. Confidence is high for ownership and native-size decode. Long-session memory growth is not measured here.

`src/theme_picker.rs:721-727` uses `img(analysis.path.clone())` for a preview only 160 or 300 logical pixels high. `vendor/zed/crates/gpui/src/elements/img.rs:543-547` sends that resource through `window.use_asset`, which calls `cx.asset_entry` at `vendor/zed/crates/gpui/src/window.rs:4263-4264`. `vendor/zed/crates/gpui/src/app.rs:3023-3041` explicitly caches loaded results until `remove_asset`. Picker code has no image-cache owner, asset removal, or atlas eviction, and closing its window at `src/theme_picker.rs:136-138` does not remove the application-global asset entries.

The decoder reads native-size pixels and converts to RGBA/BGRA at `vendor/zed/crates/gpui/src/platform.rs:3248-3256`, regardless of preview bounds. `src/theme_picker.rs:59-76` separately retains original compressed bytes. The existing 40-million-pixel limit allows 160 million bytes of raw pixel data per image. That is a theoretical size bound, not observed usage. GPUI also supports animated WebP frame decoding at `elements/img.rs:700-733`; this deserves a separate frame-budget check.

Own a bounded preview cache in the picker and evict the old image from CPU assets and window atlases on replacement or close. Create a display-sized thumbnail from the captured bytes, while retaining original bytes for Apply and Derive. Do not globally evict an image still used by another window. Launcher icons use the same default asset path at `src/launcher.rs:591-592`; their smaller normal dimensions make them lower priority, but absolute application icons are unconstrained by UI size.

Verify repeated distinct previews return to a bounded CPU/GPU resource count after replacement and after close. Check large JPEG, PNG, static WebP, animated WebP, orientation, and malformed inputs. Confirm Apply and Derive use the captured original content and do not silently apply the thumbnail.

## 6. Separate theme and wallpaper loops reread files every half second

Priority P2. Confidence is high for repeated reads and locking, unmeasured for battery impact.

`src/shell.rs:314-337` sleeps 500 milliseconds, constructs a ThemeStore, calls refresh_system, and reads shell config. `src/theme/apply.rs:540-550` acquires the apply lock and parses selection before checking whether system appearance changed. The lock creates the directory and opens apply.lock at `src/theme/store.rs:60-69`. Selection JSON reads happen at `:127-133`. Follow-system mode opens a darkman control socket and exchanges data at `src/theme/apply.rs:138-147`.

`src/config.rs:137-156` reads both config.toml and current/dojo.json before its equality shortcut. The shortcut correctly avoids repeated parsing, but does not avoid reads. `src/wallpaper.rs:106-142` has a separate 500-millisecond file-read loop. Shell invalidation already compares resolved colors at `src/shell.rs:341-345`, so the issue is repeated background work, not unconditional color redraws.

Subscribe to appearance changes and watch the parent directories of config and selected theme generations. Watch directories rather than only files, because publication replaces symlinks and files. Coalesce notifications and keep a slower reconciliation fallback. Reuse the theme Paths and share publication notifications between shell colors and wallpaper.

Verify external config edits, atomic rename, generation switches, rollback, darkman restart, missing files, and rapid consecutive Apply operations. Preserve the previous valid theme on malformed input. Compare file reads, locks, background-task starts, and sockets during idle before measuring energy.

## 7. Four blank reservations create full GPUI renderer instances per output

Priority P2 for startup and capacity. Confidence is high for resource creation, unmeasured for total residency.

`src/frame.rs:64-118` opens four reservation windows plus the visual frame. Each Wayland window creates `WgpuRenderer` at `vendor/zed/crates/gpui_linux/src/linux/wayland/window.rs:572-591`. A renderer core creates its own pipelines at `vendor/zed/crates/gpui_wgpu/src/wgpu_renderer.rs:1229-1237`, and its initial native instance buffer requests 2 MiB at `:1280-1284`. Atlas allocation is lazy but uses at least 1024 by 1024 pixels when a texture is needed at `vendor/zed/crates/gpui_wgpu/src/wgpu_atlas.rs:183-215`.

Retain four edge reservations and their layer-shell behavior. Investigate a reservation-only window path with a minimal transparent buffer and no general renderer, or share immutable pipeline objects where compatible. First apply finding 4, which removes unnecessary scratch textures without adding a special window lifecycle.

Verify all reserved edges, fullscreen frame visibility, input pass-through, output removal, scale changes, and failed partial initialization. Measure window creation and resource allocations rather than extrapolating RSS from declared buffer sizes.

## 8. The launcher resolves icons for every result before showing ten rows

Priority P3. Confidence is high for excess lookup scope, unmeasured for user-visible delay.

`src/launcher.rs:253-275` resolves all matched icons on each uncached result set. `src/launcher/catalogue.rs:13` bounds results to 64. Render displays at most ten rows at `src/launcher.rs:587`, so a broad query resolves icons the user may never see. `src/launcher/icons.rs:7` also calls default_theme_gtk for each icon, though `.with_cache()` already caches library lookup work. This is bounded interactive work, not an idle leak.

Resolve the visible ten rows plus a small look-ahead batch, then request more on selection/scroll changes. Determine the icon theme once per batch. Preserve the existing missing-icon cache and off-main-thread resolver.

Verify keyboard navigation and fast queries show the correct icons, stale jobs cannot overwrite a new catalogue's entries, and scrolling requests the newly visible icons. Measure catalogue sizes and icon lookups before deciding whether this is worth implementing.

## Existing efficient behavior

- The live `TrayImageCache` reuses unchanged RenderImage identities and calls `window.drop_image` for replaced or removed pixmaps at `src/tray.rs:95-124`. Treat the earlier leak as fixed. Its full-pixmap clones and equality comparisons are subordinate to finding 1.
- Launcher work waits on a Condvar while idle, reuses its matcher and catalogue, and stores only the newest pending request at `src/launcher/worker.rs:43-95` and `:120-127`. The ready channel is bounded to one message. Results are generation-checked at `src/launcher.rs:227-232`.
- Launcher results are limited to 64 and only ten rows render. Icon resolution and catalogue discovery run outside the UI thread.
- Theme config content equality prevents repeated parsing. Resolved color equality prevents repeated refresh_windows. Wallpaper replacement only notifies on path change at `src/wallpaper.rs:52-56`.
- Tray menus load on user request. The temporary 10-millisecond menu lookup loop is bounded to 50 attempts at `src/frame.rs:549-566`, so it is not a permanent idle timer.
- Output reconciliation is event-driven and generation-checked after a 350-millisecond delay at `src/shell.rs:410-434`.
- WGPU atlases free tile space and release unreferenced textures at `vendor/zed/crates/gpui_wgpu/src/wgpu_atlas.rs:139-157`. Their minimum allocation size alone is not evidence of a leak.

## Coverage and measurement limits

No runtime cost or battery saving is quantified. Source evidence proves scheduled work and requested resource lifetime, not energy usage or physical GPU memory residency. Compositor occlusion, GPUI inactive throttling, driver caching, and current services can affect results. The parent audit owns live observations.

This audit does not inspect the extraction algorithm's internal cost, speech worker process scheduling, unrelated configuration daemons, or Niri's compositor implementation. Path antialiasing, input hit tests, fullscreen placement, shared images, and atomic theme publication are capability constraints for any follow-up. No production improvement is authorized by this report itself.
