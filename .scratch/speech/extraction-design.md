# Hex speech extraction

## Caller and ownership

```rust
let cancel = dojo_speech::Cancellation::default();
let selection = dojo_speech::Selection::default();
let mut engine = dojo_speech::Transcriber::load(&selection, &cancel)?;
let capture = dojo_speech::Capture::start(None)?;
// Wait for the user's stop action.
let clip = capture.finish()?;
let text = engine.transcribe(clip, &cancel)?;
```

Installation is explicit through `install_model(&selection, &cancel)`.
File callers use `AudioClip::from_wav(reader)` instead of microphone capture.
The independent speech package imports six private modules from the pinned
Hex subtree, with a one-line path-borrow compilation correction. Its path
adapter uses Dojo's data directory. No upstream engine
or desktop types appear in the public API.

Capture's worker opens CPAL and reports readiness after stream startup, then
continuously drains audio. Finish closes the microphone before normalization;
cancel and drop close it and discard audio. There is no idle microphone or
pre-roll. The eventual shortcut controller owns hold duration and recording UX.

Loading verifies artifact size and SHA-256. Hex handles GGUF validation,
prewarming, chunking and session scratch cleanup. Model memory remains until
the transcriber is dropped. Cancellation suppresses results; native inference
finishes before returning a cancellation error.

## Synthesis

Candidate A proposed a speech library and CLI. B proposed a supervised helper
with a private process protocol and forced native abort. Both scored 21/25
across provenance, isolated dependencies, audio integrity, lifetime and build
feasibility in the independent comparison. A is the base because extraction
needs a directly usable library. We retain B's strict WAV conversion, pre-load
SHA verification and separate microphone/model lifetimes. Hex's curl installer
remains the single artifact installer; A's HTTP rewrite is omitted.

The CLI consumes the same library. A helper can later use it if Dojo needs
native process isolation. This extraction adds no persistent speech service.

## Delivery boundary

Model installation, WAV transcription and timed capture are implemented in the
library and CLI. Copilot/Kinesis control, visual responses and T3 submission
are subsequent work. A successful build establishes no live transcription
quality or microphone behavior.

## Build evidence

The Vulkan-enabled package built with `cargo build --locked` inside devenv.
Its CLI successfully listed the seven pinned GGUF artifacts without installing
a model or opening the microphone. Cargo's resolved dependency graph contains
no GPUI, GTK, OpenCode or `voice-control` package. Live capture and inference
have not been exercised; no tests were run.
