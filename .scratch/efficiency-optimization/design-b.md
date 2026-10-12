# Candidate B: preserve owners, replace recurring work

## Usage (caller's view)

The shell starts its existing tray service and one shared desktop observation entity. Each output retains its own window, input regions, and image cache.

```rust
let tray = TrayService::start()?;
let desktop = DesktopState::start(tray.handle(), cx)?;
FrameView::new(desktop.clone(), window, cx);

// FrameView observes one entity; unchanged snapshots emit no event.
cx.subscribe(&desktop, |view, _, change, cx| {
    view.refresh_visible_state(change, cx); // image work stays window-local
});

// Existing action boundary remains; bounded enqueue reports congestion.
tray.handle().activate(&item, TrayAction::Activate)?;
```

```rust
// Picker replacement/close releases registered render images explicitly.
self.previews.replace(request_id, original_bytes, bounds, window)?;
self.previews.clear(window);
```

```javascript
// Both systemd and CLI fallback enter under the same process-held flock.
await serveWithOwnership(); // owns socket, coordinator, warmed worker
service.changed(event);     // recomputes its single nearest deadline
```

## Problem

The tray rebuilds proxies and fetches unchanged pixmaps every 750 ms while waking every 25 ms. Every desktop frame repeats host probes and redraws. Several graphics resources outlive their contribution to pixels. Reduce this work inside the current owners, preserving SNI/KDE, dbusmenu, speech readiness, atomic theme publication, visuals, and input. Audit CPU attribution prioritizes tray work; future savings remain unmeasured.

## Shape

The blocking tray command owner remains one thread. One cancellable async signal bridge on the existing zbus executor forwards typed invalidations and coalesces wakeups. It uses connection-level matches and a mutable address filter, rather than one task/thread per item. Watcher registration callbacks notify the same mailbox. Only the command owner writes item/menu snapshots and cached proxies.

```rust
// src/tray.rs; all transport types private
struct TrayOwner<'bus> {
    items: BTreeMap<ItemKey, CachedItem<'bus>>,
    inbox: Arc<Mailbox>,
    fallback: Option<Instant>,
}
struct CachedItem<'bus> {
    address: ItemAddress,
    owner: UniqueBusOwner,
    proxy: zbus::blocking::Proxy<'bus>,
    snapshot: TrayItem,
    dirty: DirtyFields,
}
struct Mailbox {
    // One mutex + condvar protects bounded command FIFO, coalesced dirty
    // keys, global rescan bit, and stopping bit; notification cannot be lost.
    state: Mutex<InboxState>, wake: Condvar,
}
struct SignalBridge { cancel: Cancellation, completion: Completion }
impl TrayOwner<'_> {
    fn run(&mut self) -> anyhow::Result<()> { todo!("not implemented") }
    fn reconcile_one(&mut self, key: ItemKey) { todo!("not implemented") }
}
impl TrayHandle {
    pub fn activate(&self, item: &TrayItem, action: TrayAction)
        -> Result<(), TrayBusy> { todo!("not implemented") }
}
```

Commands run before one bounded reconciliation unit; then the owner waits until work or the nearest fallback deadline. Finite D-Bus method timeouts bound each unit, though a misbehaving provider can still delay input by one call. Command FIFO overflow returns an error to the UI; signal overflow sets a rescan bit. Cancellation bypasses queue capacity. Never hold the mailbox or publication lock during D-Bus calls. No fallback deadline exists for an empty registration set; nonempty trays receive a slow authoritative sweep.

Retain proxy identity but disable zbus property caching for owner reads. SNI providers can emit NewIcon/NewStatus without PropertiesChanged, so cached get_property values would make reconciliation stale. Subscribe to both interfaces, PropertiesChanged including invalidated fields, NameOwnerChanged, and dbusmenu layout/property signals. Owner changes discard proxies, icons, and menu snapshots; generation-tagged invalidations cannot resurrect an earlier owner. Parse names, paths, property payloads, pixmap dimensions/lengths, and menus at this boundary. Snapshot equality suppresses publication.

