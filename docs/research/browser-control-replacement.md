# Browser Control as Dojo's browser driver

Inspected 2026-10-13 at commit `b4e4238742fa73daabfb664f1108113e9577849d`. This is a source assessment. No extension was installed, browser was controlled, or implementation changed. The checkout is `/tmp/dojo-browser-control-research-b4e4238`.

## Recommendation

Browser Control fits the user's existing-profile browser goal. Its extension and relay already implement Playwright control of user-attached Chromium tabs, semantic snapshots, screenshots and human handoffs. Reusing the package avoids rewriting those operations. However, an unchanged MCP installation does not provide Dojo's native human takeover or cancellation guarantees. Replace the CUA browser implementation with this package as the sole browser engine after resolving those lifecycle gaps. Retain the CUA native Driver and shared external-action broker. The user's "maybe" calls for this assessment; it does not authorize deleting the existing implementation yet. [Architecture](https://github.com/anomalyco/browser-control/blob/b4e4238742fa73daabfb664f1108113e9577849d/README.md), [execution lifecycle](https://github.com/anomalyco/browser-control/blob/b4e4238742fa73daabfb664f1108113e9577849d/src/session-manager.ts#L414).

This replacement retains the CUA native desktop Driver. Browser Control uses Chrome's debugger API and CDP; it does not execute through Dojo's Niri agent seat. Its in-page ghost cursor is separate from Dojo's compositor cursor. [Extension command forwarding](https://github.com/anomalyco/browser-control/blob/b4e4238742fa73daabfb664f1108113e9577849d/extension/src/background.ts#L506), [ghost cursor dispatch](https://github.com/anomalyco/browser-control/blob/b4e4238742fa73daabfb664f1108113e9577849d/src/relay.ts#L1595).

## Immutable inputs

| Input | Inspected pin |
| --- | --- |
| Repository | `b4e4238742fa73daabfb664f1108113e9577849d` |
| Package | `@opencode-ai/browser-control` `0.8.4`, MIT |
| Package manager | `pnpm@11.20.0` |
| Node runtime requirement | `>=22.19.0`, a requirement rather than an exact runtime pin |
| Extension manifest | `0.0.25`, Chrome minimum `120` |
| Runtime dependency lock resolutions | `@effect/platform-node` `4.0.1`, `@effect/platform-node-shared` `4.0.1`, `effect` `4.0.1`, `acorn` `8.18.0`, `pixelmatch` `7.2.0`, `playwright-core` `1.63.0`, `pngjs` `7.0.0`, `ws` `8.22.0` |
| Lockfile SHA-256 | `453160ba7b8c8979eb7c772467b130374973ecc02a229c74ff65012fca8387d9` |
| Package manifest SHA-256 | `7a6909c12388aac77679366da607682478d76c720ec45d7fab39fb8d514b7a81` |
| Extension manifest SHA-256 | `b178d4f2ccc6ca9af6391ff20a9ac8aacae799285a3716a8996d6581fff739b5` |

