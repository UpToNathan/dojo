# Notify when the agent pauses

Status: ready-for-agent
Type: task
Blocked by: 01

In `src/bin/dojo-agent.rs`, at the `InputPaused` sites, send one notice per pause using `dojo::notify`.
