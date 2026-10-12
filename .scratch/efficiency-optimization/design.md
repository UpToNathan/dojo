# Event and resource ownership

The chosen base is candidate B. The independent comparison in
[design-judgment.md](design-judgment.md) agrees. It retains the existing tray,
shell, voice, picker, and renderer owners rather than introducing a general
desktop runtime. Candidate A contributes immutable shared snapshots and
coalesced change publication. Its new global runtime facade is rejected because
existing owners can hide the event plumbing behind smaller local interfaces.

The implementation contract is bounded notifications, interruptible shutdown,
retained proxy identities with authoritative reads for nonstandard SNI signals,
shared host observation, filesystem invalidation instead of half-second reads,
and deadline-driven speech. Each window remains responsible for image eviction.
Path scratch allocation becomes lazy; previews are bounded thumbnails while
original bytes remain the source for Apply and Derive.

Warm transcription is retained. Both voice startup paths must acquire the same
lock before model preload or socket unlink. Streaming persistence and reconnect
must preserve current session continuity. No competing power daemon or arbitrary
suspend policy is introduced. Resource and latency measurements, not forecast
percentages, decide whether further residency tradeoffs are worthwhile.

Verification proceeds in units: tray protocol/idle test, speech lifecycle tests,
renderer and preview checks, desktop fullscreen/input/frame checks, then live
counter comparisons. Runtime evidence and append-only decisions live under
`/tmp/dojo-optimization/`. Production activation follows passing behavior checks.
