# Contained worker caller usage

Design only. These calls do not exist yet. The service authenticates the connection and owns the `CaptureSession`, cancellation source, and request deadline. Pi invokes one read tool and remains the only durable scheduler.

```rust
// MCP/direct socket adapters parse the same closed typed request.
let request = VisualParseRequest {
    capture_id: arguments.capture_id,
    options: VisualOptions::text_and_icons(),
};
let output = session.captures.parse_visual_regions(
    &service.perception,
    request,
    session.read_context(connection_request),
).await?;
// output.capture_id/digest/dimensions identify the already-delivered PNG.
// output.authorized == false; output.capture_consumed == false.
// All regions have interactive == false. No element token or action is minted.
```

A parse uses any registered output, crop, or exact-window capture. A window need not be actionable to inspect its pixels. The caller supplies no PNG, filename, PID, binding, worker executable, manifest, library, or artifact selection.

```rust
// Host setup, outside model/tool arguments. Opening verifies the configured
// immutable installation but starts no inference process at idle.
let perception = PerceptionBackend::open(
    installed_bundle,
    WorkerLimits::from_host_configuration(host_config)?,
)?;

// Disconnect/stop reaches the exact request signal from the connection reader,
// even while the main service loop processes another client.
connection_lifetime.cancel();
// In-flight parse kills/reaps its child and suppresses late publication.
```

```rust
// Native actions still use independent consuming admission.
let observation = session.observe(window_scope).await?;
let parsed = session.captures.parse_visual_regions(
    &perception,
    VisualParseRequest::for_capture(&observation.capture_id),
    session.read_context(connection_request),
).await?;
assert_eq!(parsed.capture_id, observation.capture_id);
// Parsing neither refreshes TTL nor consumes the capture. A later explicitly
// requested native action must independently pass current-frame/human checks.
let receipt = session.capture_action(explicit_action).await?;
```

The pinned tool and Rust method are both `parse_visual_regions`. Pi explicitly classifies it as a metadata-only read. It does not return a duplicate image block or clear Pi's retained capture marker. Replay against an expired or retired capture returns a typed refusal; it never captures a replacement implicitly.
