# Run the agent runtime without a microphone

Status: resolved

Today the agent runtime lives inside `dojo-voice`, which starts only while a microphone exists (`dojo-voice-mic.service`, `ExecCondition` `dojo-mic-present`). The Agent panel and the model picker fail with "Could not connect to Dojo voice" without it. Split the runtime from the voice input so the runtime runs on every host and voice is one input to it. Touches the unit files, the Agent panel, the model picker and the socket they share. Chosen over starting the service on demand.

Blocked by: 01

## Comments

Built on branch runtime-split and merged. One process; the speech worker starts only when a microphone exists, `voice-input on|off` follows hotplug, and `dojo-voice.service` starts at login everywhere. Idle runtime-only RSS about 109 MB. Existing hosts need the Home Manager activation and one restart of `dojo-voice.service`. Not verified: real microphone hotplug and a fresh-host activation.
