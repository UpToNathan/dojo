# Browser ownership and lazy service admission

Scope: read-only source exploration for architect Phase A. No implementation or live proof was performed. The pinned CUA reference remains `88aff7e5424e9ed45b93ded85eb4111472ca5190`. Findings describe the current working tree, not an asserted upstream parity result. Read the adoption spec, issues 07 and 16, the design decision, AGENTS.md, GLOSSARY.md and domain instructions. No applicable ADR directory exists.

## Finding

The lazy service cannot use the current production-shaped experimental navigation gate without a new trusted activation boundary. The service demands its own PID/start equal the operator's preconfigured broker identity (`src/bin/dojo-agent/service.rs:661`), while Niri reads and pins that identity when it installs the external listener (`vendor/niri/src/niri.rs:2637`, `vendor/niri/src/external_admission.rs:212`). A service that exits and respawns obtains a new process identity. Updating environment in the new process does not update Niri's listener closure.

Keep the compositor as the sole mutation writer. Change how a trusted Rust service generation becomes authorized to hand it a prepared connection. Replacing the writer with a service-side check followed by CDP dispatch would reintroduce issue 16's race.

## Existing execution and ownership

| Owner | Resource and authority | Source evidence |
| --- | --- | --- |
| Pi extension | Durable conversation document, effect replay policy, tool ordering, pretransport reservations and recovery barrier | `tools/speech/desktop-extension.mjs:83`, `:117`, `:129`, `:173` |
| MCP adapter | A mutex-protected service connection; model-visible navigation args contain only capture ID and URL | `src/bin/dojo-agent.rs:62`, `:475` |
| Lazy Rust service | Shared process, serialized requests, independent connection sessions, capture authority, optional seat controller, browser episode and owned browser | `src/bin/dojo-agent/service.rs:168`, `:318`, `:1423`, `:1456` |
| AgentController | One live Niri IPC connection and its agent peer ID; dropping the connection releases the seat | `src/agent_input.rs:1`, `:11`, `:28`, `:57` |
| BrowserSession | Tokio runtime, exact-window source, read episode and local ref map; close on drop | `src/bin/dojo-agent/browser.rs:218`, `:451` |
| OwnedBrowser | Private generated profile, launched supervisor child, process generations and ownership proof; no caller-supplied executable/profile/endpoint | `src/bin/dojo-agent/browser_owned.rs:379`, `:389` |
| Transport broker helper | Authenticated Niri probe, page-target HTTP upgrade, exclusive FD transfer and correlated receipt; does not write navigation | `crates/dojo-external-admission/src/broker.rs:45`, `:121`, `:135` |
| Niri actor | Authenticates broker generation, validates live target/endpoint, constructs the navigation frame, owns pending writer and performs final admission plus one nonblocking kernel write | `vendor/niri/src/external_admission.rs:256`, `:302`, `:317`, `:128` |

`ActiveSession::spawn` connects to the versioned private service socket or launches the current executable as the service (`service.rs:172`, `:218`). The path includes Niri socket and operator policy digest (`:111`). `Hello` checks wire version, matching policy digest, name and timeout before creating a `Session` with no controller, capture namespace `connection-{id}`, no browser, and no unknown-effect barrier (`:1441`). Empty service lifetime ends after a short grace; the current constant is 250 ms (`:29`, `:1324`). This makes service identity naturally ephemeral.

Policy is loaded from the operator's optional `DOJO_DESKTOP_POLICY`. The file must be regular, bounded, owned by root or the current user, and not writable by others. Each authorization reloads its digest and fails if changed (`policy.rs:24`, `:31`, `:55`). The service authorizes parsed operations before `handle`; release/status/browser close bypass policy so cleanup remains possible (`service.rs:75`, `:1425`). Policy permission and compositor coordination remain separate checks. With no policy configured, this layer supplies no policy restrictions (`policy.rs:16`).

## Browser reads, creation and navigation

`BrowserCreate` retires earlier capture/episode state, launches an isolated server-owned browser, waits for a successful read observation, then installs both owned lifetime and read episode into the session (`service.rs:573`). Trusted executable checks canonicalize the operator installation, require root-owned immutable ancestors, and restrict supported Chromium product names (`browser_owned.rs:119`). Creation therefore cannot simply become a generic model-supplied launch command.

