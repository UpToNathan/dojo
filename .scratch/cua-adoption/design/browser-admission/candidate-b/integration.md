# Candidate B integration and proof plan

Design only. No implementation, test run, installed binary replacement, or live desktop change is included. The pinned CUA revision is `88aff7e5424e9ed45b93ded85eb4111472ca5190`.

## Affected owners and callers

| Existing owner/caller | Proposed change | Retained responsibility |
| --- | --- | --- |
| `src/bin/dojo-agent/service.rs`, `ActiveSession::spawn` | Ask pinned launch attester for its fixed per-compositor child slot; authenticate connected service kernel generation instead of trusting a pathname/Hello PID | Lazy connection, Rust session and timeout ownership |
| New minimal Rust launch-attester module/binary | Accept bounded start requests; execute only selected recipe; attest child over pinned channel | No browser ownership, durable store, or scheduler |
| `vendor/niri/src/niri.rs`, external listener startup | Explicit operator pin selects attester generation/recipe; ordinary config cannot construct enrollment | Optional enablement and compositor generation |
| `vendor/niri/src/ipc/server.rs`, `ClientCtx` | Capture kernel credentials and start/pidfd on accept; send actor-ordered peer-close revocation | Connection-local peer ownership |
| `vendor/niri/src/agent_input.rs`, `AgentSeat` | Associate live scope with exact authenticated owner; revoke on seat expiry, teardown, takeover | Existing human priority, geometry and coordination rules |
| `vendor/niri/src/external_admission.rs` | In-module generation registry and per-scope ingress/pending writers; explicit source teardown | Typed final frame and one nonblocking actor send |
| `src/bin/dojo-agent/service.rs`, navigation preflight | Use live enrollment; consume capture/read episode exactly once; maintain cancellation scope | Policy, isolated browser, fresh observation and internal intent authority |
| `src/bin/dojo-agent/browser_mutation.rs` and `crates/dojo-external-admission/src/broker.rs` | Concentrate `navigate` behind authenticated gate and unique FD transfer | Exact `/devtools/page/{target}` upgrade and compositor authentication |
| `src/bin/dojo-agent.rs`, MCP adapter | Forward existing typed URL/capture input; expose optional route availability from live gate | No model-visible enrollment token or fake durable intent |
| `tools/speech/desktop-extension.mjs`, Pi runtime | Preserve committed unsafe reservation, unknown outcome and sequential ordering through restart | Sole durable ownership and `pi-ai` model transport |

Keep launch recipe knowledge in the attester, process/seat/generation admission in Niri's existing external-admission module, and CDP preparation in the existing broker. Do not add a registry service, general permit engine, or transport pass-through wrappers. Remove the static service PID-environment preflight after replacing it with the single enrolled route; do not leave a second gate bypass.

## Generation and FD ordering

1. Operator explicitly pins live launcher PID/start and pidfd plus selected recipe. Niri uses kernel credentials per message and closes its enrollment route on that generation's death or control-channel loss. A same-UID replacement cannot inherit trust.
2. Launcher owns a single child slot per authenticated compositor generation. Concurrent lazy-start requests converge on it. It opens the selected immutable ELF, constructs fixed argv/environment, closes unrelated FDs, launches the child, and obtains its pidfd/start. It hands Niri the child channel endpoint and pidfd with a compositor nonce and recipe digest. Receipt loss queries the same slot; it does not spawn another child.
3. Niri validates the attester and child generation. pidfd metadata must name the actual attested child and be checked against kernel Hello credentials and fresh start identity, not a JSON PID. For socketpair endpoints, creation-time `SO_PEERCRED` describes the creator; authenticate child messages with `SO_PASSCRED`/SCM credentials. Mark channels close-on-exec except the one deliberate child bootstrap inheritance. Trusted service code closes enrollment/admission FDs in every launched descendant.
4. A live service seat obtains an actor-minted scope. IPC kernel owner generation must match registration. A caller cannot reuse another process's peer or an old connection serial. The private per-scope ingress carries bounded one-FD requests and correlated receipts; a separate generation-control path carries cancellation. Niri validates FD type/tuple/listener inode and the broker's exact page/native binding. Supported native/root/listener identities stay distinct; current unsupported split topology still refuses.
5. On service disconnect, explicit cancel, seat expiry, takeover, service death, or generation replacement, actor revocation invalidates the affected scope first. Close ingress, remove all writable/expiry source tokens, and drop every received FD. Closing Niri's ingress releases queued kernel SCM_RIGHTS references; receive errors must also drop all extracted rights, including excess/truncated ancillary data. Writer rearming never escapes scope ownership.
6. Cancellation sent after transfer is a request. Only an actor-produced receipt for the exact attempt proving no attempted send is RefusedZero. A lost acknowledgment, dead peer, process exit or timeout keeps Pi Unknown. Native human input is never delayed waiting for this acknowledgment. A revocation callback processed before ready blocks the write. A ready callback that already attempted send produces Unknown even when a later cancellation runs.
7. Final ready checks include activation/process/seat/scope liveness, capture geometry/revision, browser generation and current human authority. pidfd readiness/fresh generation checks reduce stale-owner admission, but no claim is made that external physical process death or input arrival is atomically ordered with `send`. The enforceable contract orders processed actor revocation/takeover before actor send. No await or browser-side buffered write follows admission.
8. Every attempted send, including partial or full acceptance, enters unresolved state and drops the socket. EAGAIN/EINTR with no acceptance can remain pending with all final checks repeated; a cancellation before a later attempt can then produce zero-dispatch refusal. Neither activation replacement nor session recreation clears global seen intents or unresolved targets. Capacity exhaustion refuses further submissions rather than evicting consumed intent identities.

