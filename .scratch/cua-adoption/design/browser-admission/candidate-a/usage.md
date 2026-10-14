# Usage: compositor-owned lazy service activation

The operator enables the Dojo service in Niri's existing external-admission configuration and pins an immutable installed service artifact (identity mechanism remains a deployment decision). Niri starts no service at compositor startup. The first Dojo service request asks the compositor to activate its configured service; Niri launches one generation and returns a generation-bound local endpoint. Niri gives that process a private registration channel and retains the other endpoint as the revocation authority.

The Pi extension remains the caller-facing boundary. It commits its unsafe reservation and durable unknown barrier before transport, then the existing MCP adapter asks the Rust service to navigate. A service restart does not replay that call or inherit the old generation's captures.

```js
// Pi extension: existing durable reservation remains before MCP transport.
const reservation = await reserveUnsafeNavigation(callId, target);
const result = await mcp.browserNavigate({
  captureId: target.captureId,
  url: target.url,
  intentId: reservation.id,
});
return presentOutcome(result); // exact outcome UI semantics remain open
```

```rust
// MCP adapter: same typed transport owner; no direct CDP call.
let service = ActiveSession::connect_or_activate(compositor_socket, policy).await?;
service.browser_navigate(BrowserNavigateRequest {
    capture_id,
    url,
    pi_intent: PiIntentId::parse(intent_id)?,
}).await
```

```rust
// Service: current read episode and capture are consumed once; compositor
// retains the sole pending write and is the final admission point.
let receipt = service.navigate(request, cancellation).await?;
match receipt {
    NavigationReceipt::RefusedBeforeWrite(reason) => release_pi_barrier_only_via_existing_accepted_mechanism(reason),
    NavigationReceipt::Unknown(correlation) => keep_pi_barrier(correlation),
}
```

Niri owns service lifetime. It starts the configured executable only on demand, binds the child PID/start and pidfd to a fresh activation generation, and passes one private control capability over an inherited socketpair. Service requests and each Niri IPC agent connection are bound to that activation generation. Agent seats retain kernel-authenticated connection credentials and peer IDs; the writer request must name a peer owned by the same registered process generation. Neither a seat nor same-UID status activates a broker.

The service keeps its existing process-local session, capture/read episode, browser, and controller ownership. Its broker still authenticates Niri, upgrades only `/devtools/page/{target}`, verifies the exact page target against the current read episode and exact native window/process generation, then transfers exactly one FD. Niri validates the activation, peer, session, target, endpoint, geometry, revision, and coordination state before installing its writer. Immediately before one nonblocking frame write, the actor repeats the live checks and constructs the sole allowed `Page.navigate` frame.
