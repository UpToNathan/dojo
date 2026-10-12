# Voice desktop handoff and clickable message dismissal

Type: task
Status: resolved

Voice requests repeatedly pause on human ownership even when the user explicitly
asks the agent to use its cursor. Add handoff on push-to-talk release while
preserving immediate human takeover and held-input protection. Message bubbles
must accept clicks to dismiss individually without clearing other messages.

## Verification

Speech state tests pass, including individual dismissal and follow-up history.
A real agent pointer click removed the user bubble while leaving the reply visible.
Live screenshots: /tmp/dojo-dismiss/before.png and /tmp/dojo-dismiss/after.png.
All 13 compositor agent-input tests passed, including handoff, human takeover,
and refusing handoff while keys are held. The release build passed. Live nested
handoff IPC passed in /tmp/dojo-agent-nested.zpQ06W. Fullscreen regression passed
all eight edges in /tmp/dojo-agent-nested.OEo3Ng using the updated release binary.
The first fullscreen run used a stale debug binary that rejected agent-color
and displayed its config-error overlay; it was superseded by the release run.

The updated compositor is installed for the next login. The current compositor
remains running, so voice handoff does not become active until that login.
Click dismissal is already active in the running shell.
