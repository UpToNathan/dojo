# Local task corpus

Repeatable desktop tasks that measure whether the assistant completes real work. Tasks live in
`tools/speech/task-corpus.json`; scoring is in `tools/speech/task-corpus.mjs`.

## Running

From the live Niri session, inside devenv:

```sh
devenv shell -- node tools/speech/run-task-corpus.mjs --list
devenv shell -- node tools/speech/run-task-corpus.mjs --agent scripted
devenv shell -- node tools/speech/run-task-corpus.mjs --agent dojo --only gtk-press-save
```

The runner starts a nested Niri on an inactive empty workspace, launches the fixture, runs the task and
asserts the host's active workspace is unchanged. It uses a private accessibility bus. Results are written to
`<evidence>/results.json` (path printed at start), including recorded events and, for the dojo agent, a bounded
transcript. Defaults: installed Niri build, the agent named by that installation's `environment.sh`, and the
first immutable GTK4 library that loads. Override with `NIRI_BINARY`, `DOJO_SEMANTIC_AGENT_EXE`,
`DOJO_TEST_GTK_LIBRARY` and `DOJO_TEST_ELECTRON_BINARY`.

- `--agent scripted` makes fixed tool calls with no model. It validates fixtures, the oracle and the pipeline.
- `--agent dojo` is the real agent: the same PiRuntime (pi-durable) and dojo-agent MCP as the voice service,
  on a fresh in-memory conversation. It spends the chosen provider's quota. Provider failures (for example a
  usage limit) are reported as `agent_error` and excluded from the pass rate.

## Scoring

The runner, not the agent, decides success: after the agent stops it calls `desktop_verify` itself (or reads the
Electron fixture's own log for `vision_only` tasks). A delivered action never counts as success. The agent's own
use of `desktop_verify` is reported separately as `agent_verified`. Outcomes are `passed`, `failed`, `unknown`,
`over_budget`, `skipped`, `agent_error`, and `error` for runner failures.

## Coverage

| Tier | Tasks | Runs |
| --- | --- | --- |
| `native_semantic` | five GTK tasks, two same-titled windows | yes |
| `vision_only` | click the stock Electron seat fixture by pixels | yes (the scripted baseline clicks the image centre) |
| `shell`, `browser` | open the browser, open a tab | skipped: they need the host Helium profile, which the isolated workspace deliberately excludes |

## Measured results

- `--agent scripted` (no model): 5 of 5 GTK tasks and the Electron task pass.
- `--agent dojo` with the live selection (`opencode-go` / `glm-5.3-flash`), one run per task, 2026-10-15/16:
  6 of 6 pass, no provider errors, all within the action budget. Five GTK tasks used one to three semantic actions
  and four of them called `desktop_verify` (agent_verified). The Electron task used capture-bound pixel input and
  passed with 4 actions and 2 tool errors. Single runs of a small, cheap model: treat them as a smoke baseline, not a
  pass rate.

## Findings that changed the agent

- **Stringified arguments.** Before the fix the model sent object arguments as JSON strings
  (`"action": "{\"kind\":\"invoke\"}"`). Tool validation rejected them (`action: must be object`), so the agent
  fell back to unverified low-level input; in that run no task finished within budget. `desktop-extension.mjs` now repairs
  such arguments before validation (`repairStructuredArguments`), only where the schema permits an object or array
  and not a string. After the fix, validation failures dropped to one across the final run.
- **Verification.** With working arguments the agent verifies through `desktop_verify` for exact-window targets
  (4 of 5 GTK tasks). The earlier "never verifies" trace was largely these validation failures plus visual-only
  windows, not disobedience. `assistant.md` states the check by window type and shows a correct predicate shape,
  after one run produced malformed predicates.
- **Provider drops.** `Connection error.` appeared intermittently (2 of 6 tasks in one run) and the runtime has
  retries disabled. The runner retries once (`--agent-retries`) and reports persistent failures as `agent_error`.

## Perception worker and refusal diagnostics

- **OCR is staged.** `scripts/stage-perception-installation.mjs` produced the immutable installation
  `/nix/store/4fmkvn1rwjh7g9gv02gp6sxpqdm8y2xs-dojo-perception-installation.json` from the verified CUA 0.2.1 archive.
  `scripts/install-niri-fork.sh` now pins it (`DOJO_PERCEPTION_INSTALLATION`, which must be a root-owned immutable
  store file) into the installation's `environment.sh`, hashes it into the generation and roots it. The runner reads it
  from there. The scripted vision baseline now calls `parse_visual_regions` through the MCP on the same capture it
  clicks and requires the button label: it read "Click test" from the real Electron window.
- **Installer fix.** The installer used `rg` to find store paths to pin. `rg` is not installed here, so GC roots were
  silently skipped. It now uses `grep`.
- **Capture refusals now say why.** `capture window identity or rendered geometry changed` is followed by the expected
  and actual pid and rect (`vendor/niri/src/agent_input.rs`, two unit tests). The compositor change is staged for the
  next login; the running compositor still emits the old message. The cause of the earlier refusals is still not
  established. Check the new message when one recurs: a real move, a different pid, or sub-pixel drift at the 1.75
  output scale each read differently.
