# Processes and Services commands

Status: resolved

Blocked by: 01

Processes: list running processes by CPU and memory, grouped by program, and send a signal with confirmation. Services: systemd user units with start, stop, restart, reset-failed and recent log lines. Inspired by pitchfork and process-compose; a dev-services adapter is not part of this ticket.

## Comments

Built. Stopping a program or killing it takes a second press; the desktop session's own units cannot be stopped from here. Signals were tested on spawned children and fake runners, service actions against fake runners.
