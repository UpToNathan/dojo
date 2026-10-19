# Native speech UI

Dojo's sidebar uses Arch when settled, the microphone while listening,
`nf-md-typewriter` while transcribing, the paper plane after sending, and the
robot while the agent works. Follow-ups use the same states. Glyphs use the
bundled Victor Mono Nerd Font at 32 pixels in the same 36-pixel container.

Messages form a stack centered near the bottom of each display. Each new
transcript or response enters at the bottom and pushes older messages upward.
Starting a follow-up recording preserves existing user and agent messages.
Messages remain visible during recording, transcription and agent work. All
messages begin a shared 30-second dismissal countdown when the session becomes
idle; renewed activity pauses it. Their surfaces and borders use 20% opacity,
with the theme accent for user messages and a neutral highlight for responses.
GPUI submits measured rounded blur regions together with each rendered buffer.
Stack spacing uses shaped text heights for wrapped messages.

Completion restores Arch while the message stack remains visible. The bubbles
receive pointer input within their bounds. Clicking a bubble dismisses that
message; the rest of the desktop remains click-through.

## Agent feedback

The rebuilt `dojo-agent` MCP server exposes `desktop_feedback`:

```json
{"response":"Opened your electricity bill.","complete":true}
```

Omit `response` when completion has nothing to say. Set `complete` to false
when the response accompanies ongoing work. Desktop input actions display the
robot; explicit release restores Arch. Feedback does not create an input seat
or start the shared cursor service. Reconnect an existing MCP client to discover
the new tool after rebuilding the server.

## Local events

The resident shell accepts phase, transcript and response events through its
existing `$XDG_RUNTIME_DIR/dojo.sock`. Each connection sends one JSON line and
receives `queued` when accepted. This acknowledges delivery to the UI queue.

```json
{"event":"phase","value":"listening"}
{"event":"phase","value":"transcribing"}
{"event":"transcript","value":"Find my latest invoice."}
{"event":"phase","value":"sent"}
{"event":"phase","value":"working"}
{"event":"response","value":"Opened your electricity bill."}
{"event":"phase","value":"settled"}
```

The CLI provides the same interface inside devenv:

```sh
target/debug/dojo --speech-state transcribing
target/debug/dojo --speech-transcript 'Find my latest invoice.'
target/debug/dojo --speech-response 'Opened your electricity bill.'
target/debug/dojo --speech-state settled
```

Text must be nonempty and at most 1000 characters. Unknown phases and malformed
events are rejected. No response phase exists. Receiving text never changes the
phase automatically.

The native presentation, on-demand Hex capture, local transcription and T3
scratch-chat routing are connected through `tools/speech/dojo-voice`. Copilot or
Super+Shift+P records while held; releasing it stops capture and sends the
transcript. Super+Shift+Escape cancels capture. Listening begins only after the
native worker opens the microphone. Inactive conversations are settled in T3.

On Arch, run `scripts/setup-audio.sh` to install PipeWire, its ALSA and PulseAudio
compatibility layers, and WirePlumber. The devenv environment uses the Nix
PipeWire ALSA plugin with the host ALSA configuration, so capture follows the
system default source without mixing host and Nix C libraries.

## Microphone presence

The voice service, with its prewarmed transcription model, runs only while a microphone can
record. `dojo-voice-mic.service` watches PipeWire (`pactl subscribe`) and starts or stops
`dojo-voice.service` as `tools/speech/dojo-mic-present` changes its answer: output monitors and
sources whose active port is reported "not available" (an unplugged jack) do not count; sources
without ports, such as USB headsets, do. The service's `ExecCondition` runs the same check.
With no microphone the voice keybindings print "No microphone is available" and start nothing,
and the model picker has no service to reach.

A Bluetooth headset only has a microphone while it is in its headset (HFP) profile; in the
music profile it counts as no microphone.
