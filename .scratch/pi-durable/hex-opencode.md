# Hex's OpenCode integration and Dojo

Research date: 2026-10-11. Inspected upstream Hex commit `104a8ac1eb87c9f393fc4ee2d434f1df30b10d1a`, dated October 7, package `voice-control` 2.1.25. This is source inspection, with no installation, implementation or tests. The current Dojo checkout has no Hex subtree; these findings describe upstream code. [Commit](https://github.com/anomalyco/hex/commit/104a8ac1eb87c9f393fc4ee2d434f1df30b10d1a), [manifest][cargo].

## What Voice Action actually does

Ordinary dictation transcribes locally and pastes text. Optional OpenCode post-processing rewrites that transcript using the configured model. Voice Action is another optional text-generation path: the user dictates an instruction, Hex adds selected text when available plus foreground application and Brave hostname, asks for exact paste-ready text, and pastes the resulting text at current focus. It does not expose a computer-use agent or an action-planning loop. A failed rewrite preserves the corrected local transcript; failed Voice Action pastes nothing. [README][readme], [processor prompts and execution][processor].

Continuous voice commands are a separate macOS feature. Local recognition matches configured phrases and context to built-in actions or custom handlers. Actions include opening applications, URLs or paths, typing and keystrokes. Custom TypeScript commands use a Bun host and run local code. This path is independent of OpenCode generation. [README][readme], [command actions][commands], [personal command host][personal].

## Transport and lifecycle

Hex requires a separately installed OpenCode V2 `opencode2`, with an optional `VOICE_CONTROL_OPENCODE_CLI` executable override. Hex neither bundles/pins the runtime nor owns/restarts/updates its managed service. Its CLI discovery call, `opencode2 api get /api/info`, lets OpenCode discover/start its own service. Exact missing-route errors permit older info/status/health routes. `opencode2 debug paths` supplies the state directory. Hex reads the active server's matching PID/version registration for endpoint and password. [Compatibility contract][compat], [processor][processor].

Actual catalog and generation requests use `/usr/bin/curl` against authenticated loopback HTTP. Basic auth credentials and request bodies go through curl configuration on stdin, rather than argv or temporary files. Hex rejects non-loopback endpoints, ignores curlrc/proxies/redirects, bounds responses and shares deadlines/cancellation across discovery and generation. Registration files must be bounded owner-only regular files, opened without following symlinks. [Processor][processor].

Catalog discovery reads `/api/model` and `/api/model/default`, with `x-opencode-directory` for Hex's application-support workspace. Generation posts to `/api/experimental/generate`, falling back to `/api/generate` only on HTTP 404. Its request is just `prompt` and optional `model`; the result is just `data.text`. The compatibility document explicitly calls it **stateless** and notes it uses the server's base configuration, rather than the catalog request's directory. No session IDs, conversation history, tool declarations, action permissions or MCP calls are passed through this integration. The generation route's complete server implementation was not audited here; the supported Hex contract is one-shot text generation, not an agent tools interface. [Compatibility contract][compat], [Request/Response definitions and generation][processor].

The OpenCode compatibility baseline records August 26 versions and prior tests. It expressly warns that this is a dated snapshot, not the newest CLI available. Hex's current source is newer, but that does not prove a newer provider-backed compatibility test. [Compatibility contract][compat].

## Linux limitations

`main.rs` gates `dictation_processor`, `commands`, `personal_commands`, `recognition`, context and local API modules to macOS. Linux has a separate runtime supporting local dictation and GPUI settings. Its docs exclude voice commands, application/browser context and meetings. Therefore upstream's OpenCode Voice Action/post-processing path is not ready to import into Dojo on Linux. [Main module gates][main], [Linux guide][linux].

Linux desktop dictation normally runs in a user systemd service, with a separate settings client and private Unix socket. Foreground service operation under another supervisor is documented. Native input depends on clipboard, virtual keyboard and layer-shell protocols plus input-device access. Dojo's existing desktop coordinator is a better place to integrate agent actions than copying Hex's input/paste route. [Linux guide][linux].

The TypeScript SDK documents an app-owned recording model: supply WAV bytes, Hex prepares models and returns a transcript, with an explicit helper executable. It documents `service --embedded`, model reuse and helper cleanup on cancellation. However, the inspected source implements that embedded option in the macOS CLI. Linux's separate `Command::Service` has no embedded option. The SDK description therefore does not establish a working Linux embedded-helper distribution; a Linux implementation/port is required before relying on this route in Dojo. [SDK guide][sdk], [main][main], [Linux CLI][linuxcli].

## Rust reuse

The Cargo manifest has one `voice-control` package, no workspace of independent internal crates, and the source has no `src/lib.rs`. The code is primarily modules of the application binary. A git subtree makes source available, but does not make those modules selectively importable as Rust library crates. Reuse requires extracting suitable modules into a library, adding an upstream-compatible library target, or integrating the underlying transcription dependency `transcribe-cpp`. Its optional `transcribe-rs` dependency is for the ONNX benchmark feature. [Manifest][cargo], [source tree](https://github.com/anomalyco/hex/tree/104a8ac1eb87c9f393fc4ee2d434f1df30b10d1a/src).

## Recommendation for Dojo and pi-durable

Borrow the separation of capture, local transcription and optional processing. For our voice-only input with visual responses, send the transcript into a pi-durable conversation and let its registered desktop tools use the existing coordination service. Dojo should own presentation and retain persistent task/conversation state in pi-durable. Hex's OpenCode adapter supplies neither durable state nor computer-use tooling.

A rewriting stage should be optional and justified by transcript quality, since it adds another model call and may change the user's instruction. Ordinary local transcription needs no OpenCode runtime. If adopting Hex source, first define the Linux transcription library/helper boundary and its cancellation/lifecycle behavior. A subtree can preserve source provenance after that design, but adding the entire app does not itself solve selective library reuse.

[readme]: https://github.com/anomalyco/hex/blob/104a8ac1eb87c9f393fc4ee2d434f1df30b10d1a/README.md
[cargo]: https://github.com/anomalyco/hex/blob/104a8ac1eb87c9f393fc4ee2d434f1df30b10d1a/Cargo.toml
[processor]: https://github.com/anomalyco/hex/blob/104a8ac1eb87c9f393fc4ee2d434f1df30b10d1a/src/dictation_processor.rs
[commands]: https://github.com/anomalyco/hex/blob/104a8ac1eb87c9f393fc4ee2d434f1df30b10d1a/src/commands.rs
[personal]: https://github.com/anomalyco/hex/blob/104a8ac1eb87c9f393fc4ee2d434f1df30b10d1a/src/personal_commands.rs
[compat]: https://github.com/anomalyco/hex/blob/104a8ac1eb87c9f393fc4ee2d434f1df30b10d1a/docs/opencode-compatibility.md
[main]: https://github.com/anomalyco/hex/blob/104a8ac1eb87c9f393fc4ee2d434f1df30b10d1a/src/main.rs
[linux]: https://github.com/anomalyco/hex/blob/104a8ac1eb87c9f393fc4ee2d434f1df30b10d1a/docs/linux.md
[linuxcli]: https://github.com/anomalyco/hex/blob/104a8ac1eb87c9f393fc4ee2d434f1df30b10d1a/src/linux.rs
[sdk]: https://github.com/anomalyco/hex/blob/104a8ac1eb87c9f393fc4ee2d434f1df30b10d1a/sdk/typescript/README.md
