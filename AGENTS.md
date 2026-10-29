# Hokum Agent Guide

## Speed

- Move fast and fix forward: merge once it compiles; wait only for checks
  that gate the merge.
- No fixed sleeps to wait on CI: use `gh run watch <id> --exit-status`.
- Never end your turn while still waiting on something.
- Dev releases never cancel each other: a running release always finishes,
  and pushes in between are skipped so only the newest pending run goes next
  (`cancel-in-progress: false` on Release's per-channel group). Never add
  `cancel-in-progress: true` or a cancel step to a build or publish workflow.
