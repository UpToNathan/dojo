# Send and receive

Type: task
Status: needs-triage

`dojo-mesh send --window ID --to PEER [--copy-missing]`: read the window from Niri, ask its adapter what it shows, check the peer has the files, send, and close the window once the peer opened it. `dojo-mesh receive` on the peer opens only adapter-described things in the operator's session (through Niri's IPC). Exit statuses and JSON output the panel can show. Audited.
