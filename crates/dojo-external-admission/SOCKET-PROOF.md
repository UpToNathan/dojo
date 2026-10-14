# Real writer feasibility

The local socket spike demonstrates a workable final-write mechanism. It does
not establish production endpoint identity, Niri event ordering or exact native
window correlation, and it enables no desktop mutations.

Run the reviewed artifacts:

```
devenv shell -- cargo test --manifest-path crates/dojo-external-admission/Cargo.toml
devenv shell -- cargo run --manifest-path crates/dojo-external-admission/Cargo.toml -- --sockets
```

## Source constraints

The pinned CUA CDP implementation uses `ws.split()` and
`writer.send(Message::Text(...)).await` in
`libs/cua-driver/rust/crates/cua-driver-core/src/browser/cdp_ws.rs`, lines 263
and 385. In tokio-tungstenite 0.24, `Sink::start_send` explicitly treats a
WouldBlock write as accepted and queued. `poll_ready` and `poll_flush` later flush
that queued data. Tungstenite 0.24 also defaults to a 128 KiB write buffer. A
preliminary compositor revision check before this send cannot revoke those bytes
before a later library flush.

In zbus 5.19, `Connection::send` acquires the async `socket_write` mutex before
calling `WriteHalf::send_message`. Its default implementation loops over async
`sendmsg` until all message bytes are sent. `call_method_raw` also acquires a
serial-number semaphore and creates the message. These operations can wait on
transport readiness outside the compositor actor. Holding that mutex does not
order against Niri human input. Moving the ownership query earlier preserves the
race.

