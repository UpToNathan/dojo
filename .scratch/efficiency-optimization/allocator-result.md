# Fixed mmap threshold releases picker decode memory

The fixed allocator configuration retains substantially less PSS after actual picker use. Five interleaved runs per side each opened and closed the picker five times. Retained growth after the fifth close has median 137.591 MiB with the inherited default allocator, versus 5.234 MiB with `glibc.malloc.mmap_threshold=131072`. The difference is 132.357 MiB for this workload.

| Metric | Inherited/default | Fixed threshold |
| --- | --- | --- |
| Post-close PSS growth, median | 137.591 MiB | 5.234 MiB |
| Post-close PSS growth, range | 137.498 to 137.811 MiB | 5.033 to 5.641 MiB |
| Fifth-close minus first-close PSS, median | 132.734 MiB | 1.188 MiB |
| Mapping check, median | 23.66 ms | 23.59 ms |
| Mapping check, range | 23.30 to 24.56 ms | 23.07 to 24.65 ms |

Mapping checks have 20 ms polling resolution and include the control round trip. The ranges overlap. They provide a guard against a large mapping regression, without establishing a latency speedup. The 1.2-second preview wait and 0.5-second close wait are excluded from those latency values.

The relevant limit is allocator retention of freed large buffers in worker arenas. Every default run ends with four dirty anonymous mappings aligned to 64 MiB boundaries, each roughly 33 MiB. No fixed-threshold run retains those mappings. Their combined size accounts for the measured gap. The JPEG is 5120 by 2160 with three components, so its native RGB buffer is 31.64 MiB before allocator padding and other worker allocations. This result concerns freed allocation capacity, rather than a live picker image reference.

Glibc dynamically adjusts its default mmap threshold. Explicitly setting the threshold makes it static and places sufficiently large allocations outside the normal heap, allowing their storage to return on free. [GNU C Library memory allocation tunables](https://sourceware.org/glibc/manual/latest/html_node/Memory-Allocation-Tunables.html).

The measured configuration change is appropriate to scope to the shell launcher. This test does not establish a benefit for voice models, unrelated applications, battery discharge, startup latency, or active rendering throughput.

## Real work and isolation

The runner is [probe-picker-allocator.py](probe-picker-allocator.py). Run it from the repository with `devenv shell -- python -B .scratch/efficiency-optimization/probe-picker-allocator.py`.

One owned release Niri process hosted all ten runs. Each run used a fresh release shell, a private runtime/state/config/cache directory, and an absolute path to the child Wayland socket. The private session bus prevented host tray or activity-state contamination. No Apply, Derive, output scaling, host service restart, or installed configuration change occurred. The inherited allocator side was unchanged. The fixed side appended only the named threshold to any existing GLIBC_TUNABLES.

The copied shell executable was identical for all runs. Its SHA-256 was `b06241421be972fcb40be5457a64d41e4766ff2d84e1ff5a57697f29ab3d9a36`. The release Niri SHA-256 was `4677bd2c0bdd6feac9ce0cb77ff125f8747782ff19766974ff099cda4902b46f`. The astro JPEG SHA-256 was `bc3c764354b68c6b2b5086ffedfdca0a0f1da41affe3cef0406b1b62ea69e741`. Kernel boot ID, PID/start-tick identities, executable paths, output configuration, inherited allocator variables, and load averages are in the metadata.

All 50 cycles confirmed dialog mapping and disappearance, and each reread at least the 357227-byte source JPEG. One preview screenshot per run confirmed the actual astronaut image. All ten screenshots have identical SHA-256 `992b05c167a20a79ae769c5710ca1e9a4bdc5210904ae84c4965957c37e54865`. The first screenshot was also visually inspected.

There were zero measured-run, preview, or close failures. Before the measured sequence, one setup attempt failed because the system dbus-daemon defaulted to a missing session configuration. The runner now creates its own private configuration. Niri logged ten expected notification-service warnings while writing screenshots on the private bus. Screenshot creation succeeded. All owned shell processes and the owned compositor were confirmed absent after cleanup.

## Evidence and limits

The complete result is [result.json](/tmp/dojo-optimization/dojo-picker-allocator-ympilg7e/result.json). Its directory contains metadata, per-run results, shell/compositor/bus logs, preview PNGs, and raw smaps before use and at each open/close. The first failed setup log is under `/tmp/dojo-optimization/dojo-picker-allocator-v96x2vdo`.

The fixed-side retention assertions were declared before measurement: no more than 64 MiB growth over baseline and no more than 16 MiB drift after the first close. Every fixed run passed both. The five default values and five fixed values are separated by far more than their own variation. This is an end-to-end process-memory result for the real UI, not a decoder microbenchmark.

The fixture begins without a selected wallpaper generation, so its absolute baseline PSS differs from the resident host shell. Both sides load the same repository astro image and display the same UI. The reported result uses growth over each fresh shell's own pre-picker baseline. The physical memory counter is PSS from smaps_rollup. GPU client memory is not added to it. No peak-memory or battery saving is inferred from these retained-memory values.