## Durable uncertainty and approval limits

Pi records unsafe reservation/unknown before the MCP transport begins. Rust intent/grant IDs remain ephemeral one-use capture authority. A Pi tool-call identity may be carried as authenticated extension transport correlation if supported, but a model-supplied string is never proof of committed intent. No durable grant table is introduced. A new refusal settles only its own attempt and cannot clear an older unknown outcome.

Service restart requires new registration, capture, browser episode and seat. Pi's unknown document prevents new follow-on effects after an ambiguous attempt even if Niri restarts and its in-memory barrier vanishes. The direct desktop route remains an explicitly trusted execution route; no claim is made that a new direct client reconstructs lost Pi durable state. Implementation must define authenticated Pi context for Pi-originated calls and preserve the existing extension gate; registration is not a cross-restart recovery certificate.

Recovery is intentionally fail-closed pending accepted authority semantics. Fresh observation supplies evidence but cannot settle either barrier. Do not add a generic clear-unknown request. Do not interpret a launcher enrollment, navigation option, OS deployment approval, or successful capture as durable permission. Existing-profile consent and mutation beyond typed Page.navigate remain separate gates.

## Runnable acceptance target on inactive `dojo-tests`

Implement a Rust test runner `browser-admission-live` under the external-admission crate, reusing the existing nested-navigation actor test controls. This is a proposed executable target, not a command that exists today. Run only on the inactive disposable `dojo-tests` route. It launches source-built Niri, an explicitly pinned test attester, the real lazy Rust service via MCP, a Dojo-owned isolated Chromium profile, and local deterministic pages. It must not depend on a production Python process or replace installed files.

Build with the existing documented devenv commands, then run the proposed target:

```sh
devenv shell -- cargo build --bin dojo-agent
devenv shell -- cargo build --manifest-path vendor/niri/Cargo.toml --no-default-features
devenv shell -- cargo run --manifest-path crates/dojo-external-admission/Cargo.toml --no-default-features --bin browser-admission-live -- --nested-route dojo-tests --require-inactive --scenarios lazy,restart,takeover,cancel,unknown,identity
```

The runner must verify route inactivity before creating disposable source-local/nested artifacts and refuse when it cannot prove isolation. Chromium visits a local fixture with a unique page marker and delayed effect observation. A second local page proves exact target discrimination. Record actual native Wayland client PID/start/window, browser root, listener inode/PID, page target, selected ELF/recipe, service/launcher/compositor generations, actor event order, FD counts, Pi reservations and observations. Fixture self-report and a canned WebSocket server cannot substitute for actual Chromium navigation.

| Scenario | Observable proof |
| --- | --- |
| Lazy and concurrent start | No service before first request; MCP causes exactly one attester-spawned service; fresh observation reaches its exact Chromium page; final writer is Niri |
| Restart and stale replay | Kill idle service, reconnect MCP, obtain new attested generation; old capture/peer/scope/FD/intent refuse; fresh observation can use the new generation only when Pi has no unresolved effect |
| Enrollment negatives | Same-UID arbitrary process, same ELF with arbitrary env/argv, forged launcher packet, unknown recipe, reused PID/start, wrong child pidfd and stolen peer serial all fail before page dispatch |
| Takeover before actor write | Pause after FD transfer/before ready, inject real primary-seat event, process takeover, then release writer; exact Chromium marker unchanged; zero-dispatch receipt; human focus/cursor preserved |
| Takeover after actor send | Delay local fixture effect/result, let actor send first, inject primary-seat takeover; outcome remains unknown, target blocks fresh-ID and cross-seat native/browser effects, human input remains immediate |
| Cancel at each handoff boundary | Pause before sendmsg, immediately after transfer, with SCM_RIGHTS still queued, after writer-source installation and after actor send; actor-acknowledged zero-dispatch cases have no navigation and return FD/source counts to baseline; all ambiguous/attempted cases retain unknown |
| Process/peer/launcher loss | Crash service with queued and accepted writers; close seat IPC; expire seat; kill launcher; old scopes cannot fire and queued FDs are released. Launcher replacement fails until explicit enrollment, without clearing barriers |
| Exact binding | Wrong page/window/listener owner, stale geometry, closed read episode reuse, split native/root unsupported topology and a changed process generation refuse; unique writer transfer leaves no broker writable clone |
| Pi durable loss interval | Pi commits unsafe intent; lose receipt and restart MCP/service, then separately compositor; Pi durable unknown survives, no automatic replay or follow-on navigation occurs; fresh observation alone never clears it |

Use test-only barriers local to the disposable runner, not configurable production pause modes. Run focused Rust credential/ancillary-data/source-cleanup tests and Pi extension tests before the real route. Existing socket fixture evidence is retained as a lower-level regression case. No test was run for this design package.
