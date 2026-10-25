# Handoff adapters

Type: task
Status: needs-triage
Blocked by: 04

Adapters for the browser (tab URL through Browser Control), kitty (`kitty @ ls`: directory and foreground program, with remote control secured as the spec's "Terminal remote control" section requires), nvim (file, line and column over its RPC server), yazi (directory) and the Dojo agent chat. Each reads state on the sending node and recreates it on the peer; each says what did not carry over.
