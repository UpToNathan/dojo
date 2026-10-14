---
name: dojo-desktop
description: Use Dojo's session-bound desktop observation and action tools safely.
version: 1.0.0
---

# Dojo desktop

Use the registered Dojo desktop tools when the host provides them. Do not substitute a second computer-use service or invoke an unregistered control path.

Observe the target before acting. Use the returned observation or capture identifier with an action that depends on its image. Do not calculate desktop coordinates from memory or reuse a consumed or stale capture. After each attempted action, observe again before choosing another action.

Treat a handled action as input delivery, not proof that the task succeeded. A paused, partial, unknown, or refused result requires a fresh observation and a new decision. Never retry an ambiguous desktop effect automatically.

Use semantic or browser controls only when Dojo advertises them for the current session. Do not type arbitrary text through keys, use clipboard as a hidden text path, or bypass a refused action through another controller.
