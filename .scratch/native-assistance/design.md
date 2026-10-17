# Native evidence design

## Decision

Use GTK7 surface accessibility annotations as the first exact-window binding.
The compositor resolves the requested mapped surface and returns its current
endpoint and annotation generation. The service proves process generation and
AT-SPI unique-owner credentials, walks only that root, then rechecks the native
target before publishing. Observations remain read-only.

Applications without this annotation retain application-scoped observations.
Their nodes cannot satisfy exact-window element verification. A supplied but
invalid annotation is a refusal/unavailable observation, not a reason to switch
to a different root.

## Caller flow

1. `AccessibilityTarget { window_id }` obtains the selected surface's endpoint.
2. `observe_window(request, root)` performs the existing bounded AT-SPI walk.
   When no endpoint exists, `observe(request)` preserves application scope.
3. The service repeats `AccessibilityTarget` and requires equality, plus an
   unchanged Linux process generation, before returning evidence.
4. Verification trusts element evidence only from an available, bound tree.
   Missing/truncated trees preserve unknown outcomes.

## Synthesis

Two independent designs considered protocol-bound window reads and improved
application-only observations. The protocol design won on exact-window
correctness and usefulness to supported stock GTK clients. The independent
review agreed. Preserve the second design's honest application fallback and
explicit scope. Keep existing serialized scope fields to avoid an unrelated
contract migration; do not introduce unreachable future capability variants.

Reject title, geometry, PID uniqueness and synthetic XID matching. These do not
prove the relationship between a native surface and an accessibility root.
Defer draft annotation protocols with no proven installed toolkit support.

GTK7 includes earlier GTK shell semantics. The implementation must preserve
surface offsets and existing activation policy before advertising that version.
The acceptance gate is two same-process windows with distinct accessibility
roots in nested Niri. Source presence alone does not satisfy that gate.

## Source identity

GTK 4.22.4, commit `7f99ab1a26408b6499a18f353f081e3c0598ea5c`,
supplies the protocol XML and toolkit emission behavior. CUA remains pinned at
`88aff7e5424e9ed45b93ded85eb4111472ca5190`; its bounded traversal is reused.
The new Niri adapter replaces CUA's ambiguous Wayland window correlation.

## Diagnostic changes

Pi remains the canonical history. Its trace projection includes actual native,
Driver, perception, browser-control and bounded-step tool names. Receipts retain
the originating assistant entry ID. Tool errors preserve unknown effects unless
an explicit receipt establishes refusal. Verification evidence is separate from
input delivery. Export analysis never executes recorded calls.

## Semantic action extension

The exact-window design remains the authority. A generic annotation consumer
accepts the pinned MR493 protocol, and toolkit producers publish the actual
window/root pair. GTK and generic annotations must agree if both are present.
Independent annotation generations, process start time, bus GUID and unique
owner identify the observation. An actor-owned input generation invalidates
references across sessions when human activity or agent input occurs.

The service issues bounded, one-use references belonging to one connection.
It rewalks the same root and rechecks capability and identity before preparing
one typed request. zbus authenticates, serializes and receives replies. Its
write half is sealed before transferring the sole commit descriptor to Niri.
Niri accepts this handoff only from the pinned immutable Dojo agent executable
owning the native peer. It checks ownership and identity on every writable
notification, then makes one nonblocking kernel send. Partial bytes are never
completed or retried. A typed correlated application reply can retire the
pending barrier. UI success still needs observation.

A separate application-scoped action design was considered. It requires a
complete process-generation client fence. It remains disabled until that
membership can be proved. Application observations never become exact-window
references. Arbitrary toolkit/runtime coverage requires producer support;
consumer enumeration cannot manufacture the missing relationship.

## Positive evidence and incomplete coverage

Completeness governs absence and outcome verification. A positive, nonprivate
node from an incomplete or query-filtered exact-root walk can receive a ref.
The service rewalks that same root and verifies the node and capabilities before
dispatch. This permits a public button beside a password field without treating
omitted private subtrees as absent. Application scope never issues refs.

Unknown semantic effects fence the whole exact native process generation,
including sibling windows and other native connections from that process. The
service also blocks a new operation while its prior operation remains pending.
Pi retains its existing conversation-wide external-effect marker. A matching
late reply retires the barrier while retaining the earlier unknown classification.
There is no new approval store or scheduler.