The owned proof records supervisor root/start, native browser/start, CDP process/start, debugging port, native window and profile. `current_owned` checks liveness, ancestry, exact native-window identity and exact launch/profile command (`browser_owned.rs:340`, `:358`). Native window PID and CDP listener PID can be distinct. Do not erase that distinction in a new registration type.

`BrowserObserve` creates an owned or externally configured read episode, retires old episodes on target change, and discards the episode after observation failure (`service.rs:605`). Native proof checks compositor generation, native process generation, exact window mapping and capture-correlated geometry (`browser.rs:75`, `:200`). Read output reports `actionable:false`, `route:browser_read_only`, and `image_semantic_atomic:false` (`:367`). Ref lookup is local to the episode and resolves under cancellation/deadline control (`:428`). `retire_for_navigation` rechecks frozen native state, clears refs and consumes the engine's bound navigation endpoint (`:295`).

Navigation follows this path:

1. Pi commits a recovery reservation and `externalUnknown=true` before sending the unsafe tool (`desktop-extension.mjs:136`). The registration is sequential and replay-unsafe (`:8`, `:173`). This is Pi persistence, not a Rust run store.
2. MCP forwards typed `BrowserNavigateRequest { capture_id, url }`; transport failure becomes an unknown receipt (`dojo-agent.rs:475`).
3. Service policy authorization occurs before navigation handling (`service.rs:1425`). Handling checks explicit enablement, cancellation, URL syntax, broker socket and the static own-PID/start pin (`:637`).
4. Service obtains the exact-window capture frame, takes the current browser episode, validates window/native PID/start agreement, consumes capture authority and retires captures (`:671`). It lazily creates the controller if needed and uses its Niri peer ID (`:697`). Rust creates an internal intent UUID and capture grant; caller JSON cannot supply either (`:709`). These IDs attest a live reservation. They are not the Pi durable ToolCall ID and are not durable consent records.
5. `browser_mutation::dispatch` consumes the binding/grant and creates a bounded `NavigateIntent`. It permits a numeric IPv4 loopback endpoint and hands transport work to `navigate_cancellable` (`browser_mutation.rs:22`).
6. The helper authenticates Niri against capture compositor PID/start and a fresh nonce, upgrades `/devtools/page/<target>`, transfers the single FD and drops its own socket owner (`broker.rs:110`). Cancellation is checked before descriptor transfer. After transfer, loss of response is unknown and must never replay (`:57`, `:139`).
7. Niri authenticates sender PID/start/UID, checks bounded request and pending count, resolves exact native target, verifies endpoint socket ownership, burns the intent in its bounded in-memory set, and registers a writable source (`external_admission.rs:250`). The writer rechecks ownership revision, capture geometry and native process generation immediately before `send(MSG_DONTWAIT|MSG_NOSIGNAL)` on the event thread (`:144`). Would-block/interruption returns to the event loop and rechecks on a later turn. No partial frame continuation is sent.
8. Any attempted dispatch is unknown, including a complete accepted frame; Niri installs its exact-target unresolved barrier (`:173`, `:201`). The service installs its session barrier on unknown/error and drops the controller (`service.rs:740`). Pi clears its durable barrier only on an explicit refusal with zero accepted bytes (`desktop-extension.mjs:143`).

## Cancellation, pending effects and cleanup

The connection reader continues reading while a serialized effect runs, so EOF sets `cancelled` before `Disconnect` reaches the service loop (`service.rs:1214`). Disconnect then drops that session (`:1373`). Release closes episode and owned browser, releases controller and retires captures (`:549`). Browser close also retires capture state and explicitly reports that it does not clear the recovery barrier (`:596`). Expiry closes browser resources after capture TTL, and controller expiry is independent (`:345`).

An FD already transferred to Niri is no longer governed directly by the service atomic flag. Existing cancellation then relies on controller/peer retirement, compositor revision/target checks and the writer deadline. A blocked service waits for its receipt before the serialized disconnect event is handled; therefore the existing code does not prove immediate cancellation of a transferred, pending writer on client loss. Before implementation, specify and prove cancellation ordering at descriptor-transfer and final-write boundaries.

Niri caps pending writers at 16, remembers at most 4096 intent IDs, and has a bounded writer lifetime (`external_admission.rs:54`, `:323`, `:360`). Those are ephemeral coordination records. They are not conversation persistence. A writer drop without a reported dispatch reports refusal (`:107`). A transferred socket's mutation outcome remains unknown if the receipt is lost.