The package manifest uses ranges for several dependencies. The committed lockfile resolves the complete transitive graph and integrity hashes. Any Dojo build should retain that lockfile, freeze dependency installation, pin its Node runtime separately, and hash generated CLI/extension artifacts. Building from this commit is more precise than installing the moving repository URL. Build scripts include a timestamp build ID, so commit identity alone does not identify binary output. These are proposed integration requirements, not completed deployment work. [Package](https://github.com/anomalyco/browser-control/blob/b4e4238742fa73daabfb664f1108113e9577849d/package.json), [lockfile](https://github.com/anomalyco/browser-control/blob/b4e4238742fa73daabfb664f1108113e9577849d/pnpm-lock.yaml), [build script](https://github.com/anomalyco/browser-control/blob/b4e4238742fa73daabfb664f1108113e9577849d/scripts/build-cli.ts).

## What works unchanged

- Existing Chromium-family profile access comes through the extension. The user attaches an existing tab with the toolbar button, then the client adopts it into a session. Adoption is exclusive. Reset/delete releases adopted user tabs without closing them. Relay-created tabs have separate session ownership. [Target ownership](https://github.com/anomalyco/browser-control/blob/b4e4238742fa73daabfb664f1108113e9577849d/src/target-registry.ts), [session lifecycle](https://github.com/anomalyco/browser-control/blob/b4e4238742fa73daabfb664f1108113e9577849d/src/session-manager.ts#L291).
- CLI and stdio MCP share a detached relay at `127.0.0.1:19989`. Operational calls start it when needed. Status and discovery do not require starting it; terminating an MCP client does not terminate the shared relay. This is browser-driver state, not an LLM scheduler. [Relay lifecycle](https://github.com/anomalyco/browser-control/blob/b4e4238742fa73daabfb664f1108113e9577849d/src/relay-lifecycle.ts), [MCP registration](https://github.com/anomalyco/browser-control/blob/b4e4238742fa73daabfb664f1108113e9577849d/src/mcp.ts#L499), [loopback defaults](https://github.com/anomalyco/browser-control/blob/b4e4238742fa73daabfb664f1108113e9577849d/src/relay-helpers.ts#L11).
- The main MCP operation is `execute(code)`. Code receives Playwright browser/context/page plus persistent session `state`, snapshots/ref locators, ARIA snapshots, screenshot helpers, handoff/demonstration and network capture. MCP also offers session operations, network capture, recording, flight recorder, secrets operations and the bundled skill. Dojo can select the tools it actually needs rather than expose all of them. [Tool definitions](https://github.com/anomalyco/browser-control/blob/b4e4238742fa73daabfb664f1108113e9577849d/src/mcp.ts#L55), [execution globals](https://github.com/anomalyco/browser-control/blob/b4e4238742fa73daabfb664f1108113e9577849d/src/execute.ts#L2129).
- Existing-tab approval, browser status controls and human handoffs belong to the extension. `requestTab` can prompt for access to an unattached tab; `handoff` waits on the exact target's completion control. These browser controls can be reused if their UI matches the intended Dojo experience. [Tab request](https://github.com/anomalyco/browser-control/blob/b4e4238742fa73daabfb664f1108113e9577849d/extension/src/background.ts#L963), [handoff](https://github.com/anomalyco/browser-control/blob/b4e4238742fa73daabfb664f1108113e9577849d/src/handoff.ts).

## Compatibility gaps

### Human control and focus

Browser Control does not map browser target IDs to Niri windows or consult Niri human-ownership epochs. Its human-input module generates agent pointer trajectories and timing; it is not a native-human takeover detector. The content script handles explicit approval/handoff controls. Source inspection found no ordinary human pointer event that suspends all agent execution. This is a compatibility finding, not a live behavior test. [Input module](https://github.com/anomalyco/browser-control/blob/b4e4238742fa73daabfb664f1108113e9577849d/src/human-input.ts), [content script](https://github.com/anomalyco/browser-control/blob/b4e4238742fa73daabfb664f1108113e9577849d/extension/src/content-script.ts#L125).

The tab-request flow explicitly activates the matched tab and asks Chrome to focus its window before displaying approval. CDP recording also activates its target. Dojo must decide when such focus changes are acceptable, including background work and tests on inactive workspaces. Browser-native tab ownership cannot substitute for compositor focus ownership. [Request focus](https://github.com/anomalyco/browser-control/blob/b4e4238742fa73daabfb664f1108113e9577849d/extension/src/background.ts#L1009), [recording](https://github.com/anomalyco/browser-control/blob/b4e4238742fa73daabfb664f1108113e9577849d/src/recording-relay.ts).

### Cancellation

HTTP disconnect aborts the request Effect, but a started session execution runs in a detached worker. The interruption handler cancels only work that has not started. There is no general `execute_cancel` MCP tool. Session reset/delete waits for the execution semaphore and can time out after 10 seconds; it is not an emergency stop. Killing Pi's MCP client therefore cannot be presented as proof that browser effects stopped. [HTTP close handler](https://github.com/anomalyco/browser-control/blob/b4e4238742fa73daabfb664f1108113e9577849d/src/http-api.ts#L276), [started execution](https://github.com/anomalyco/browser-control/blob/b4e4238742fa73daabfb664f1108113e9577849d/src/session-manager.ts#L414), [detached worker and lifecycle permit](https://github.com/anomalyco/browser-control/blob/b4e4238742fa73daabfb664f1108113e9577849d/src/session-manager.ts#L982).

Dojo needs a browser-specific stop mechanism before promising its current takeover semantics. Candidate approaches need investigation: upstream cancellation support, a small pinned fork that retires exact session transport and refuses further effects, or a clearly narrower trusted-code contract. A wrapper that checks Niri only before `execute` cannot interrupt mutations inside a long script.

### Trusted code and read-only mode

The relay executes code with Node's `AsyncFunction`, not an isolated security sandbox. Browser code can access logged-in page state and Node execution capabilities. Read-only sessions block CDP `Input.*`; `page.evaluate()` can still mutate page state. The always-blocked CDP methods cover browser-close and broad cookie/cache clearing, not all mutations. This trusted-code design is compatible with a general assistant that already has shell authority. The concrete labeling requirement is narrower: do not present this mode as equivalent to Dojo's existing read-only DOM observations. [Execution](https://github.com/anomalyco/browser-control/blob/b4e4238742fa73daabfb664f1108113e9577849d/src/execute.ts#L2112), [CDP guardrails](https://github.com/anomalyco/browser-control/blob/b4e4238742fa73daabfb664f1108113e9577849d/src/cdp-guardrails.ts).

The relay uses loopback binding, Host/fetch-site checks and extension-origin allowlisting. It does not authenticate remote callers and deliberately trusts local agent code. Extension permissions include `activeTab`, `alarms`, `debugger`, `offscreen`, `tabCapture` and `tabGroups`, plus content scripts on all URLs and frames. Installation remains an explicit browser-extension step. [HTTP boundary](https://github.com/anomalyco/browser-control/blob/b4e4238742fa73daabfb664f1108113e9577849d/src/http-api.ts#L69), [WebSocket boundary](https://github.com/anomalyco/browser-control/blob/b4e4238742fa73daabfb664f1108113e9577849d/src/relay.ts#L770), [manifest](https://github.com/anomalyco/browser-control/blob/b4e4238742fa73daabfb664f1108113e9577849d/extension/manifest.json).

### Pi ownership and persistence

Pi can invoke the existing stdio MCP adapter, with stable browser session IDs associated with Pi task identity. No extra planner/provider layer is needed. The upstream relay nevertheless persists session descriptors under `~/.browser-control/relays/<port>/sessions.json` and per-session execution journals. Those contain browser ownership and diagnostics, not approval records. Dojo should decide retention and cleanup rather than silently add a second durable task history. JavaScript session state resets after relay restart. [Catalog](https://github.com/anomalyco/browser-control/blob/b4e4238742fa73daabfb664f1108113e9577849d/src/session-catalog.ts), [journal](https://github.com/anomalyco/browser-control/blob/b4e4238742fa73daabfb664f1108113e9577849d/src/session-journal.ts), [MCP session identity](https://github.com/anomalyco/browser-control/blob/b4e4238742fa73daabfb664f1108113e9577849d/src/mcp.ts#L499).

## Replacement boundary

Remove Dojo's old browser launcher, browser DOM adapter, experimental mutation path and corresponding MCP routes once the replacement is integrated. Keep native desktop capture/input and general external-action admission infrastructure, which also applies to AT-SPI. The CUA dependency alias also supplies image perception, so deleting browser adapters does not justify dropping all copied CUA core code. The parent task is checking those local deletion boundaries separately.

The first useful acceptance case is an explicitly attached existing Helium tab: observe through `snapshot`, interact through Playwright, preserve that tab at task end, then demonstrate cancellation and human takeover. Until those cases pass, this is a suitable engine candidate rather than a completed replacement.
