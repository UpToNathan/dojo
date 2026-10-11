# Replace the custom arrow with the Nerd Font pointer

Type: task
Status: resolved

Implement [the spec](../spec.md). Retain a reproducible font-to-asset command, test buffer dimensions and alpha handling, and inspect the actual rendered cursor.

## Answer

Replaced the hand-drawn arrow with the actual `fa-mouse_pointer` glyph at U+F245 from the bundled Victor Mono Nerd Font. A reproducible exporter produces a transparent cyan 68x96 PNG; buffer scale 4 gives a 17x24 logical pixel icon. Niri embeds the asset and decodes it only at seat creation. No installed font, Fontconfig setup or new runtime dependency is needed. The fork includes asset provenance and the source font license.

All 210 Niri library tests pass, including the new alpha/premultiplied-color test. The default-feature release build passes the MCP/GPUI lifecycle scenario with output scale 1.75 (`/tmp/dojo-agent-nested.nC0DuH`). Visually inspected the actual rendered cursor in `idle-cursor-visible.png`; its region returns exactly to the baseline after idle shutdown. Click/key delivery, worker restart and disconnect cleanup remain verified.

Installed the release build for the next Niri login, retaining the previous binary. The active compositor remains PID 1112, so its cursor remains the old graphic until logout/login. The MCP adapter and shell need no rebuild for this compositor change.
