# Building Dojo on T3 Code

Research date: 2026-10-11. Current source inspected at `5047ee78858bfdeb254c7386e2d427d3bccb8430`, dated October 11. The installed application was identified separately by the parent research as `0.0.46-nightly.20261008.2849`. Current main's server manifest says `0.0.45`; neither label proves wire compatibility with the installed nightly. No installation, build, tests or user-state mutations were performed.

## Recommendation

The user clarified that the goal is to **talk to T3**, rather than reuse its implementation. No T3 fork or replacement UI inside T3 is required. Dojo can remain its own application and connect as an authenticated client to the running server. The TypeScript bridge recommendation is a way to speak the existing Effect RPC protocol with fewer duplicated transport rules.

Dojo can use T3 as its agent backend. The shortest useful integration is an app-owned TypeScript bridge that uses a pinned T3 RPC/contracts implementation and exposes a small versioned Unix socket protocol to Dojo's Rust/GPUI shell. Attach to the already running T3 environment first. Later, run a dedicated T3 server without Electron if Dojo should own the execution environment. Avoid extracting individual provider adapters initially: their dependency graph includes T3 contracts, shared services, orchestration and Effect.

This preserves the user's already configured native agent subscriptions and our desktop MCP tools. Hex can supply transcription before submitting input. Dojo renders responses, thinking and action state from T3 subscriptions; it does not need to use T3's React UI. Clef or another classifier remains a separate optional component, with its own billing and credentials.

## Existing boundary supports another UI

T3 explicitly puts provider processes, terminals, Git and workspace files in a server-owned environment. Web, desktop and mobile clients use authenticated RPC; Electron's renderer follows that same boundary. `packages/client-runtime` already shares authorization, connection supervision, RPC, focused state reducers and operations across clients. Its README describes public subpath boundaries, so this is a deliberate client/backend separation. [Architecture][overview], [client runtime][runtime].

However, client-runtime is `private: true`, exports source TypeScript and depends on workspace packages. There is no established independent, published Dojo-ready SDK with a promised stable semver API in the inspected material. Reuse means vendoring/building pinned source or maintaining a small fork/bridge. Calling these APIs public within the monorepo does not make them independently published packages. [Manifest][package], [server dependencies][deps].

## Headless server and process ownership

The CLI supports `t3 serve`. It runs the server without opening a browser, prints headless pairing details and disables automatic project creation from the current directory. Flags include host, port, base directory and no-browser. This is a real non-Electron backend, not merely a hidden desktop window. [CLI implementation][server], [flags][config].

The optional Linux background service uses systemd and lingering; macOS uses a login service. Dojo need not install that service to run the foreground CLI. The reviewed CLI/server contracts do not promise automatic process exit at idle or socket activation. Dojo would own those behaviors if wanted. Stopping the server affects running turns, terminals and remote clients; the service docs explicitly describe these interruptions. Attaching to a shared installed T3 server therefore must not let Dojo shut it down just because its own panel is idle. [Service guide][service], [CLI][server].

For a dedicated instance, use an isolated data root and explicit localhost endpoint. Do not open the installed application's database with a second server. Treat all background jobs, running providers, terminals and observers as lifecycle owners before implementing scale to zero.

## Protocol, authentication and discovery

The protocol uses Effect RPC over WebSocket, typed in `packages/contracts/src/rpc.ts`. The client session constructs `RpcClient.makeProtocolSocket` with the WebSocket and RPC serialization layers. This is more than ad hoc JSON messages; a bridge should reuse the pinned protocol implementation rather than reverse-engineer selected frames in Rust. [RPC contracts][rpc], [session implementation][session].

Clients discover environment endpoints/capabilities, authorize with scoped credentials and establish sessions. T3 separates authentication from per-method authorization. The headless flow has pairing/token management; a local client should use these supported flows rather than reading internal database secrets. The parent source audit also confirmed that MCP-scoped credentials do not authorize WebSocket UI/RPC access. Our existing T3 MCP connection is therefore useful for app orchestration but is not interchangeable with a GPUI client subscription connection. [Environment auth][auth], [connection runtime][connect], [CLI][server].

A bridge should negotiate environment descriptors/capabilities and preserve client request IDs. The connection runtime handles reconnect, cursors and reducers; transport reconnection alone is insufficient to reconstruct the current thread or distinguish accepted commands from retries. [Architecture][overview], [connection runtime][connect].

## Input, responses, cancellation and persistence

Orchestration V2 contracts include thread creation, thread launch, turn start and interrupt commands, plus RPC dispatch and subscriptions. The schema defines command inputs and normalized events rather than requiring clients to speak every provider's native protocol. Dojo can submit a voice transcript, observe the selected thread and render its changing state. [Commands][commands], [RPC][rpc].

T3's event log is the orchestration source of truth, with command receipts and transactional durable intent before side effects. This gives thread history and reconnectable state. It does not imply exactly-once desktop clicks after an interrupted provider/tool call. Keep Dojo's desktop actions guarded by fresh observation and the existing pointer coordination revision. Provider-specific restart behavior must remain visible to the UI. [Architecture][overview].

