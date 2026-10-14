# Embedded CUA observation runtime

The complete pinned CUA core and contract compile in this standalone Rust workspace with default features disabled. Explicit owners now run the retained snapshot store and a coherent read-only BrowserEngine. Pi remains the scheduler. No CUA ToolRegistry, daemon, replay loop, or autonomous agent is instantiated.

`EmbeddedEpisode` retains snapshot payloads under exact episode, compositor, native client, window, PID, and process-start identity. Its mutex orders publication, cancellation, and teardown. Pending publication cannot survive close, cancellation, or owner drop. A replacement observation invalidates previous refs. Original sparse element indices remain unchanged. Mutation always returns `MutationAdmissionUnproved`.

`BrowserReadEpisode` directly runs the complete imported BrowserEngine for native-window binding, selected-tab identification, screenshots, composed DOM snapshots, and retained frame-bound refs. A unique episode context owns its connection pool, store, liveness, and cancellation. Construction skips CUA's global cleanup hooks. Every observation rebinds native-window and selected-tab evidence. Screenshot and DOM snapshot acquisitions are sequential, so `image_semantic_atomic` is false.

## Rust integration

Use `cua_driver_core::browser_observation` for the browser owner and safe observation/ref types. The engine, CDP pool/connections, registry, and mutable context remain private. `close_handle()` gives the service a cancellation handle while an observation future borrows its session. Closing the episode rejects further sends, cancels pending replies, clears refs, and closes its connections.

The trusted host implements `host::BrowserPlatform` for OS proofs and `ExactWindowSource` for current compositor/client/process generations. `BrowserReadEpisode::new(platform, identities, window_id)` accepts those distinct adapters, and it creates its own private episode identity. Caller tool JSON cannot supply that identity.

`browser_observation::niri` supplies the production Linux adapter. `BrowserConfig::new(endpoint)` accepts an operator-configured literal loopback WebSocket URL. `NiriWindowSource::describe` supplies the selected Niri window's exact identity, compositor PID/start, title, CDP-normalized bounds, geometry proof, and optional exact window cardinality. `NiriBrowserPlatform` checks live Linux process generations and implements both browser proof and exact-identity interfaces. A service can supply a separate cheap live identity source while reusing one capture-correlated scene descriptor during an observation.

Endpoint discovery requires a matching `--remote-debugging-port` argument and a loopback listening socket inode held by the exact browser root PID. Process fingerprints must remain identical before and after attestation. The adapter does not probe guessed ports, follow a child-process listener without proof, enable debugging, or launch a browser. Missing configuration returns no endpoint. Explicit configuration alone does not grant socket ownership or permission to inspect a consumer profile.

The adapter preserves CUA classification and trusted installation checks. Normal Chromium remains a standalone consumer and is grant-refused. Electron can use the embedded-application route. A service-owned isolated Chromium root requires a separate trusted lifecycle proof before driver-owned access is enabled. The tests establish the mock engine and Linux proof boundaries, not real Chromium observation parity.

## Read-only effect gate

CDP dispatch checks explicit episode liveness and current native/process/endpoint identity, then applies a strict command and parameter allowlist. Every poll that writes to the WebSocket holds the same liveness mutex used by close. Closing the episode stops new polls and pending replies. Identity and observation deadlines bound host calls and transport work.

The allowlist permits bounded DOM, accessibility, layout, screenshot, and attachment reads. `Runtime.evaluate` accepts only the exact imported visibility query and exact parameters. `Runtime.callFunctionOn`, arbitrary JavaScript, unknown methods, navigation, typing, dialogs, download configuration, and file input changes are refused. Preparation and every imported mutation tool also refuse before argument parsing, consent, paths, or platform calls. Read-only pools refuse existing-profile claims and reconnects. No configuration switch constructs mutation admission.

The public API returns safe element descriptions. It retains the backend object and frame/document identity internally. Cross-episode refs, replaced snapshots, changed process/compositor/client generation, and changed frame loaders refuse. OOPIF content remains in observations; resolving an OOPIF ref is explicitly unavailable until its attached-session revalidation is exposed through the safe read API.

## Broad import decision and remaining gates

The full source/build closure passes. Selective retained-snapshot ownership and the read-only BrowserEngine context pass their bounded tests. A stock full-core lifecycle wrapper remains rejected. The original core still contains ambient runtime scope, independent idle eviction/revival, configured authorization fallback, and ungated legacy effect entrypoints. Those modules are private and uninstantiated by the observation API.

`entrypoint-audit.json` inventories Tool implementations, CDP calls, lifecycle revival/eviction, ambient context access, and read-context patches. It is source evidence, not external mutation admission. Direct bypass probes reproduce upstream semantic resolution without session-owner enforcement, bare-row retargeting, anonymous publication after end, and lifecycle revival. The explicit owner rejects those shortcuts.

Ticket 16 remains the release gate for all CDP and AT-SPI effects. The read-only transport does not prove atomic mutation admission or priority over human takeover. Existing-profile grants, preparation, and real browser integration need their own ownership, policy, and fixture evidence. Native capture and same-render Niri scene correlation are separate required capabilities.

## Import and patches

The upstream revision is `88aff7e5424e9ed45b93ded85eb4111472ca5190`. `source-manifest.json` records every original file from the complete core and contract roots and their embedded Skills resources. `imported.sha256` checks unchanged source and the archived platform source. `patched.sha256` checks modified imported source. `patches.json` records original/current hashes and new files; unified diffs are under `patches/`.

`platform-extraction.json` records the CUA Linux identity, trusted-installation, process-role, and listener parsing helpers retained in `niri_evidence.rs`. Their function bodies remain intact with local imports and visibility changes. Exact-root inode checks, operator-only endpoint configuration, argv proof, and Niri callback integration are documented divergences. The original platform source and its tests are archived under `upstream/`; those platform tests are not claimed as executed.

The local workspace manifest resolves upstream workspace dependency declarations, and Cargo.lock records the build closure. MIT attribution remains in LICENSE.md. No deployed Python dependency is introduced. Dormant copied source is not an adopted runtime capability.

## Verification

Run from the Dojo repository root:

```sh
devenv shell -- bash vendor/cua/driver/verify.sh
```

The script verifies source hashes, checks the full minimal-feature core, tests episode ownership and direct bypass probes, runs the entire retained browser suite and snapshot-store suite, and checks public and forbidden-import documentation examples. Browser tests execute real loopback mock WebSockets and retained engine snapshots. Close tests hold a real pending CDP reply and a real engine observation before teardown. Linux tests inspect a real listener through `/proc` and prove endpoint configuration alone cannot authorize it.

The standalone target directory is ignored. This work does not change the root workspace, shared adoption ledger, installed desktop, or Pi scheduler.

## Dojo owned browser route

Dojo's MCP adapter now creates a trusted operator Chromium child with a private temporary profile and fixed starter page. A dedicated subreaper retains daemonized children. Each read revalidates exact native/process ancestry, fixed launch/profile bytes and the literal-loopback root listener inode. The optional separate browser-root identity does not relabel the captured native client. Known sensitive DOM values are removed before retained labels are constructed.

Actual Helium/MCP acceptance covers screenshots, refs, session/window/stale refusals, trusted positive verification, incomplete negative evidence, credential exclusion and owned process/profile cleanup. See [the integration record](../provenance/dojo-owned-browser.md). Navigation remains experimental and disabled by default; split native-client/browser-root navigation bindings refuse before broker dispatch.

`patched.sha256` includes declared modified and new source files. `imported.sha256` contains only unchanged pinned imports. Regenerate declared patches from the immutable git object; do not overwrite copied-file hashes to accept an unexplained source change.
