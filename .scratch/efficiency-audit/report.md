# Dojo efficiency and battery audit

Audit date: 2026-10-11. Baseline HEAD: `7fb1e4ffa79843314624783fe8f302c734322caf`,
plus the current working tree and activated tray-image leak fix. No production
code, running service, power setting, or desktop configuration was changed
during this audit. Three parallel source audits used the swarm workflow;
measurement review used benchmark-checklist and explain-the-number.

Dojo can become substantially quieter without losing its desktop, speech, or
agent capabilities. The first target is repeated idle tray work. The next
targets are unnecessary redraws, duplicate process ownership, and oversized or
retained graphics resources. This audit does not quantify future savings.

## Current live baseline

Five passive intervals of approximately 5.10 seconds sampled process CPU
counters, proportional memory, and unique DRM clients within each process.
No audio recording, speech request, synthetic animation, or benchmark ran
during this baseline. The existing T3 conversation and audit agents remained
active, so this is a quiet-shell observation in an active desktop, not a
whole-machine idle or energy benchmark. The shell is a development build.

CPU percentages below use one logical core as 100%; the machine has 16 logical
CPUs. Graphics percentages are client GFX engine counter deltas, not whole-GPU
power. PSS apportions shared CPU memory. GPU allocations may include shared
display buffers and must not be added across processes.

| Process | CPU median, range | PSS | GPU allocation | GFX activity median |
| --- | --- | --- | --- | --- |
| Dojo shell | 38.23%, 35.89–39.03% | 232.8 MiB | 552.1 MiB | 0.15% |
| Niri | 4.70%, 4.51–4.90% | 88.4 MiB | 528.0 MiB | 2.61% |
| wluma | 0.98%, 0.78–0.98% | 18.7 MiB | 50.3 MiB | 2.28% |
| Current speech worker | 0% in all intervals | 122.7 MiB | 699.0 MiB | 0% |
| Current voice coordinator | 0% in all intervals | 34.7 MiB | negligible | 0% |

A separate, untraced 5.00-second thread sample attributed 22.58% of a core to
the shell's `zbus::Connectio` thread, 13.79% to `dojo-status-not`, and 0.80% to
the GUI main thread. An earlier exploratory thread sample showed the same
ordering. This establishes the tray/D-Bus threads as the dominant current CPU
consumer. It does not identify individual functions inside those threads.

All six baseline snapshots reported zero read failures. Syscall tracing was
attempted but blocked by the host's ptrace policy. The policy was left intact.
No release-versus-development, renderer alternative, or battery-policy winner
was benchmarked. Establish an optimized deployment baseline before assigning
numerical gains to those alternatives.

Evidence: `/tmp/dojo-efficiency-audit/runtime-baseline.json` and
`/tmp/dojo-efficiency-audit/threads.json`. Rerunnable readers:
[sample-runtime.py](sample-runtime.py) and [profile-threads.py](profile-threads.py).

## Ranked work

### 1. Make the tray event-driven and establish a release baseline

Highest priority for CPU and likely battery impact. The measured hot threads
belong to a worker that checks channels every 25 ms and refreshes items after
750 ms. Every refresh rebuilds proxies and requests item properties, including
unchanged icon pixmaps. The two current icon payloads are approximately 250 KB
and 262 KB. The new image cache fixes GPU accumulation; it does not avoid this
repeated property retrieval and decoding.

Evidence: `src/tray.rs:498`, `:551`, `:598`, `:626`; thread counters above.
Installed Dojo and speech execute development binaries. The development
profile optimizes only selected graphics dependencies, not all tray/D-Bus code:
`home-manager.nix:25`, `tools/speech/voice.mjs:326`, `Cargo.toml:29`.

Retain item proxies, subscribe to notifier/property/name-owner changes, and
block waiting for commands or a refresh deadline. Keep a slow reconciliation
fallback for imperfect providers. Avoid serial full sweeps delaying menu input.
Package optimized resident binaries separately from development builds.
Release optimization and event-driven scheduling solve different problems;
neither has a measured savings figure here.

Validation: five interleaved samples per candidate on the same tray, then
registration/removal, status and icon changes, menus, bad providers, shutdown,
and multiple outputs. Verify bus traffic falls and displayed state still updates.

### 2. Let the desktop frame and static bubbles sleep

High-confidence source finding. Every FrameView polls host state, starts
`pgrep`, clones tray state, and notifies once per cycle even if nothing visible
changed. Host work multiplies with output count. Settled speech bubbles
self-schedule throughout their 30-second lifetime, although their entry motion
ends after 0.35 seconds and exit motion occurs only near expiry. Sent also
requests frames despite its constant indicator.

