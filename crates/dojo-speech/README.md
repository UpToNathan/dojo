# Dojo speech

Local Linux capture and transcription extracted from
[Hex 2.1.25](https://github.com/anomalyco/hex/tree/104a8ac1eb87c9f393fc4ee2d434f1df30b10d1a).
This package has its own manifest and lockfile and compiles independently of
Hex's desktop app, GPUI, GTK and agent integrations.

## Build and use

From the Dojo repository root:

```sh
devenv shell -- env CARGO_TARGET_DIR="$PWD/target" CARGO_BUILD_JOBS=4 \
  cargo build --locked --manifest-path crates/dojo-speech/Cargo.toml

devenv shell -- target/debug/dojo-speech models
devenv shell -- target/debug/dojo-speech install
devenv shell -- target/debug/dojo-speech transcribe recording.wav
devenv shell -- target/debug/dojo-speech serve
devenv shell -- target/debug/dojo-speech record --seconds 5
```

The voice service does not need `target/debug`. Build release binaries and run
`devenv shell -- scripts/install-helpers.sh` to copy `dojo-speech` and the
computer recording and verification helpers to `~/.local/share/dojo/bin`, with
their Nix libraries GC-rooted. The service looks for `DOJO_SPEECH_EXE` (or
`DOJO_COMPUTER_RECORDING_EXE`, `DOJO_COMPUTER_VERIFICATION_EXE`), then that
directory, then `target/release` and `target/debug`.

The same script installs `dojo-voice-ctl`, a std-only client (`crates/dojo-voice-ctl`) that
`tools/speech/dojo-voice` runs first for `begin`, `finish`, `cancel`, `toggle` and `status`.
A keypress then takes about 5 ms instead of the 250 ms Node needs to load the whole service.
It exits 75 when the full client is needed (the service is not running, or another action),
and the wrapper falls through to Node, which also starts the service on demand.

Only `install` downloads models. `--model` and `--language` select from Hex's
pinned GGUF catalog; defaults are `parakeet_unified_en` and `en`. The model cache
is `$XDG_DATA_HOME/dojo/speech/models`, normally
`~/.local/share/dojo/speech/models`. `DOJO_SPEECH_DATA_DIR` can specify another
absolute speech data root. The installer requires `curl`.

Vulkan is enabled by default. Build with `--no-default-features` for CPU only.
Devenv supplies ALSA, OpenBLAS, CMake and Vulkan tools. GPU inference also needs
an installed Vulkan driver; the engine chooses an available backend.

The model list reports file presence and expected size. Installation verifies
the checksum, and loading checks SHA-256 again. WAV input is bounded to
30 minutes and 28.8 million input/normalized samples, with validated rates,
channels, integer scaling and finite samples. Microphone capture has no
duration cap. `record --device` selects a microphone by name substring.

## Library and lifetime

```rust,no_run
use dojo_speech::{Cancellation, Capture, Selection, Transcriber};

let cancel = Cancellation::default();
let mut engine = Transcriber::load(&Selection::default(), &cancel)?;
let recording = Capture::start(None)?;
// Wait for the user's stop action.
let audio = recording.finish()?;
let text = engine.transcribe(audio, &cancel)?;
```

Capture opens only on start; finish, cancel and drop close it. Cold capture has
no pre-roll. Loading and inference block and belong outside GPUI's UI thread.
Drop the transcriber to release model memory. Cancellation suppresses canceled
results but cannot interrupt the native inference call. Ctrl+C cancels CLI
installation or capture and suppresses an inference result.

This package does not bind shortcuts, paste text, display a UI or submit to T3.

## Source provenance

`vendor/hex` is a Git subtree pinned to
`104a8ac1eb87c9f393fc4ee2d434f1df30b10d1a`. Private imports are `audio.rs`,
`dictation.rs`, `spoken_text.rs`, `transcription_models.rs`, `gguf_session.rs`
and `linux_transcriber.rs`. The application-path adapter uses Dojo's cache.
The WAV adapter follows Hex's `transcription_service.rs` validation/scaling
rather than its Linux CLI's WAV decoder.

The subtree has one local compilation correction: pass `&path` to
`validate_gguf_artifact` in `linux_transcriber.rs`. After updating it, review the six
imports and pinned `transcribe-cpp` version, then build this package.
Hex's MIT notice is preserved in [LICENSE](LICENSE). Native notices remain in
[Hex's notices](../../vendor/hex/THIRD_PARTY_NOTICES.md). Model weights have
separate upstream licenses.

## Persistent capture worker

`serve` reads newline-delimited JSON commands from stdin with `action`
(`begin`, `finish`, or `cancel`) and a string `id`. Events on stdout carry
the same id. `begin` opens the microphone; `finish` closes it and emits
`transcribing`, then `transcript`. `cancel` closes it without transcription.
Errors emit an `error` event with a `message`. EOF closes capture and unloads
the model. Startup loads and prewarms the model, then emits `ready`;
recordings reuse it. Transcript events include `inferenceMs`, excluding loading.

Run `devenv shell -- python3 scripts/check-speech-worker.py` to capture two
short recordings and verify model reuse and cancellation. This opens the
microphone, but does not submit transcripts to T3.

The desktop keeps `dojo-voice.service` running with the graphical session and
preloads speech during login. Home Manager installs and enables the tracked
[unit](../../tools/speech/dojo-voice.service). The microphone remains closed
until a capture command. Stop the user service to release the model.
