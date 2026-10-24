# Windows command

Status: resolved

Blocked by: 01

List Niri windows (app, title, workspace) over the in-process `niri-ipc` socket and focus the chosen one. Most recently focused first.

## Comments

Built over niri-ipc with 250 ms timeouts. Focusing was tested against a fake Niri socket and in the nested session only.
