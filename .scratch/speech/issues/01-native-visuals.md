# Native speech state and agent feedback

Type: task
Status: resolved

Replace the visual prototype with native shell presentation.

## Agreed behavior

- Arch is the default settled icon.
- Listening uses the microphone; transcription uses `nf-md-typewriter`.
- Sent uses the paper plane, including follow-up messages.
- Working uses the robot. All glyphs use the same size and container.
- Responses accompany work as brief floating speech bubbles that fade.
- Completion restores Arch immediately. Bubble lifetime is independent.
- Speech bubbles pass pointer input through to the underlying application.

Use the existing shell control socket for phase, transcript and response events.
Expose agent feedback through dojo-agent's MCP interface. Capture shortcut
activation and automatic T3 conversation routing are separate integration work.

## Answer

Implemented the native GPUI presentation with validated local socket events and
CLI controls. Added MCP desktop_feedback; actions show working and release or
explicit task completion restores Arch. Response opacity and reading lifetime
remain independent of task completion.

Built both dojo and dojo-agent through devenv. State and control tests pass.
Live autostart reload succeeded; captured all states and response completion in
/tmp/dojo-speech-native. The settled icon crop is pixel-identical before and
after the sequence. Actual MCP discovery, feedback delivery, invalid-response
rejection and clean EOF exit passed without creating a cursor seat. Pointer
pass-through retains the existing input region; a live click check was not run.
