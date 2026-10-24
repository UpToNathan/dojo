# Ask the Dojo agent from the launcher

Status: resolved

Blocked by: 01, 02

`? <text>` sends the text to the agent runtime and opens the Agent panel on the same output. A bare `?` opens the panel. No speech and no microphone involved.

## Comments

Built: `? text` sends the message over the agent runtime socket. There is no separate Agent panel in the shell; the runtime's speech bubbles show the message and the reply. A bare `?` shows a hint.
