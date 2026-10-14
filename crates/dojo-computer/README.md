# Dojo computer authority

This Rust package integrates CUA's pinned immutable capture registry with Dojo's
named outputs, positioned windows, process-start identity and human ownership
revision. It owns storage and one-use admission, and performs no compositor,
provider, or credential I/O. The shared `dojo-agent` process supplies live
identity/geometry and executes admitted input through the independent Niri seat.

An observation retains exact output/crop/process/workspace/ownership evidence,
PNG digest, encoded dimensions and image-to-output mapping. Output and region
observations are read-only. Positioned-window captures can authorize one move,
click or scroll in delivered-image pixels. Failed static validation does not
consume a capture; admission does. The service retires all sibling captures
before attempting any effect, and never restores them after pause/uncertainty.
Release, disconnect, restart and expiry invalidate the session's references.

Capture storage expires after 30 seconds and does not create or renew an input
seat. The shared service starts lazily for capture storage, and stops when its
last seat/capture lease expires. The legacy screenshot tool still starts no
service. Visible screenshots can include occluding windows. Guarded clicks and
scrolls re-hit-test the actual mapped target inside Niri at delivery.

The baseline is not semantic UI-state verification or image redaction. An
unchanged window can contain changed controls, and a screenshot can contain
sensitive visible content. Images retain the existing Pi transcript/memory
policy. Typed rich input, complete authorization/privacy policy, native capture,
semantic snapshots and side-channel admission are later adoption tickets.

```sh
devenv shell -- cargo test --manifest-path crates/dojo-computer/Cargo.toml
```

The complete acceptance scenario uses Rust and Pi's existing Node host:

```sh
devenv shell -- cargo build --bin dojo-agent
devenv shell -- cargo build --manifest-path vendor/niri/Cargo.toml --no-default-features
devenv shell -- cargo build --manifest-path tools/agent-input/Cargo.toml --locked
devenv shell -- cargo test --test capture_desktop -- --ignored --nocapture
```

The ignored test creates a nested compositor and native Wayland fixture. It
checks real MCP observations/clicks, exact target delivery, wrong-session and
replay refusal, geometry changes, human takeover, occlusion and held-key release
on MCP loss. It also drives the real MCP tools through Pi's durable scheduler
using a scripted provider and disposable SQLite state. It checks image retention
and blocks a repeated capture action before Rust execution. No model API,
credentials, Python, live compositor restart or installed configuration is used.
