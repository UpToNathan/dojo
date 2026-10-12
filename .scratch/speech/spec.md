# Speech input

Voice is the input to Dojo's agent. Responses appear visually, without spoken
output. Local transcription will feed the T3 backend using the user's existing
agent subscriptions.

## Shortcuts

- Copilot emits Super_L, Shift_L and keycode 201. The active Niri keymap
  maps that physical key to F23 at level zero and XF86Assistant with those
  modifiers. Niri matches the base symbol, so bind Mod+Shift+F23. Verified
  against `/tmp/dojo-copilot-recheck-keymap` and a successful live press.
- Super+Shift+P is the fallback for the Kinesis keyboard.
- Super+Shift+Space opens the existing Dojo command palette.

The Copilot and fallback shortcuts hold on-demand native capture and local
Hex transcription. Transcripts are sent immediately to T3 scratch conversations,
using the configured default model and existing subscription credentials.

Interactions within a session share one T3 conversation. The session stays open
while recording or running an agent and for 30 seconds after returning to idle.
Follow-ups within that interval append to the same conversation and visual stack.
After 30 idle seconds, Dojo settles the thread and the messages expire. The next
interaction starts a new conversation. Users never need to say "new chat."

The host audio stack follows the ArchWiki PipeWire guide: PipeWire audio,
WirePlumber, ALSA routing and PulseAudio compatibility. Run
`scripts/setup-audio.sh` to install the Arch packages and activate the user units.
Audio infrastructure is shared by desktop applications. The native speech worker
preloads its verified model at graphical-session startup and retains it
across recordings for the session. The microphone opens only during a recording.
Requests use explicit zero reasoning for Codex. Application-open requests use
`desktop_launch` and require a verified new Niri window.

## Agent context

Before dispatch, Dojo copies `tools/speech/assistant.md` into the scratch
workspace's `AGENTS.md`. This operating brief explains Dojo, voice invocation,
desktop assistance, tool capabilities and limits, and verified completion.
It is workspace instruction context, not a provider-level system prompt.

## Desktop handoff and dismissal

Releasing push-to-talk yields human input ownership without moving the human
cursor or keyboard focus. Held keys, pointer grabs and suppressed buttons block
the handoff. Human input takes ownership back immediately. Clicking an individual
message dismisses it while preserving the rest of the conversation stack.