Shutdown marks stopping, wakes the owner, cancels the bridge, and joins both before dropping the connection. A stream awaiting a message must select cancellation. Joining before cancellation, waiting for queue space during teardown, or allowing callbacks to retain the mailbox/connection creates deadlocks or leaks.

For zbus 5.19, build proxies with `blocking::proxy::Builder::cache_properties(CacheProperties::No)`. Spawn the bridge through `connection.inner().executor()`; bounded `MessageStream::for_match_rule` streams select cancellation, then `AsyncDrop::async_drop` removes matches before completion. GUI subscribers receive one latest revision, never queued snapshots.

```rust
// src/frame.rs; one GPUI entity for all outputs
struct DesktopState { visible: DesktopSnapshot, observers: Observations }
impl DesktopState {
    fn start(tray: TrayHandle, cx: &mut App)
        -> anyhow::Result<Entity<Self>> { todo!("not implemented") }
    fn reconcile(&mut self, dirty: DesktopDirty, cx: &mut Context<Self>)
        { todo!("not implemented") }
}
// src/theme_picker.rs; picker owns original bytes and bounded thumbnails
struct PreviewImages { current: Option<PreviewAsset>, generation: u64 }
impl PreviewImages {
    fn replace(&mut self, id: u64, original: Arc<[u8]>, bounds: PreviewBounds,
        window: &mut Window) -> anyhow::Result<()> { todo!("not implemented") }
    fn clear(&mut self, window: &mut Window) { todo!("not implemented") }
}
```

Desktop observations aggregate tray notifications, host events where available, and directory watches into one dirty set and one nearest reconciliation deadline. Unsupported host probes run once globally at the fallback deadline. Watch the theme root containing the atomically replaced current link; validate committed targets and retain prior wallpaper on failure. Generation/selection publication and external-file ownership checks remain unchanged. Frames compare their visible projection before notifying; animations schedule only affected outputs. Speech bubbles animate entry/exit/pulses and sleep until expiry during their static middle.

Picker thumbnails use RenderImage, bypass original-path app caching, bound decoded pixels and outstanding requests, reject stale completions, and drop replaced/closed images through Window::drop_image. Keep original bytes for Apply/Derive. WGPU allocates path scratch at the first actual path batch, keyed by target size/format/sample count/device generation. Empty/raster-only renderers allocate none; teardown/device recovery clears scratch. Keep reusable scratch after path rendering; investigate eviction separately.

Voice retains its coordinator and warm native worker. Lock precedes preload, unlink, and bind; only the owner removes its socket. One timeout handles settlement, reconnect, persistence flush, and eligible WebSocket close. Retain active-run subscriptions and sequence recovery; flush bounded history on deadlines/shutdown. Preserve 16 KiB newline commands, 45-second request deadlines, stale request rejection, and closed microphone outside capture. No model eviction or arbitrary suspend policy.

The public interfaces hide invalidation, transport, scheduling, and resource lifetime within existing owners, per boundary-discipline and separate-before-serializing-shared-state.

## Tradeoffs and alternatives

We accept one bounded blocking-call delay in exchange for retaining a small tray ownership change. An all-async actor hides call latency better but requires cancellation-safe command execution and broader migration of watcher registration/proxy lifetimes. Per-item tasks expose duplicate scheduling and shutdown rules. Global renderer sharing requires a separate window/device design and loses against local allocation gating.

## Synthesis decision

Pending comparison by the orchestrator.

## Risks and verification gates

Can connection-level signal matches preserve both watcher protocols across bus restarts? Prove registration, owner replacement, missed/invalidated properties, queue overflow, menu responsiveness, and blocked-provider shutdown before removing polling. Verify unchanged snapshots cause no notifications, output hotplug/input remains correct, and atomic publication handles watch loss. Exercise preview replacement/close, late decode completion, path/raster alternation, resize/device recovery, speech deadlines/reconnect, concurrent startup, and warmed push-to-talk. Measure optimized deployment CPU, wakeups, PSS, actual DRM allocations/activity, frame submissions, bus traffic, and latency with the audit readers; compare battery discharge only under matched unplugged workloads.

## Next implementation step

Implement the tray mailbox and cancellable bridge first, with behavior tests for invalidation, fairness, and teardown.
