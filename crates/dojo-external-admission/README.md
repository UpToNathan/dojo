# External admission feasibility prototype

Executable deterministic model for CUA adoption ticket 16. It does not integrate
with Niri, CDP, AT-SPI or the shared service, and it does not enable mutations.
There are no production transport integrations. The model has controllable
typed fixtures with separate commit, dispatch, effect and receipt steps. The
socket spike additionally exercises real loopback WebSocket and authenticated
Unix zbus fixtures; see [the writer proof](SOCKET-PROOF.md). Source design comes from `.scratch/cua-adoption/issues/16-side-channel-admission.md`
and the reviewed design decision, rather than an imported CUA ownership gate.
CUA's agent locks cannot order against human input.

Run through devenv:

```
devenv shell -- cargo test --manifest-path crates/dojo-external-admission/Cargo.toml
devenv shell -- cargo run --manifest-path crates/dojo-external-admission/Cargo.toml
devenv shell -- cargo run --manifest-path crates/dojo-external-admission/Cargo.toml -- --sockets
```

## The proved model guarantee

An actor turn performs native identity/revision/permission checks and calls a
sealed synchronous nonblocking transport try-commit. Admission occurs at the
successful irreversible local transport commit within that same turn. All human
coordination transitions run on the same actor thread. A human takeover that
runs before that commit prevents it; every queued intent is canceled. A zero-byte
WouldBlock result leaves an intent queued and requires all checks again before
another try. No preliminary query produces a transferable permit.

Human takeover increments the coordination revision, records human client
ownership, cancels queued requests, and marks committed requests unknown. It
never receives a transport reference, holds an external lock, invokes an app,
awaits a future, or waits for a receipt/timeout. Actor state, grant count, target
count and fixture packets are capped at 64; mutation payload lengths are bounded.
This is an algorithmic ordering/bounded-work proof, not a latency measurement.

`ExternalPermit<'a>` is borrowed, non-cloneable and has private fields. Only
`Actor::try_dispatch` creates it, immediately passing it to a sealed adapter.
It cannot be retained for a later asynchronous send. Adapters can copy command
bytes only as part of the synchronous commit; no caller obtains a reusable
permission object. The model and experimental socket adapters are sealed because a synchronous Rust signature alone
cannot stop an implementation from doing blocking I/O or secretly buffering a
later write. Adding a real implementation requires auditing that exact path.

The model's successful commit is an irreversible fixture packet insertion. The
socket adapters use one nonblocking kernel send, with actual accepted-byte
evidence and close-on-partial behavior.
A real adapter must identify its corresponding transport linearization point,
including TLS/WebSocket/zbus buffers, partial framing and retry behavior. An async
`send` or actor-issued token followed by a send on another task does not satisfy
this proof. Compositor event scheduling must also prioritize/process native human
input before a later admission turn; arrival timestamps alone are not actor order.

## What remains impossible under this contract

Transport acceptance is earlier than application effect. The fixture deliberately
allows a command accepted before takeover to dispatch and apply after takeover.
That cannot be rolled back by this actor. A late successful receipt remains an
unknown outcome with receipt evidence. It never becomes verified UI success.
`NoApplicationEffectAfterTakeover` returns `UnsupportedOrdering` without a commit.
An application-side cancelable admission mechanism would be required for that
stronger guarantee; plain CDP/D-Bus requests have no such demonstrated mechanism.

Partial writes are unknown and cannot retry. Disconnected means proven zero-byte
acceptance; adapters with uncertain acceptance must report the partial/unknown
case. Lost replies and post-commit deadlines remain unknown. Every outcome requires
fresh observation. Commit retires every earlier actionable observation generation in the actor,
including other windows. An unresolved
in-flight/unknown request also blocks new actionable stamps for that client until
a receipt establishes transport completion; read-only observation outside this
model can continue. Lost/partial effects can remain blocked indefinitely because
there is no proof they settled. Starting a new actor cannot prove an old external
request settled; production recovery must retain that uncertainty across restart.

## Identity and authority

A target includes compositor epoch, Wayland client ID/generation, window
ID/generation, PID/start time, endpoint generation and document generation.
Changing any mapped identity cancels its queue and marks its already committed
operations unknown. Removed windows refuse. Observation stamps, grants and intent
handles carry an actor-instance identity that cannot cross another actor or restart,
even if numeric epochs and counters coincide. Grants bind exact target and typed
mutation, expire, are revocable, and are consumed when preparing one intent.
Observation stamps are consumed Rust values, with generation retirement after commit.

`install_target`, `observe` and `grant` are trusted fixture injection boundaries.
They do not prove real process ownership, capture correlation or human approval.
A production Niri bridge must supply authoritative current state and initialize
actual human ownership; a Dojo policy boundary supplies the exact approval.
Model-visible arguments cannot invoke those authority constructors. Fresh native
observation generations must come from capture/semantic stores, never model input.
Receipt calls must originate from the authenticated, correlated transport.

## Boundary coverage

Tests take over after preparation, preliminary check, blocked write, successful
commit, external dispatch, application effect and receipt readiness for both
routes. They assert zero commits before admission, unknown late effects after
admission, canceled follow-ons, immediate actor state changes, and no replay.
Other cases cover every identity generation, process/PID reuse, wrong actors and
windows, epoch restart, grant revocation, session teardown, partial writes, lost
replies, deadlines, route mismatch, stronger-ordering refusal, and observation
retirement. The default executable mode prints a reviewable commit/takeover/late-receipt/replay
trace without touching the desktop or network.

Production enablement remains blocked on real nested human-input proof and audited
innermost transports. Every browser/AT-SPI mutation, including navigation,
set-value, input, dialog, download, setup and preparation, must use that demonstrated
path. Unknown/unclassified requests refuse. Permission approval remains distinct
from coordination admission.
