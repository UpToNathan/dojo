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

## Agent runtime and voice input

`dojo-voice.service` is the Dojo agent runtime: the Pi conversation, memory, sign-in, the model
catalog and the command socket (`$XDG_RUNTIME_DIR/dojo-voice.sock`) that the model picker and
`tools/speech/dojo-voice` use. It starts with the graphical session on every host, with or
without a microphone, so `send`, `models`, `select-model`, `history`, `new` and the sign-in
commands always work. Voice is one input to it: microphone capture and the prewarmed
transcription model.

Decision: one runtime process whose speech worker is started only when a microphone exists,
rather than a second service that forwards transcripts over the socket. The speech model already
lives in its own native process (`dojo-speech`), so a mic-less runtime never starts it and holds
only the Node runtime. A second service would have to share capture state with the runtime (a
response must wait for an open capture, the Niri input handoff, the listening and transcribing
phases), which a socket round trip per event would only make fragile. The socket and its
commands did not change, except for the new `voice-input` command below. The names
`dojo-voice.service` and `dojo-voice.sock` stayed so installed hosts, the Rust clients and the
keybindings keep working; the name now covers the runtime and its voice input.

`dojo-voice-mic.service` watches PipeWire (`pactl subscribe`) and, as `tools/speech/dojo-mic-present`
changes its answer, runs `dojo-voice voice-input on|off`. `on` loads the speech worker; `off`
cancels any capture and ends the worker, releasing the model. Only the speech worker follows the
microphone; the runtime and its conversation are never restarted by it. The runtime also checks
once when it starts and again when a voice action finds no microphone, so a missed event heals
on the next keypress. Output monitors and sources whose active port is reported "not available"
(an unplugged jack) do not count as microphones; sources without ports, such as USB headsets, do.
With no microphone the voice keybindings print "No microphone is available" and the rest of the
runtime keeps working.

On a host that upgrades from the earlier layout, run `systemctl --user restart dojo-voice.service`
once (or log in again) so the runtime that is already running picks up the new code; Home Manager
enables and starts the unit but does not restart a running one.

A Bluetooth headset only has a microphone while it is in its headset (HFP) profile; in the
music profile it counts as no microphone.
