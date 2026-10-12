## Problem

The shell currently multiplies host sampling by output count, while the tray host spends most of its observed CPU in short channel checks and periodic D-Bus property sweeps. Theme and wallpaper consumers also poll files, and speech animation/lifetimes need deadline-driven redraws without evicting the prewarmed model. The architecture must keep D-Bus and filesystem boundaries outside GPUI, preserve SNI/dbusmenu and newline JSON protocols, retain atomic theme publication, and keep each output's window and input resources independent.

## Usage (caller's view)

The frame subscribes once to shared machine state and registers each output as a recipient. Views receive only changed snapshots and notify only when their own visible state changes:

```rust
let host = DesktopEvents::start(tray_config, theme_root)?;
let output = host.subscribe_output(output_id);

cx.spawn(async move |view, cx| {
    while let Some(change) = output.next().await {
        view.update(cx, |this, cx| this.apply(change, cx)).ok();
    }
});
```

Tray actions remain request/response-free commands; menus and current tray state come from the latest shared snapshot:

```rust
host.tray().activate(item_id);
let menu = host.tray().menu(item_id);
host.tray().send_menu_event(item_id, menu_id, event);
```

A speech bubble holds a deadline subscription only while it has a visual transition or an expiry scheduled. The voice coordinator retains session/model ownership independently of shell visibility:

```rust
let animation = speech.present(event, Instant::now());
while let Some(frame) = animation.until_next_change().await {
    cx.notify();
}
```

## Shape

```rust
// desktop_events.rs: one owner for host-wide observation and lifecycle.
pub struct DesktopEvents { inner: Arc<DesktopRuntime> }
pub struct OutputSubscription { receiver: watch::Receiver<Arc<OutputSnapshot>> }
pub struct OutputSnapshot { pub tray: Arc<[TrayItem]>, pub theme: ThemeGeneration, pub host: HostSnapshot }

impl DesktopEvents {
    pub fn start(config: DesktopConfig) -> Result<Self, StartError>; // not implemented
    pub fn subscribe_output(&self, id: OutputId) -> OutputSubscription; // not implemented
    pub fn tray(&self) -> TrayCommands; // not implemented
}
impl Drop for DesktopRuntime { /* TODO: cancel task tree, close bus/watch handles, join blocking boundary workers */ }
```

`DesktopRuntime` owns one bounded command channel, one shared latest-value snapshot, and cancellation tokens for its actor tasks. Per-output subscriptions are watch channels: slow views coalesce obsolete snapshots rather than accumulating a queue. The frame compares the fields it draws before requesting a GPUI frame. Output-specific windows, image caches, hit regions, and renderer resources remain local to each view.

```rust
pub struct TrayCommands { tx: mpsc::Sender<TrayCommand> }
pub enum TrayCommand { Activate(ItemId), Context(ItemId), Menu(ItemId, MenuEvent) }
pub struct TrayActor { /* private Connection, retained item proxies, name owners, menu snapshots */ }
async fn run_tray_actor(actor: TrayActor, commands: Receiver<TrayCommand>, changes: Sender<TrayChange>, cancel: CancellationToken); // not implemented
```

The tray actor waits on command receive, scoped zbus item/property/name-owner signals, and a long reconciliation deadline in one select loop. It owns retained proxies and parses wire properties into domain snapshots. Signal changes refresh only affected items; registration/removal and provider restart repair the proxy set. The slow reconciliation sweep repairs lost/misordered provider events without serially blocking menu commands. Commands are bounded; if full, the UI reports/retries the action rather than hiding unbounded work. Bus waits and retry backoff are cancellation-aware.

A single native watcher boundary reports changed paths to the same runtime; it never publishes partial theme state:

```rust
pub struct FileChanges { rx: watch::Receiver<FileGeneration> }
pub async fn watch_publications(root: PathBuf, cancel: CancellationToken) -> Result<FileChanges, WatchError>; // not implemented
```

Watch the parent directories and treat events as invalidation hints. Re-read `current` and `selection.json` after rename, validate the selected generation, then publish one immutable generation snapshot. Debounce bursts; a slow reconciliation deadline handles missed events. The theme store remains the sole writer and retains its staged-write/rename protocol.

Resource owners keep their state close to use. `TrayImageCache` drops replaced/removed GPUI images on the owning window. Theme previews use a bounded LRU of display-sized decoded images, keyed by generation and dimensions; source bytes remain available for Apply/Derive, and picker close drops preview handles. WGPU path scratch textures are `Option<PathScratch>` and allocated on the first path-bearing scene, resized only then, and released with renderer teardown. No cross-window GPU handle is shared.

Speech animation stores `next_deadline` and a phase (`Entering`, `Static`, `Exiting`, `Done`); timers are scheduled only for entry/pulse/expiry boundaries. The voice service remains the sole coordinator and retains the warmed worker across bubble expiry and idle UI. Startup lock precedes preload/socket replacement; shutdown cancels capture, closes subscriptions and worker stdin, closes socket, then joins. No idle eviction or system sleep policy change is part of this sketch: either could break active work or fast speech.

The public surface is deliberately small: one runtime, one output stream, and a tray command handle hide transport, reconnection, debounce, and reconciliation policy. GPUI keeps only presentation decisions; no wire or watcher types escape. No task owns a permanent interval, no view polls shared state, and all queues have explicit capacity and full behavior.

## Synthesis decision

Candidate A favors one host runtime with an event-driven tray actor and latest-value subscriptions. Its central ownership makes host-wide state observable once and lets each output independently consume coalesced changes.

## Tradeoffs accepted

- We accept an occasional full reconciliation sweep in exchange for repairing imperfect providers without continuous polling.
- We accept coalescing intermediate view states in exchange for bounded memory and presenting the latest state.
- We accept delayed model eviction in exchange for warm push-to-talk response and preserved speech capability.

## Alternatives considered

- Keep a polling worker per frame: simplest local calls, but duplicates host work and leaks cadence policy into each output.
- Give each output a D-Bus subscription: isolates consumers, but duplicates bus connections/proxies and complicates ordering and recovery.
- Push every event through per-view unbounded queues: preserves every intermediate event, but exposes backpressure policy and can retain stale tray/theme work when a view stalls.

## Open questions and risks

Can zbus expose all required SNI property and owner changes with retained proxy subscriptions for the providers in use? Which events require fetching full pixmaps versus updating only status/title? What bounded command capacity preserves current input responsiveness under a hung provider? Confirm GPUI permits watcher tasks to update all output views safely through the chosen watch primitive.

## Next implementation step

Implement the tray actor and latest-value publication behind the current `TrayHandle`, then validate registration, icon/status changes, menus, provider restart, multiple outputs, shutdown, and lost-signal reconciliation before moving other pollers.
