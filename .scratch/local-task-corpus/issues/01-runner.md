# Task runner for the local corpus

Type: task
Status: resolved

## Brief

Add a runner that starts each corpus fixture in the isolated nested Niri
workspace, submits the goal to the voice agent, exports the trace and scores it
with `scoreRun`. Resolve pid and window id into predicates at run time.

## Acceptance

- Runs on inactive `dojo-tests`; the host's active workspace is unchanged.
- Writes per-task outcomes and `summarize()` totals.
- Needs fixtures for `qt6` (volume slider) and `electron-stock`; the GTK fixture
  exists.

## Answer

Built as `tools/speech/run-task-corpus.mjs`; usage and scoring are in [docs/task-corpus.md](../../../docs/task-corpus.md).
The scripted baseline passes 5 of 5 GTK tasks and the Electron vision task, with the host workspace unchanged.
The browser and shell tasks are skipped on purpose (host profile). The `--agent dojo` path runs the live selection (`opencode-go` / `glm-5.3-flash`) and passes 6 of 6 runnable tasks in a
clean run; see the measured results and findings in the doc.
