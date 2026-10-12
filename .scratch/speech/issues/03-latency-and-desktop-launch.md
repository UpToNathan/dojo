# Voice latency and desktop application launch

Type: task
Status: resolved

Preload the local model at graphical-session startup and retain it for the
session. Set Codex voice requests to low reasoning explicitly. Provide a
real application launch tool that shares the launcher implementation and
verifies a new Niri window from the requested application. Browser requests
must open a new standalone window, including when a browser is already open.
Verify through native speech, MCP, and the voice dispatch path.

## Answer

Speech preloads at session startup, stays resident, and opens the microphone
only during capture. Optimized SHA-256 in development builds removes the
unoptimized checksum work. Codex voice selections now explicitly use low
reasoning. Desktop launching shares Dojo's existing launcher, reads the
graphical environment from systemd, and verifies a matching new Niri window.

Verified two native recordings with one model load and cancellation; 40 speech,
38 library, and 5 adapter tests passed. The MCP launch check passed with display
and data-directory variables removed. Two full voice-dispatch browser requests
opened verified Helium windows using Luna with low reasoning. The resident
service is installed, enabled for graphical login, and ready. Repro commands
are scripts/check-speech-worker.py, scripts/check-desktop-launch.py, and
scripts/check-voice-browser.py, run through devenv.

## Reasoning correction

The user requested zero reasoning. The Codex catalog lists only low through
max for Luna, but T3 accepts `reasoningEffort: none`. Voice now sends `none`
explicitly. A real Luna voice-dispatch request completed successfully, and the
thread configuration reports `none`. The end-to-end check requires `none`.
