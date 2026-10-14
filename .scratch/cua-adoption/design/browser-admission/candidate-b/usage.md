# Lazy service with an explicitly enrolled launch attester

This is a design sketch, not implementation or deployment authorization. It follows the pinned CUA baseline `88aff7e5424e9ed45b93ded85eb4111472ca5190` and the three browser grounding documents. Apply architect and unslop to this package.

The operator explicitly selects one already running Rust launch attester process generation and its fixed service launch recipe before enabling this optional Niri gate. This replaces the current selected broker generation with a selected child-launch authority. That change in trust scope must be stated during enrollment. Ordinary configuration, a model argument, a service environment flag, and an arbitrary same-UID process cannot enroll the authority.

The attester has no conversation state, action queue, browser owner, timer-driven scheduler, or durable database. It answers bounded lazy-start requests and attests only the child it actually launches. The service retains sessions, captures, browser ownership, timeout handling, and CDP preparation. Pi retains all durable tool reservations and uncertainty. `pi-ai` retains model transport.

## Caller usage, written before the types

The current MCP adapter asks for the same lazy service. `LazyDesktop` privately uses the attester instead of allowing a requesting MCP process to choose the service executable or its launch environment.

```rust
let mut session = LazyDesktop::connect(niri_socket, session_name, idle_timeout)?;
// Service may not exist yet. Its start is attested without a predicted child PID.
let observation = session.browser_observe(input)?;
```

Inside the Rust service, the navigation caller still retires the read episode and consumes capture authority before requesting an effect. The gate hides registration, session-channel creation, descriptor transfer, and cancellation acknowledgments.

```rust
let admitted = captures.admit_target(input.capture_id, &current_frame)?;
let page = browser.retire_for_navigation()?;
let prepared = PreparedNavigation::from_observation(page, admitted, input.url)?;
let outcome = admission.navigate(&session_lifetime, prepared, &cancelled)?;
// Only an authenticated RefusedZero can settle this attempt as no dispatch.
// Unknown retains Pi's committed externalUnknown reservation and stops effects.
```

On the compositor actor, seat teardown and human takeover use one revocation operation. The operation consumes the actor-bound scope, closes its ingress, removes all queued writers, and preserves unknown targets.

```rust
state.external_admission.revoke(scope_key, reason);
// Native ownership transition and cancellation invalidation happen on this actor.
// A later writable callback must observe the revoked scope before any send.
```

## What the operator enrolls

Niri pins the attester's kernel PID/start and pidfd, the compositor generation, and a recipe digest. The attester itself is explicitly selected as a process, as the current broker is. A path or content hash alone does not preserve that claim. The same executable launched with different arguments, preload environment, inherited FDs, or configuration may behave differently. A hash proves bytes, not a selected execution or an operator decision to trust it.

The selected recipe contains the exact Rust ELF opened for `execveat`, argv, an environment allowlist with fixed values, policy identity, allowed inherited descriptors, and resolved immutable Nix installation dependencies. Runtime compositor socket/display values come from the authenticated enrolled compositor connection and must match its generation. Requests supply only a start request and bounded session metadata. They cannot select `--service` alternatives, other commands, preload values, wrapper behavior, arbitrary policy files, or a general environment.

The attester executes the selected FD, creates a dedicated seqpacket enrollment channel, and passes its service end only to that child. It transfers the Niri end and the child's pidfd through its pinned authenticated control connection. It closes every temporary duplicate before reporting the child ready. Child messages carry kernel SCM credentials, checked against the attested PID/start and pidfd. The child authenticates the compositor generation on that same channel. There is no public reusable enrollment token.

The trusted launcher is the unforgeable origin relative to untrusted MCP clients: Niri accepts attestations only from its operator-pinned process generation, received on its pinned live connection. A peer that merely possesses an IPC seat or repeats recipe metadata cannot manufacture that origin. The trust assumption includes preventing untrusted code from ptracing or stealing descriptors from enrolled processes. If the host allows such tampering, the current process pin also does not establish isolation; implementation must refuse a claimed stronger boundary.

## Cancellation and results

Each service session obtains a separate admission ingress associated with its authenticated live Niri peer. Closing that scope drops kernel-queued SCM_RIGHTS references and actor-owned pending writer FDs. Cancellation requests carry the scope identity on the generation control channel, so they cannot queue behind a blocked page FD write. An authenticated actor acknowledgment proves refusal only if no send was attempted for that exact intent. Caller timeout, process death, or loss of acknowledgment produces Unknown after possible transfer. It cannot prove cancellation.

Takeover processed before the final nonblocking compositor send prevents dispatch. A send attempted before takeover may take effect later in Chromium. Even full frame acceptance is Unknown. Niri never sends a remainder or automatically retries. A session release, attester restart, service restart, fresh observation, and new registration never settle an unknown effect.

Neither navigation approval options nor recovery authority are invented here. Existing Pi outcome/persistence behavior and separate installed-system approval apply. Mutation remains optional and gated. An explicit recovery protocol needs accepted semantics before either Pi or Niri obtains a clearing API.