## Native providers and MCP

T3 controls native agent implementations with their own credentials and sessions. Current docs list Codex, Claude, Cursor, Grok Build, OpenCode and Antigravity. Source also contains a Pi provider package. Native provider authentication should remain owned by the existing provider environment, rather than copying subscription refresh credentials into Dojo. [README][readme], [provider constraints][providers].

Pi integration uses Pi RPC and its coding-agent process, including `pi --mode rpc` helpers. It is not pi-durable embedded inside T3. Provider-specific adapters normalize model catalogs, commands and events; their imports include shared provider/core services and T3 contracts. MCP provider-session plumbing appears in Pi and OpenCode adapters, while other harnesses have their own configuration/injection paths. Our desktop MCP is currently configured for Codex; T3's injected, namespaced orchestration MCP does not prove that our separate desktop server is available in every provider. Pi may need an explicit extension or generic MCP integration. This research did not prove that every configured external desktop MCP is injected identically into every provider. Keep our currently working Codex desktop-tools path and verify each additional provider separately before claiming parity. [Pi adapter][pi], [Pi RPC][picore], [provider constraints][providers], [dependencies][deps].

## Licensing and adoption choices

T3's source is MIT, copyright 2026 T3 Tools Inc. It permits modification, redistribution and use in Dojo with preservation of the copyright/license notice. Dependencies and bundled native components still have their respective licenses; MIT on T3 is not a license for provider brands or commercial services. [License][license], [dependency list][deps].

Attaching to installed T3 shares existing configured providers, threads and server state with the least extra process cost. The blocker is establishing a legitimate client connection and matching the installed version/capabilities. Running a dedicated headless server gives Dojo lifecycle/data ownership but adds backend weight and configuration. Extracting provider packages gives more control over scope, but requires reconstructing their shared services and maintaining upstream changes. Start with the bridge, then decide from actual dependency/process needs whether a smaller fork is justified.

No claim of runtime compatibility, successful external client pairing, actual zero-idle behavior or provider-wide MCP parity has been tested in this research.

[overview]: https://github.com/pingdotgg/t3code/blob/5047ee78858bfdeb254c7386e2d427d3bccb8430/docs/internals/overview.md
[runtime]: https://github.com/pingdotgg/t3code/blob/5047ee78858bfdeb254c7386e2d427d3bccb8430/packages/client-runtime/README.md
[package]: https://github.com/pingdotgg/t3code/blob/5047ee78858bfdeb254c7386e2d427d3bccb8430/packages/client-runtime/package.json
[server]: https://github.com/pingdotgg/t3code/blob/5047ee78858bfdeb254c7386e2d427d3bccb8430/apps/server/src/cli/server.ts
[config]: https://github.com/pingdotgg/t3code/blob/5047ee78858bfdeb254c7386e2d427d3bccb8430/apps/server/src/cli/config.ts
[service]: https://github.com/pingdotgg/t3code/blob/5047ee78858bfdeb254c7386e2d427d3bccb8430/docs/user/background-service.md
[rpc]: https://github.com/pingdotgg/t3code/blob/5047ee78858bfdeb254c7386e2d427d3bccb8430/packages/contracts/src/rpc.ts
[session]: https://github.com/pingdotgg/t3code/blob/5047ee78858bfdeb254c7386e2d427d3bccb8430/packages/client-runtime/src/rpc/session.ts
[commands]: https://github.com/pingdotgg/t3code/blob/5047ee78858bfdeb254c7386e2d427d3bccb8430/packages/contracts/src/orchestrationV2.ts
[providers]: https://github.com/pingdotgg/t3code/blob/5047ee78858bfdeb254c7386e2d427d3bccb8430/docs/internals/providers.md
[auth]: https://github.com/pingdotgg/t3code/blob/5047ee78858bfdeb254c7386e2d427d3bccb8430/docs/internals/environment-auth.md
[connect]: https://github.com/pingdotgg/t3code/blob/5047ee78858bfdeb254c7386e2d427d3bccb8430/docs/internals/connection-runtime.md
[pi]: https://github.com/pingdotgg/t3code/blob/5047ee78858bfdeb254c7386e2d427d3bccb8430/packages/provider-pi/src/server/adapter.ts
[picore]: https://github.com/pingdotgg/t3code/blob/5047ee78858bfdeb254c7386e2d427d3bccb8430/packages/provider-pi/src/server/rpc.ts
[license]: https://github.com/pingdotgg/t3code/blob/5047ee78858bfdeb254c7386e2d427d3bccb8430/LICENSE
[deps]: https://github.com/pingdotgg/t3code/blob/5047ee78858bfdeb254c7386e2d427d3bccb8430/apps/server/package.json
[readme]: https://github.com/pingdotgg/t3code/blob/5047ee78858bfdeb254c7386e2d427d3bccb8430/README.md
