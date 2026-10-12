# Final review: implementation and evidence

The earlier review is superseded. I found no concrete correctness defect in the final source or activation evidence. The optimized shell `b06241421be972fc` is active as PID 678716, the voice service is active with one warm worker, and release Niri `4677bd2c0bdd6fea` is selected for the next login while the current compositor remains unchanged. Frame mapping, four reservations, restored widths, fullscreen edge crops, actual pointer passthrough, and the live static-bubble expiry have retained artifacts.

The final test count checks out: 101 root tests (one existing ignored), 43 native speech tests, 17 voice tests, 19 Niri input tests, and one GPU renderer test: 181 passed, one ignored. The corresponding raw logs pass. The five quiet samples reproduce the reported shell range and residency; the measured warm worker remains about 699 MiB of client GPU memory with zero observed idle GPU execution. Treat that residency, retained voice history, deferred native inference cancellation, and post-first-use path textures as explicit resource costs. The AC run supports current idle observations, not a battery-life or before/after speedup claim.

Three evidence/reporting gaps remain for the trail to close:

1. `outcome.md` says raw verification logs are under `/tmp/dojo-optimization/`, but the reported 300-frame tray GPU cache/replacement probe has no raw log there. Preserve its exact invocation and output, or label that result as transcript-only.
2. `graphics-result.md` still leaves live picker replacement/close atlas counters and output-scale changes pending. Carry those limits into `outcome.md`'s remaining-verification notes; the focused decode tests do not prove those UI resource lifetimes.
3. The trail's first combined D-Bus/fullscreen row points only to the nested fullscreen directory. A later row supplies `/tmp/dojo-optimization/tray-protocol.log`, so the evidence exists now; append a correction linking both artifacts. Also preserve a source revision or worktree digest with the final suite, since the tested changes are still dirty relative to the logged base HEAD.

Other previous concerns are resolved: the renderer test now passes on the actual WGPU adapter; D-Bus output is retained; the final root test log supersedes the pending row; the release shell and next-login compositor decisions are recorded separately; and the animation fixture/code discrepancy is no longer material to the outcome. No source changes or tests were made during this audit.

## Attention

reviewed by gpt-6.1-sol

- Keep the missing raw tray GPU probe and the unverified live picker scale/atlas checks visible until their evidence is saved or their status is explicitly accepted as deferred.
- Append trail corrections for the combined D-Bus/fullscreen evidence and the tested source snapshot; do not edit earlier TSV rows.
