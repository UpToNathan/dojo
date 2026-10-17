---
status: accepted
---

# The lock screen is a separate on-demand GPUI program

The lock screen runs as its own program, `dojo-lock`, that takes the session lock through `ext-session-lock-v1`. It is not a surface inside the resident shell. The compositor keeps the session locked if the locker dies, so a stalled or crashed shell can never fail open, and a replacement locker may take over a dead lock. Dojo's UI stack is GPUI, so the program draws with GPUI rather than a separate plain renderer, which keeps the Dojo frame, palette and text rendering identical to the shell. The cost is carrying a small GPUI patch in `vendor/zed` that adds a session-lock window kind (about 250 lines; spike in `.scratch/lockscreen/spikes/`).

## Considered options

- **A surface inside the shell.** Rejected: a shell crash or UI-thread stall would leave the session unlocked or wedged, and the lock could not outlive the shell.
- **A plain Wayland client (`smithay-client-toolkit`) with a software or wgpu renderer.** Not tried, because GPUI worked. It remains the fallback if the GPUI patch becomes too costly to carry.

## Consequences

- The locker starts on demand rather than staying resident, to keep idle cost low. Startup was measured at about 0.1 s with the Vulkan driver pinned to the installed GPU and 0.4–0.6 s with every driver enumerated, so the locker must pin its driver.
- GPUI's frame loop parks when nothing changes. The lock view must therefore not animate continuously.
- The locker must exit explicitly if its Wayland connection is lost; the spike saw GPUI spin at full CPU after a protocol error.
