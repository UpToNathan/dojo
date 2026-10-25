# Agent peer access

Type: task
Status: resolved
Blocked by: 02

Give the agent peer tools built on the shell layer, with an audit log, provenance and a pause switch. Hand desktop work to the peer's agent as a peer task.

## Answer

The agent has `mesh_status`, `mesh_find`, `mesh_exec`, `mesh_copy` and `mesh_task` (`tools/speech/mesh-extension.mjs`), all calling `dojo-mesh` marked as the agent with the conversation as the session. Access is permissive: no per-call prompts, with the audit log, the origin mark, `dojo-mesh pause` and a tool surface that takes no user name as the safeguards. `mesh_task` hands work to the peer's own agent; the assistant instructions say to use it instead of driving a peer's desktop. There is no tool to pause, resume, enrol or change the policy. Not a boundary: while the policy grants root SSH between nodes, anything running as the user can still use it (ADR-0006).
