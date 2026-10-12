# Active optimization result

The optimized shell runs as PID 691605 from versioned installation
`b06241421be972fc-mmap128k`. Its previous default-allocator release remains
available for rollback. The updated voice singleton is active. Tested release
Niri `4677bd2c0bdd6fea` is selected for the next login; active Niri PID 604052
remains on `91de956b9ee4ee61`.

## Current resident observations

After five actual picker opens and closes, five approximately five-second quiet
windows on the 2880 by 1800, 60 Hz, scale-1.75 desktop produced these counters:

| Process | CPU, percent of one logical core | PSS | DRM client GPU memory | GPU execution |
|---|---:|---:|---:|---:|
| Shell | median 0.196; range 0.00 to 1.368 | 103.99 MiB | 404.32 MiB | zero recorded |
| Voice coordinator, PID 673218 | median/range 0.00 | 29.67 MiB | zero | zero recorded |
| Warm native worker, PID 673335 | median/range 0.00 | 92.85 MiB | 698.70 MiB | zero recorded |

Evidence: `/tmp/dojo-optimization/idle-mmap-verified.json`. All six endpoints
and five deltas contain the target processes, with no recorded read errors.
CPU ticks have 100 Hz resolution. A zero reading does not mean zero cost.
Context switches are not hardware wakeups. GPU memory can be shared and must
not be summed across clients. The advisory lock has one idle `cat` helper,
PID 673330, measured separately at 1.19 MiB PSS and zero accumulated CPU ticks.

The earlier audit observed 38.23% of one core in the debug shell. The final shell
is release optimized, and the runs are not interleaved. Current idle behavior
is established; no overall percentage speedup or separation of compiler and
event-delivery effects is claimed. AC power prevents a discharge-power or
battery-runtime claim.

## Allocator comparison

A separate configuration comparison used the same optimized binary and
5120 by 2160 JPEG in a private nested compositor and session bus. Five
alternating runs per setting performed five actual preview opens and closes
per run. Default adaptive allocation retained a median 137.591 MiB above its
pre-picker PSS, range 137.498 to 137.811. Fixing the existing mmap threshold at
128 KiB retained 5.234 MiB, range 5.033 to 5.641.

All 50 cycles reread the source JPEG, and all ten preview screenshots were
byte-identical. There were zero run, preview, or close errors. Dialog mapping
latency medians were 23.66 and 23.59 ms with overlapping ranges and 20 ms
polling resolution. This shows no measurable mapping-latency difference;
it does not measure completed thumbnail decoding latency.

The default runs retained four roughly 33 MiB dirty arena mappings; the fixed
runs did not. The installed bootstrap applies the setting only to the shell
and preserves an explicitly supplied caller threshold. The [GNU allocator
manual](https://sourceware.org/glibc/manual/latest/html_node/Memory-Allocation-Tunables.html)
documents this setting. Raw comparisons are in
`/tmp/dojo-optimization/dojo-picker-allocator-ympilg7e/result.json`;
[probe-picker-allocator.py](probe-picker-allocator.py) repeats the experiment.

## Verification

- Root suite: 101 passed, one existing systemd launch test ignored.
- Native speech suite: 43 passed. Actual capture produced two transcript events
  from one model load and allowed capture after cancellation.
- Voice suite: 17 passed, including real locks and split-byte Unicode sockets.
- Niri agent input: 19 passed, including three-output motion and gap scheduling.
- Real WGPU test: one passed, checking pixels, lazy scratch allocation, resizing,
  and resource invalidation.
- Private D-Bus provider: registration, icon/title/status changes, menu updates,
  removal, shutdown, and zero idle property reads passed.
- Tray GPU probe: zero allocation growth over 300 cache frames and 300 churn
  frames, with a deliberately recreating control that grows as expected.
- Release fullscreen: all eight edge crops unchanged, frame restored, and
  actual pointer delivery through the frame passed. MCP/GPUI input passed.
- Live settled bubble: visible screenshot, zero CPU ticks and GPU execution
  over 5.002 seconds, then normal expiry at 30 seconds.
- Live picker with final allocator: five actual opens/closes, correct preview
  visually inspected, and 32 KiB maximum closed GPU growth without accumulation.
  Preview replacement within one open picker and output-scale changes remain
  outside the live checks. Pure preview tests cover dimensions and decoding.
- Live activation: one shell, frame and four reservations, current wallpaper,
  original widths and scrolling positions preserved. Restart reproduced the
  pre-existing 12-pixel offscreen-column clamp; the saved width was restored.

A final counter check exposed fixture contamination: nested MCP inherited the
host runtime and sent its Working indicator to the host. The fixture now has
one private runtime with an absolute child Wayland socket. Its rerun passed,
and the host idle repaint assertion recorded zero GPU execution. This was a
verification defect, not a picker graphics failure.

Raw logs, screenshots, source manifest, and source overlay archive remain in
`/tmp/dojo-optimization/`. Exact bounds and compatibility timers are in
[resource-contract.md](resource-contract.md). Warm model residency, retained
conversation metadata, and deferred native in-flight cancellation remain
explicit costs. Display and suspend policies were not changed.