The relevant source seams are public. zbus exposes `connection::socket::Socket`,
`Split`, `ReadHalf`, and `WriteHalf`, and `Message::method_call(...).serial(...)
can serialize a descriptor-free request without sending it. A custom socket
writer can override `send_message` rather than inherit the async partial-write
loop. A library fork is not required merely to expose this seam. A coherent
adapter still needs authentication, reader routing, serial ownership, operation
classification, cancellation and connection lifetime.

## Actual socket evidence

The cleartext WebSocket fixture performs a real HTTP upgrade with tungstenite,
then hands the writer FD to `RawWebSocketWriter`. A prebuilt masked text frame
contains exactly `Page.navigate`, a bounded URL and the actor intent's request ID.
The actor performs one `send(MSG_DONTWAIT | MSG_NOSIGNAL)`. The peer uses
Tungstenite's real parser, validates the method and URL, and holds its effect.
Human takeover runs while that effect remains held. After release, the peer
applies one fixture effect and sends a real JSON acknowledgement. A separate
read-only decoder correlates its ID. Applying that receipt to the actor preserves
the unknown outcome.

The D-Bus fixture uses real EXTERNAL authentication against a zbus p2p object
server, followed by a real serialized `org.a11y.atspi.Action.DoAction(7)` request.
The server decodes and dispatches it through the generated zbus interface. The
application method waits on an independent fixture notification before effect.
Human takeover completes while it waits. The real bool method return is decoded
with safe zvariant header/body deserialization and matched by reply serial. It
also preserves unknown with late-receipt evidence.

Two additional Unix stream fixtures exercise real kernel pressure. A small
SO_SNDBUF filled with read-only Peer.Ping bytes makes the mutation syscall return
WouldBlock with zero accepted bytes. Takeover cancels the queued intent, and
reading the peer proves that no mutation bytes were appended. A 16 KiB semantic
text message produces an actual partial write. The adapter immediately shuts down
the connection, the actor reports unknown, retry refuses, and the peer receives
exactly the reported prefix followed by EOF. These pressure streams are wire
fixtures, not authenticated application connections.

The raw writer refuses target generation and intent-handle mismatches before its
syscall. Reader methods never borrow the actor while awaiting socket data. They
return an opaque `CorrelatedReceipt`; applying it is a separate short actor turn.
Read and write dup FDs share O_NONBLOCK flags, so the separate readers handle
WouldBlock with bounded polling rather than changing shared socket flags.

## Minimal production broker

Keep protocol preparation and response handling in a Rust broker, and make the
compositor actor the exclusive final writer.

1. A trusted connection bootstrap establishes the endpoint, completes handshake
   or D-Bus authentication, and records native client/window/process identity,
   PID start time, compositor epoch, endpoint generation and protocol target
   identity. CDP needs the exact target/session/frame/document evidence; D-Bus
   needs the unique bus owner and observed object identity. The native-window
   relation must be proved before issuing a mutating intent.
2. Registration transfers the nonblocking writer FD to Niri through an
   authenticated service channel with explicit session/generation binding.
   All previous writable clones and library sinks must relinquish write access.
   Passing an FD alone does not attest endpoint identity or exclusive ownership.
3. Preparation creates bounded immutable framed bytes and a typed mutation
   descriptor bound to the exact one-use intent and permission grant. The actor
   must compare those bytes with the admitted operation, not trust a caller's
   classification label. Native human input and stale-target updates revoke the
   same queued record that writable readiness would dispatch.
4. On a writable event, the Niri event thread checks current target, ownership,
   revision, observation generation, grant, deadline and exact prepared intent,
   then performs exactly one nonblocking syscall. WouldBlock accepts no bytes
   and waits for a future readiness turn with fresh checks. Partial acceptance
   closes the connection and becomes unknown; no remainder is sent.
5. A separate read task supplies correlated result events with endpoint and
   request generation. Human processing never waits for that task. Late results
   preserve the already-reported uncertainty. Pending effects block follow-ons;
   durable recovery retains ambiguous effects across service/compositor restart.

This is a compositor-owned writer design. A dedicated broker that performs the
write after querying Niri or receiving a transferable permit would recreate the
race. The broker may prepare/read, but the actual mutating write must remain in
the compositor coordination turn, or use a separately proved application-side
admission mechanism.

## Practical initial scope

A cleartext loopback Chromium connection is the smallest practical first CDP
scope. This spike implements only Page.navigate. Adding the rest of CUA's commands
needs an audited classifier and existing target/element proof, including observation
scripts and control messages. Arbitrary Runtime.evaluate never becomes read-only
because its caller calls it observation. Unknown commands refuse.

TLS is outside this commit mechanism. A TLS library can queue ciphertext after
accepting plaintext; raw FD sends do not replace that protocol. Refuse wss/remote
mutations until a coherent TLS writer has its own demonstrated admission frontier.

The clean WebSocket handoff fixture sends no post-upgrade events before the first
request. Production bootstrap must preserve any already-buffered frames and full
codec state. Tungstenite readers can automatically write pong/close frames; the
spike's read-only stream refuses writes. A production broker must route audited
control responses through the exclusive writer rather than silently regain a
second writer.

For AT-SPI, override zbus's custom WriteHalf so it submits fully serialized,
classified messages to the actor and awaits only its commit outcome. The separate
zbus reader and proxies can retain their coherent decoding behavior. An explicit
serial-to-intent registry must carry authority; task-local or process-global
"current permit" state is insufficient. All unregistered mutating proxy calls
refuse. Authentication/Hello, read-only property/method calls, subscriptions and
control traffic require separate audited categories. The default send loop must
never receive an admitted mutating frame.

The Unix socket peer on an actual accessibility bus is the bus daemon, not the
application. EXTERNAL authentication and peer credentials do not prove the target
application. Unique-owner PID/UID/start-time proof and exact accessible-to-Niri
window correlation remain mandatory. The current Dojo read-only observer does
not yet provide that window correlation, so no AT-SPI mutation can be enabled
solely on this socket result.

The opt-in writer below implements Niri actor installation, authenticated FD
registration, native endpoint checks and one typed `Page.navigate` operation.
Broader message classification, production browser/MCP parity and effect settlement
remain release gates. Already-accepted CDP or D-Bus calls may begin application
effects after takeover, so those outcomes remain unknown.

## Opt-in compositor writer

`vendor/niri/src/external_admission.rs` supplies a narrowly scoped eventloop writer
for one cleartext IPv4 loopback `Page.navigate` frame per connection. Startup
requires `DOJO_EXTERNAL_ADMISSION_SOCKET`, `DOJO_EXTERNAL_TRUSTED_BROKER_PID`
and `DOJO_EXTERNAL_TRUSTED_BROKER_START`. All three are operator settings;
ordinary browser observation never enables this socket.

The Unix datagram receiver checks kernel credentials against the pinned broker
process and UID. Registration carries one FD with close-on-exec, a durable intent
and capture-authority grant, exact page target, native window, peer/revision and
browser process start time. Niri validates the mapped client's PID, and checks
that the listening TCP socket inode belongs to that exact browser process.
The trusted broker remains responsible for exact page/window correlation and
relinquishing every write owner. Endpoint PID alone does not prove a tab target.

The actor constructs the masked WebSocket frame from a typed URL and request ID.
No caller-provided raw bytes or arbitrary method classification is accepted.
Writable readiness checks the current target, human ownership and coordination
revision immediately before one nonblocking send. WouldBlock sends nothing and
rechecks on another readiness turn; a bounded timer retires pending writers.
Intent IDs are burned on registration, including expiry and unknown outcomes.
A full or partial write closes the single-use connection and reports unknown.
There is no automatic resend, remainder send or confirmed-effect claim.

`broker::navigate` performs an exact `/devtools/page/{target}` HTTP upgrade,
validates the accept key and rejects unsolicited extensions. It reads HTTP headers
one byte at a time, so no bootstrap bytes disappear in a discarded library buffer.
It transfers its only socket owner, drops the local FD and waits for a bounded,
correlated actor admission receipt. It creates no WebSocket sink or writable clone.
There is no application-result reader in this first single-use scope; every admitted
navigation requires a fresh observation and durable recovery decision.

This is an implemented opt-in seam, not a released mutation capability. Production
route registration remains refused until nested human takeover, endpoint/target
identity and crash-boundary scenarios pass. The normal browser engine retains its
read-only gate. The fixture-level socket tests are necessary source evidence and
are not a substitute for that nested proof.

The compositor retains unresolved effects by exact native window, browser PID and
start time, and compositor PID and start time. A new peer, connection, capture or
intent cannot clear that record. Native agent actions on the window use the same
refusal; primary-seat human input remains available. The Rust service blocks its
own subsequent mutating messages and retires the agent seat after uncertainty.
Pi persists its recovery barrier before transport. No settlement or host recovery
command clears the compositor record in this initial scope, so navigation remains
experimental and disabled by default.

`grant_id` identifies an internal capture-authority reservation under the explicit
operator configuration. It is not an authenticated durable consent record. Tool
arguments contain only `capture_id` and `url`; they cannot supply a peer, endpoint,
intent or grant. Disabled routes and other preflight failures return a structured
zero-byte refusal. Failures after the broker boundary report unknown.

## Disposable nested proof

Build the existing Rust fixture, compositor and Rust runner with `devenv`:

```sh
devenv shell -- cargo build --manifest-path tools/agent-input/Cargo.toml --bins
devenv shell -- cargo build --manifest-path vendor/niri/Cargo.toml --no-default-features --bin niri
devenv shell -- cargo run --manifest-path crates/dojo-external-admission/Cargo.toml --no-default-features --bin nested-navigation
```

The runner starts its own nested compositor and one native fixture window. The
fixture PID owns a real loopback WebSocket listener. The runner supplies kernel
broker credentials, exact captured geometry and primary-seat input through the
nested Wayland session. It holds the application effect after transport admission,
then sends human input before releasing that effect.

The 2026-10-12 run passed at
`/tmp/dojo-navigation-live-1286487-2526190559797169946`: takeover before registration
produced a zero-byte refusal, takeover while the accepted effect remained held
preserved unknown, the fixture recorded one effect, a used intent could not
send another frame, and a new peer with a fresh intent could not bypass the unresolved
window. Native agent focus was refused while primary-seat events remained available. `result.json`, `niri.log` and `fixture.jsonl` contain the evidence.
This run proves the actor and broker path with a native fixture. It does not prove
a live Chromium BrowserSession or MCP navigation round trip.

Production broker linkage excludes the `socket-fixtures` feature. The standalone
proof executable enables that feature by default, using zbus's async-io backend.
Fixture transport choices do not select a GUI process's D-Bus runtime.

The broker authenticates the compositor before transferring a writable FD. It
sends a nonce probe and verifies kernel `SCM_CREDENTIALS` against the compositor
PID and start time from the admitted capture. The connected datagram channel also
checks those credentials on the final receipt. A substituted socket pathname
cannot receive browser write authority merely by returning matching JSON.