Evidence: `src/frame.rs:476`, `:750`, `:1194`; `src/speech.rs:97`, `:116`.

Share host state across outputs and notify only when displayed values change.
Animate entry, exit, and active pulses; use deadlines for the static middle
interval and expiry. Honor reduced motion. Keep normal input responsiveness.

Validation: count frame submissions through each speech phase and the static
interval. Exercise deadline extension, follow-ups, dismissal, reduced motion,
power/tray changes, and output hotplug. Check visible pixels and input behavior.

### 3. Give voice one process owner and bound integration work

Source and live finding. PID 527978 is an older `voice.mjs --serve` coordinator
with worker 528009 in an earlier T3 application scope. PID 604088 is the current
coordinator with worker 604342 in `dojo-voice.service`. The older pair accounts
for approximately 128.8 MiB PSS; its worker is inactive in this baseline. This
does not prove which historical launch created the duplication.

The startup paths provide a concrete race: systemd enters `--serve` without
the fallback's `flock`; serve preloads a worker before unlinking and binding the
shared socket. Both entries must acquire the same ownership lock before preload
or socket replacement. Prefer systemd as lifecycle owner with one safe fallback.

Evidence: `tools/speech/dojo-voice.service:9`; `tools/speech/voice.mjs:51`,
`:352`, `:408`; baseline process metadata and command-role inspection.

The coordinator also rewrites its full history on each streaming item, replaces
subscriptions on follow-ups, and runs a permanent one-second interval.
Coalesce persistence, bound retained history, reuse healthy subscriptions, and
replace polling with settlement/idle deadlines. Close an unused voice WebSocket
when conversation continuity no longer requires it.

Validation: concurrent startup, session restart, reconnect and server restart,
single socket/model owner, crash recovery, follow-up continuity, stream
sequence recovery, bounded writes, and no timer without a pending deadline.

### 4. Cut graphics resources that do not contribute pixels

High-confidence allocation/lifetime findings; savings remain unmeasured.
WGPU allocates path intermediate and MSAA textures even when a scene contains
no paths. Empty reservations and raster wallpaper still go through that path.
With a four-byte format and 4x MSAA, a 2880×1800 target requests about 98.9 MiB
of raw path attachments. This is descriptor arithmetic, not a measurement of
resident or recoverable memory on this driver.

Evidence: `vendor/zed/crates/gpui_wgpu/src/wgpu_renderer.rs:1383`, `:1436`;
`src/frame.rs:64`; `src/wallpaper.rs:15`.

Allocate path scratch textures only when paths exist. Then measure whether
the four blank reserved-edge surfaces need full renderer instances, or can use
a lightweight Wayland buffer path. Treat renderer sharing as a separate,
larger change rather than the first fix.

Wallpaper previews also use full-size decoding and the application-global
image cache. Distinct images can survive picker closure. Give previews bounded
ownership and display-sized thumbnails while retaining original captured bytes
for Apply and Derive: `src/theme_picker.rs:721`, GPUI `src/app.rs:3023`.

Validation: real GPU resources and CPU memory across empty/path/raster scenes,
repeated large previews, replacement/close, multiple outputs, resize and device
recovery. Preserve antialiasing, image fidelity, layout, and fullscreen behavior.

### 5. Measure wluma capture and speech blur before changing appearance

