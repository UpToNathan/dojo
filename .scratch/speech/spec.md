# Speech input

Voice is the input to Dojo's agent. Responses appear visually, without spoken
output. Local transcription feeds Dojo's Pi runtime using the user's existing
agent subscriptions.

## Shortcuts

- Copilot emits Super_L, Shift_L and keycode 201. The active Niri keymap
  maps that physical key to F23 at level zero and XF86Assistant with those
  modifiers. Niri matches the base symbol, so bind Mod+Shift+F23. Verified
  against `/tmp/dojo-copilot-recheck-keymap` and a successful live press.
- Super+Shift+P is the fallback for the Kinesis keyboard.
- Super+Space opens the existing Dojo command palette.

The Copilot and fallback shortcuts hold on-demand native capture and local
Hex transcription. Transcripts are admitted durably to Dojo's local Pi runtime,
using Dojo's configured model and subscription authentication.

Dojo memory preserves exact requests and responses across interactions. After a
completed agent run, the next request waits for preceding summaries and starts
fresh model context with the saved memory view. One durable logical conversation
retains history. Input arriving while a run is active continues to steer that run.
The visual session remains open while recording or running, and speech bubbles
expire after 30 idle seconds. That expiry does not erase memory or close the
durable conversation.
See docs/dojo-memory.md for storage, retrieval and isolation.

The host audio stack follows the ArchWiki PipeWire guide: PipeWire audio,
WirePlumber, ALSA routing and PulseAudio compatibility. Run
`scripts/setup-audio.sh` to install the Arch packages and activate the user units.
Audio infrastructure is shared by desktop applications. The native speech worker
preloads its verified model at graphical-session startup and retains it
across recordings for the session. The microphone opens only during a recording.
OpenAI requests use low reasoning by default. Application-open requests use
`desktop_launch` and require a verified new Niri window.

## Agent context

Dojo uses the same fixed memory prompt and `tools/speech/assistant.md` operating
brief for foreground and summary calls. The brief explains Dojo, voice invocation,
desktop assistance, tool capabilities and limits, and verified completion.
They are Pi system instructions; variable memory and current requests follow.

## Desktop handoff and dismissal

Releasing push-to-talk yields human input ownership without moving the human
cursor or keyboard focus. Held keys, pointer grabs and suppressed buttons block
the handoff. Human input takes ownership back immediately. Clicking an individual
message dismisses it while preserving the rest of the conversation stack.