`BrowserSession` closes the imported episode when dropped (`browser.rs:451`). `OwnedBrowser::close` revokes its alive proof, signals owned descendants through pidfds and the unreaped launcher's process group, waits for exit, and preserves the profile if cleanup cannot prove descendants exited (`browser_owned.rs:562`). Drop attempts the same cleanup (`:627`). It does not scan or terminate personal browser profiles. The dedicated supervisor validates parent PID/start at startup (`:236`).

Pi MCP transport closes on timeout/abort and kills its child, escalating after grace (`desktop-extension.mjs:48`, `:66`). Runtime stop/close calls release before waiting on storage (`pi-runtime.mjs:232`, `:248`). Reconnect invalidates capture and verifies the pinned catalog fingerprint (`desktop-extension.mjs:93`). Read observation can update capture freshness but does not clear `externalUnknown` (`:149`). No explicit durable recovery-clear operation is present in this extension. Niri's unresolved target set also has no recovery clear path in the inspected module. A registration refresh must not silently clear either barrier.

## Proposed ownership boundary, not established behavior

Expose an ephemeral trusted activation registration in Niri for a Dojo service generation. A suitable record needs the service process identity and pidfd, executable/installation authorization, compositor generation, policy identity, activation generation and a live connection whose closure revokes the registration. Bind each agent peer/session to that registered service, rather than accepting a caller-supplied peer merely because the sender shares the user's UID.

The root design question is who attests the activation. Same-UID self-registration, caller-supplied PID/start, executable path matching alone, or a reusable bearer token inherited by arbitrary tools do not preserve the current operator pin's trust claim. Prefer a compositor-owned activation channel or an explicitly trusted launcher that creates the service and hands over an exclusive registration capability. Selection depends on the deployment trust model. The activation can be lazy and all registration records can remain in memory. No durable scheduler, Rust conversation database or Python service is needed.

Retain the current final writer and its route-specific request. Registration authorizes a service generation to submit prepared page sockets; it does not authorize a UI effect. Every effect still consumes fresh capture authority, binds the exact native process/window and page target, preserves native/CDP process distinctions, checks policy, and passes Niri's final coordination admission. Niri should recheck live activation and peer/session membership at final dispatch, revoke pending sources on activation/session closure, and keep already-written effects unknown. A service restart must obtain a new activation and new capture/episode/controller resources.

Carry Pi's committed tool intent identity as a correlation input with a typed boundary, while Rust still owns the single-use capture reservation and final transport request ID. Do not treat a string supplied by model arguments as proof of durable intent. Retain Pi's unsafe replay and recovery document through service/compositor restart. The compositor's bounded seen-intent and unresolved-target records remain admission protection within its lifetime; Pi is the only cross-restart authority.

## Facts still requiring proof or a design decision

- Trusted activation provenance and revocation semantics are unspecified. This report does not claim that a new executable check or same-UID credential check is sufficient.
- The current reader episode has an engine-produced page binding, but Niri trusts the broker's page-target attestation after validating listener ownership. A replacement boundary must preserve the trusted attester; process/window identity alone does not identify a page.
- Native browser and listener subprocess handling must be exercised through the real isolated browser and Nix wrapper installation. The service compares the binding's native PID to capture PID; Niri's endpoint verifier receives that PID. Whether every supported launch topology satisfies that endpoint ownership proof remains unproved here.
- Pending FD cancellation on client loss, service death, seat expiry and activation replacement needs explicit tests at each boundary. Registration must not revive closed sessions.
- The scoped unresolved-effect recovery protocol is incomplete in the inspected code. Fresh observation is necessary but does not itself authorize clearing the durable or compositor barrier.
- The owned-browser expiry timestamp is checked in `expire`, but the service's deadline calculation includes seat/capture/read-episode deadlines, not `owned_browser_last_use` (`service.rs:1330`). A browser kept alive solely by another renewal source may need an explicit expiry wakeup. This is a source finding, not a reproduced runtime bug.
- Browser creation already performs profile/process/window effects. This exploration proves its owner and cleanup path, not equivalence to final-writer human admission for launch itself.
- No deployment change is authorized by this design. Installed compositor/binary/config updates remain a separate approved operation. Pi remains the sole scheduler and durable owner; pi-ai remains transport owner.
