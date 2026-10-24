# Run the agent runtime without a microphone

Status: needs-triage

Today the agent runtime lives inside `dojo-voice`, which starts only while a microphone exists (`dojo-voice-mic.service`, `ExecCondition` `dojo-mic-present`). The Agent panel and the model picker fail with "Could not connect to Dojo voice" without it. Split the runtime from the voice input so the runtime runs on every host and voice is one input to it. Touches the unit files, the Agent panel, the model picker and the socket they share. Chosen over starting the service on demand.

Blocked by: 01

## Comments