wluma's client used 2.28% median GFX engine activity in the passive samples,
while Niri used 2.61%. This makes capture a useful GPU/battery investigation
target, not proof that disabling it improves net battery life. wluma provides
content-aware adaptive brightness, which may save panel power. The maintained
configuration has no physical ambient-light sensor:
`home-manager.nix:227`. Its purpose is described in the
[upstream wluma documentation](https://github.com/max-baz/wluma).

Measure capture cadence, damage gating, analysis resolution, and GPU low-power
residency. Investigate throttling or smaller analysis images while preserving
brightness behavior. Compare net discharge at matched brightness, then assess
adaptive brightness's full benefit separately.

Speech sends small blur regions on a full-output surface, while the compositor
source keeps effect geometry and clipping subregions separate. Actual capture
cropping needs tracing before claiming full-screen blur work:
`vendor/niri/src/render_helpers/background_effect.rs:237`,
`framebuffer_effect.rs:206`, `:355`. If confirmed, crop with kernel padding
while retaining actual backdrop blur and rounded clipping.

### 6. Remove remaining polling and wasted active-path work

Lower priority until profiles show a material contribution:

- Theme/config/wallpaper loops read files and acquire theme locks every half
  second. Use appearance events and directory watches with a slow reconciliation
  fallback; preserve atomic publication and invalid-config behavior.
- Agent cursor motion schedules all outputs rather than affected outputs.
  Track old/new output damage. A single-output laptop gains little from this
  particular correction.
- Launcher resolves icons for every result before displaying ten rows. Resolve
  visible rows and cache with bounded lifetime. Matching already coalesces work.
- Screenshots decode full PNG pixels only to obtain dimensions. Parse dimensions
  with a validated image header reader and retain original output bytes.
- Speech copies complete audio clips during resampling and inference setup.
  Move ownership or borrow where the native lifetime permits it. Actual benefit
  must be measured at real utterance sizes.
- Cancel cannot interrupt inference while the worker synchronously handles
  commands. Verify the native model's abort support and use a separate command
  reader with a per-request cancellation token.
- Older T3 backend entrypoints remain in earlier application scopes. Their
  measured PSS totals about 346.5 MiB. Determine connected clients and ownership
  before retiring any instance. Twelve `t3code` processes do not mean twelve
  backends; Electron legitimately uses multiple processes.

Exact source locations and verification cases are in [shell.md](shell.md),
[speech.md](speech.md), and [compositor.md](compositor.md).

## Battery policy and capability constraints

The effective host settings are already power-saver, `powersave` governor, and
`power` energy preference. The panel runs 2880×1800 at 60 Hz, scale 1.75. VRR
is supported but disabled. Neither enabling VRR nor lowering display fidelity
has an established benefit on this panel.

An additional high-priority battery finding is confirmed at runtime:
`stasis info` reports `Plan: (no enabled steps)`, and `stasis list actions`
prints an empty effective action plan. It has no configured screen-off or
suspend action, despite remaining resident and polling for activity. The
loaded `~/.config/stasis/stasis.rune` contains integration/inhibition options
only. This does not establish that lid closure or a host service can never
suspend the machine; it establishes that the configured desktop idle manager
performs no timed energy-saving actions.

Define battery and AC idle plans with appropriate lock, screen-off, and suspend
deadlines. Preserve active-work and media/session inhibition and require
lock-before-suspend. Verify actual display power-off, resume, lid handling,
remote/active agent work, and inhibitors. Stasis also reports its gamepad/game
monitors enabled with zero detected entries; if those capabilities are not
used, inspect their wakeup contribution before leaving them enabled by default.
No idle policy was changed during this audit.

The battery was charging and AC was online. Its `power_now` cannot be treated
as desktop discharge or converted into a battery-life improvement. An energy
comparison needs unplugged runs, fixed brightness/refresh/content, the same
power profile, matched workloads and temperatures, controlled warmup, at least
five alternating runs per candidate, and an equal correctness check. Record
energy consumed, wakeups or residency, GPU activity, and user-visible latency.

The warmed speech model occupied about 699 MiB GPU memory but showed zero CPU
and GFX work in these intervals. Memory residency alone does not establish
energy cost. Its preload and session retention are explicit requirements in
`.scratch/speech/spec.md:32`. Preserve fast push-to-talk first. Consider adaptive
idle eviction or CPU placement only after measuring GPU power residency and
cold/warm transcription latency. Keep the microphone closed outside recording.

Do not sacrifice working video acceleration, independent agent input, required
desktop protocols, clipboard persistence/history, secrets support, or natural
speech feedback to reduce a process count. Socket/D-Bus activation, the bounded
launcher worker, on-demand agent service, reused GPU textures, and the new tray
image cache are existing good choices.

## Proposed acceptance criteria

- Static shell content produces no self-scheduled full-output redraws.
- Empty or unchanged trays block on events rather than checking every 25 ms.
- Shared machine-state work occurs once, then updates only affected views.
- One voice coordinator owns one warmed transcription worker.
- GPU and preview memory are bounded by current assets and outputs, not history.
- Optional resources have explicit lifetimes and deadline-driven idle behavior.
- The idle manager has a deliberate, verified energy-saving action plan.
- Battery improvements retain measured input, launcher, and speech latency.

Start with release baselines and tray events, then quiet rendering and voice
ownership, then lazy graphics/preview resources. Profile capture/blur and test
battery policies after those foundations are quiet. This order follows observed
cost and prevents an architectural rewrite from replacing smaller verified gains.
