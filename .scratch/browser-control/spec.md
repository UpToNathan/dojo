# Browser Control integration

Replace Dojo's CUA-specific browser launcher, observation and experimental
navigation adapters with anomalyco/browser-control at
`b4e4238742fa73daabfb664f1108113e9577849d` (`0.8.4`). Reuse its Playwright
engine, extension, target registry, MCP schemas, snapshots and agent workflow.

Pi remains the task, provider and canonical history owner. Browser Control owns
browser sessions and tabs. Native desktop capture/input, accessibility,
verification and CUA perception remain independent capabilities. Remove the
superseded browser MCP routes and callers; do not publish two browser engines.

## Required fixes

- Bind every browser session to its agent and monotonically increasing execution
  generation. Cancellation retires that exact generation outside its execution
  semaphore. Old work cannot become work of a resumed generation.
- Preserve existing tabs and newly created task tabs at task completion. Release
  input authority without resetting/deleting the browser session or its tabs.
- Route extension command writes through Niri's input actor. Human browser-client
  activity refuses queued sends. Already written commands are in flight; neither
  transport retirement nor a successful Chrome response proves task success.
- No query followed by an unguarded send, automatic effect retry, silent fallback
  or browser focus theft. Native human input never waits for browser I/O.
- Each agent owns its Browser Control session. One agent's stop cannot cancel
  another agent's session. Upstream target registry remains the tab owner.

## Configuration and provenance

One reproducible package serves devenv and Home Manager. Pin source, dependency
lock, package manager, native broker, local patches and generated artifacts.
Configure maintained OpenCode MCP/skills, preserving Codex/other-agent merges,
and an explicit Pi runtime extension. Keep the existing browser profile and
extensions. Install the Browser Control extension through a tested Helium launch
flag and a stable Home Manager path, or report a required browser UI step.

## Verification and activation

Prove cancellation before/after admission, queued and late sends, retries,
disconnect, generation isolation, browser-client takeover, adopted/new tab
preservation, and focus isolation. Run real browser acceptance on inactive
`dojo-tests`, never the active workspace or authenticated user profile. Build
and evaluate the exact Nix package/Home Manager generation before deployment.
User authorization includes configuration installation; disruptive browser or
desktop-session restart is the final activation step requiring its own approval.

Design comparison: [native](candidate-native.md), [session](candidate-session.md),
[judgment](design-judgment.md), [transport contract](transport-contract.md).
