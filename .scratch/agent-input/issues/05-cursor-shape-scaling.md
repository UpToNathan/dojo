# Chromium cursor becomes tiny at fractional scale

Type: bug
Status: resolved

## Symptom

The human cursor is tiny throughout Helium, including browser chrome, on the
175% display. The agent input mode removed the cursor-shape protocol global.

## Reproduction

Two isolated nested Niri sessions with the installed Helium and the same 175%
output scale differ only in DOJO_AGENT_INPUT. Without cursor shapes, Helium sends
a 10 by 16 pixel cursor buffer with buffer scale 2. With cursor shapes, it requests
the default named cursor and Niri renders it at the configured size.

Evidence: /tmp/dojo-cursor-size/agent-1/cursor.png and agent-0/cursor.png,
with their cursor-trace.txt files. The reproducer is
/tmp/dojo-cursor-size/reproduce.py.

## Fix

Keep cursor shapes available in agent mode. The manager resolves an agent
pointer through that controller's temporary seat state before delegating to
Smithay. The primary pointer uses normal dispatch. Requests for retired pointers
create inert devices, and Smithay devices hold only weak seat references.
The existing cursor_image seat check prevents agent seats changing the human
cursor. Virtual keyboards and input methods retain their existing restrictions.

## Verification

The old release failed the automated Chrome protocol check with "browser fell
back to a client-rendered cursor". The new release passed in both normal and
agent modes at scale 1.75. A fresh screenshot shows the normal sized arrow.
Evidence: /tmp/dojo-cursor-size-fixed/agent-1/cursor.png and cursor-trace.txt;
command: devenv shell -- python3 /tmp/dojo-cursor-size/verify-fixed.py.

The existing protocol-global check passed. The release MCP/GPUI scenario passed
with agent pointer binding, input delivery and seat cleanup; evidence is
/tmp/dojo-agent-nested.YofGrC. The release build and installed configuration
validation passed. scripts/install-niri-fork.sh installed
/home/nathan/.local/share/dojo/niri/1ba936470bd31e9d for the next login.
The current desktop still uses the prior compositor until logout and login.
