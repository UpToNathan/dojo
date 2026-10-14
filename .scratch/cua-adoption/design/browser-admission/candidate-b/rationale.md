# Candidate B rationale

## Problem

Niri currently trusts one explicitly selected broker PID/start at startup. The Rust execution service starts lazily and may restart with another generation. Existing agent peers establish connection ownership without authenticated process identity. Making any seat owner a broker would discard the trust proof on which exact page attestation and unique socket ownership depend. The design must retain the compositor's final write and Pi's durable unknown barrier while changing enrollment.

## Usage, caller's view

The MCP adapter calls `LazyDesktop::connect`. The service passes one consumed `PreparedNavigation` to `BrowserAdmission::navigate`. Niri revokes a scope when its live authority ends. [usage.md](usage.md) records these call sites and the explicit operator enrollment; [sketch.rs](sketch.rs) derives their types. Callers never manage registration messages, inherited descriptors, or transport metadata.

## Shape

A minimal Rust launch attester keeps the existing lazy service ownership. The operator pins the live attester process and one fixed launch recipe. Only that process can attest a child it actually spawns. The child receives an exclusive inherited channel; Niri binds it to kernel sender credentials, child pidfd/start, compositor nonce, and recipe identity. This is a deliberately disclosed expansion from trusting one broker execution to trusting a selected launcher to create executions of a fixed recipe. Path or hash matching alone is insufficient, because it cannot prove launch arguments, environment, inherited capabilities, or explicit process selection.

Niri captures seat credentials at accept and binds each admission scope to a live peer owned by the exact enrolled service generation. Each scope has its own ingress and pending writers. Closing that ingress releases unreceived kernel FD references; actor revocation drops received writers and expiry sources. Revocation invalidates the scope before removal, so callback ordering cannot resurrect it. This follows separate-before-serializing-shared-state. A generation replacement retires every old scope before installation. The actor's bounded seen intents and unknown targets stay outside replaceable registration.

The three public calls hide attestation, process-generation validation, page upgrading, FD transfer, cancellation and outcome interpretation. This is a deep interface because callers supply domain authority and receive a refusal or uncertainty without coordinating protocol stages. Boundary-discipline keeps credentials and wire parsing private. Model-the-domain gives consumed capture authority and unique page writer ownership a shape. Existing page/window checks and endpoint-owner proof remain load-bearing broker obligations. No descriptor check can independently prove that the trusted broker has no writable clone.

Pi remains the only durable owner. Its pretransport unknown reservation survives missing receipts and all process restarts. Niri's actor serializes takeover with the final one-shot send, and never waits for a browser to acknowledge before serving human input. Registration creates broker authority, not navigation permission or recovery settlement.

## Synthesis decision

Pending parent arena synthesis. This candidate recommends a separately enrolled launch attester rather than moving service launch/browser lifetime into Niri.

## Tradeoffs accepted

- We accept one additional ephemeral trusted Rust process in exchange for preserving existing service ownership and avoiding a startup-predicted service PID.
- We accept a fixed launch recipe and refusal on unknown Nix wrapper behavior in exchange for explicit launch provenance.
- We accept conservative Unknown after ambiguous transfer or lost cancellation acknowledgment in exchange for preventing accidental replay.
- We accept temporary mutation unavailability after authority loss and unresolved effects in exchange for preserving barriers across activation changes.
- We accept that final liveness reads and descriptor ownership depend on trusted host process isolation; a same-UID adversary allowed to alter enrolled processes defeats process-pin trust too.

## Alternatives considered

A compositor-owned service launcher hides launch protocol in Niri but gives it execution lifetime responsibilities. A separately selected attester concentrates launch knowledge while Niri keeps final coordination authority. Direct hash/path self-registration is smaller to implement but exposes trust construction to every caller and cannot prove an operator-selected execution. A long-lived broker owning Chromium adds another browser owner and scatters read-episode/exclusive-writer retirement across processes.

## Open questions and risks

- Will the operator accept changing the selected trust subject to a launcher restricted to one fixed recipe, or require enrollment of each service generation individually?
- Which host controls prevent ptrace, `/proc` FD theft, uncontrolled inherited descriptors, and mutation of recipe inputs for the explicitly selected processes?
- Which actual Nix ELF/wrapper/dependency closure and environment are selected for this recipe, and do supported Chromium native/root/listener generations satisfy the current exact-PID restriction?
- What accepted Pi/operator authority may settle an unknown outcome and clear matching Niri barriers after new observation? Until answered, no clearing API exists.
- Does the host require launcher restart without compositor restart? This candidate fails closed and requires explicit re-enrollment when the pinned launcher dies.

## Next implementation step

Implement the fixed-recipe launch/enrollment boundary in a disposable nested test runner, then prove a same-image arbitrary invocation cannot enroll before touching browser admission.
