# Problem

The compositor already owns the final CDP frame write, live target checks, and unknown-target barrier. Its current listener pins a broker PID/start at compositor startup, while the Rust service is lazy and may restart with a new process generation. Candidate A changes activation ownership: Niri launches the configured service on first demand and owns that process generation for its lifetime. Exact page/window/native/process binding, one-FD transfer, exclusive-writer assumptions, Pi's durable reservation and unknown barrier, and `pi-ai` transport remain intact. The target is pinned CUA `88aff7e5424e9ed45b93ded85eb4111472ca5190`.

# Usage (caller's view)

See [usage.md](usage.md). Pi reserves and records unknown before MCP transport. MCP connects or requests compositor activation and forwards typed navigation arguments. Rust consumes one current capture/read episode, prepares the exact page connection, and transfers a single FD. Niri owns the only writer and returns either a pre-write refusal or an uncertain outcome. A user-facing presentation flow is intentionally not specified.

# Shape

Niri is the activator, process supervisor, and revocation owner. Operator enrollment is immutable compositor configuration identifying which installed Rust artifact Niri may launch. It is an explicit trust enrollment; it must not be inferred from UID, seat ownership, service-supplied PID, pathname, or environment. The artifact identity format (for example signed package identity versus immutable file identity) is deliberately unresolved pending the actual install/update model. Launching only the enrolled artifact preserves lazy service demand without asking a newly spawned process to vouch for itself.

Niri passes an exclusive inherited socket capability to the child, monitors pidfd and channel closure, and binds registration to activation ID, compositor generation, kernel-observed process generation, and session. At IPC accept, Niri records peer credentials/start and binds them to seat/peer; admission requires that exact registered activation and owning peer/session. Process/activation loss first marks the scope dead, then removes all associated pending writers and closes their FDs on the actor before teardown acknowledgement. At the final write, the same actor checks liveness and current target/coordination state. This gives revocation a total order against the one write. A write already accepted remains unknown.

The request data structure is per activation and indexed by session, peer and writer; no cross-generation registry or durable Rust database is needed. Prepared-page data is parsed behind the admission boundary and retains distinct native-window and browser/CDP process identities. The public surface is small: activate/connect, submit a prepared page, revoke scope. It hides process start, FD lifecycle, actor registration, endpoint verification, event source, and writer teardown. No transport or raw protocol type escapes. Validation stays at the socket/config boundary and domain admission accepts typed identities (`boundary-discipline`, `type-system-discipline`). Per-activation ownership avoids shared mutable state between service generations (`separate-before-serializing-shared-state`).

The service still does not become an alternate writer. It verifies exact page target and native window through the fresh read episode and transfers the sole upgraded socket; Niri still constructs and writes only the bounded `Page.navigate` frame after final checks. It is not proved that the FD itself proves no clone exists: exclusivity remains an obligation of the trusted Rust broker and needs route-specific evidence. After transfer, cancellation asks Niri to revoke the activation/session scope; it is not rollback. Pi remains the sole durable scheduler and owner of cross-restart unknown state. Niri's in-memory barrier may not be cleared by activation refresh, registration, fresh observation, or restart. No explicit accepted recovery settlement mechanism is established, so the design cannot specify clearing unknown state safely.

# Synthesis decision

Not synthesized yet. Candidate A explores compositor-owned lazy activation as the primary shape; the orchestrator compares it with the independently prepared candidate.

# Tradeoffs accepted

- We accept making Niri responsible for launching and monitoring one configured service artifact in exchange for compositor-owned process provenance and deterministic revocation.
- We accept an immutable enrollment/update contract in exchange for avoiding self-registration based on same-UID credentials or mutable runtime claims.
- We accept keeping process/session/writer coordination ephemeral in exchange for avoiding a second durable owner; Pi alone carries uncertainty across restarts.
- We accept that an already-written navigation cannot be cancelled in exchange for ordering all pre-write teardown against the final actor write.
- We accept an unresolved enrollment identity format and unresolved settlement API because neither is justified by the inspected accepted mechanisms.

# Alternatives considered

- A separately trusted launcher could create the lazy service and hand Niri a registration capability. It keeps service activation outside the compositor but adds another trusted component and a second lifetime/revocation protocol; its smaller Niri surface would expose more deployment coordination.
- Same-UID IPC credentials or agent-seat ownership could bind a process to a peer, but expose operator trust policy to the seat path and do not establish authorization to broker CDP or exclusive page ownership.
- The current static PID/start pin cannot admit a service generation created after compositor startup or survive service restart.
- A service-side direct CDP write hides admission complexity in fewer files but removes the compositor's final coordination point and reintroduces the side-channel race, so it violates the requirement.

# Open questions and risks

- Which operator-managed artifact identity can Niri verify across package upgrades while remaining an explicit enrollment: package signature, immutable deployment generation, or another existing install identity?
- Can the Niri child lifecycle safely supervise the current `--service` executable and preserve the existing per-compositor socket/policy boundaries without creating duplicate service instances?
- Does the host install model permit Niri to inherit a private capability FD into the service without broadening ambient authority?
- Which existing accepted Pi mechanism, if any, authorizes clearing durable unknown state after a receipt-loss recovery? What actor operation may clear Niri's unresolved target barrier?
- Which outcome presentation and approval semantics should the product expose? The current sources establish navigation policy and explicit operator trust separately, not a new consent grant.
- Do real isolated Chromium/Nix topologies satisfy exact endpoint-owner and native-window binding assumptions, and can the transferred page socket be shown to have one writer?

# Next implementation step

Prototype the Niri-owned activation lifecycle and its inherited revocation capability behind the current nested fixture, then prove credential binding and cancellation ordering before wiring the live Chromium-to-MCP route.
