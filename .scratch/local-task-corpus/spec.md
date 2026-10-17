# Local task corpus

## Goal

Measure whether the assistant completes real desktop tasks, so perception and
model work (CUA audit steps 4 to 6) is justified by failures, not guesses.

## Requirements

- Tasks live in `tools/speech/task-corpus.json`: a goal, a fixture, a success
  predicate and an action budget. Predicates never pin a pid or window id.
- Success comes only from `desktop_verify`. A delivered action is not success;
  a run with no verification after an effect scores `unknown`.
- Scoring reads `exportDesktopTrace()` events. It never replays effects.
- Tiers: `native_semantic`, `shell`, `browser`, `vision_only`. Chromium/Electron
  is `vision_only` by decision; it has no semantic route.

## Baseline (2026-10-15)

The only live desktop trace available (one voice session, 18 calls) contains
five capture actions and no `desktop_verify` call. Two of five were refused or
errored (`capture window identity or rendered geometry changed`, and one
redacted tool error that was retried at identical coordinates). Three were
delivered and unverified. Saved Pi sessions on disk (29) contain no desktop
calls, so they provide no corpus evidence.

## Findings on the live trace

- **Verification (resolved with caveats).** The first live trace showed no `desktop_verify`, which I first read as a
  guidance gap. A real-agent run showed the cause: the model sent object arguments as JSON strings and tool
  validation rejected them, pushing it onto unverified low-level routes. Fixed in
  `tools/speech/desktop-extension.mjs` (`repairStructuredArguments`, with tests). After the fix the agent used
  `desktop_verify` in 4 of 5 GTK tasks and all 6 tasks passed the runner's oracle. `assistant.md` also gained a
  "Check the result" section. One model, one run per task.
- **Perception worker (resolved).** `DOJO_PERCEPTION_INSTALLATION` was unset, so OCR was unavailable. The worker is now
  staged and pinned by the installer; OCR through the MCP read the Electron fixture's button label.
- **Refusals.** Two capture actions were refused with `capture window identity or rendered geometry changed`
  (`vendor/niri/src/agent_input.rs`, which compares pid and the exact rendered rect with the capture's). They
  failed closed with no effect. In one the following post-observation reported an unchanged 762x962 window, so
  the cause is not established; the output scale is 1.75 and exact floating-point rect equality is the leading
  suspect. Animations are excluded by design (`capture_placements` refuses them with a different error). The
  refusal reason now includes the expected and actual pid and rect (staged for the next login), so the next
  occurrence can be diagnosed from the message alone.

## Out of scope

Browser and shell tasks do not run in the isolated workspace (they need the host browser profile).
