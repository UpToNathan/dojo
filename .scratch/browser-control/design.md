# Integration decision

Choose the native final-writer design. It is the only candidate that preserves
Dojo's admission ordering without a check/send race. Adopt the alternative's
out-of-semaphore generation retirement and per-agent session ownership.

The independent judge agreed. Reject an EventStream-only gate as authority;
events can assist diagnostics but cannot order browser sends with takeover.
Keep Playwright/CDP semantics in the upstream implementation. The native bridge
owns transport admission and framing, not a second browser driver.

The kernel write is the admission point. Chrome/page effects already admitted
may continue, including page timers installed by a prior command. Report these
as in flight or uncertain. Never promise effect rollback or physical effect-start
ordering. Browser-client-wide takeover is conservative because existing browser
tabs have no proven exact Niri window binding.

Caller operations stay upstream MCP tools. Pi binds an agent session, resumes a
fresh generation for a new task, and calls `execute_cancel` before waiting for
task-storage shutdown. Task completion preserves tabs and retires input.

Packaging and agent configuration use the same immutable package. The old CUA
browser adapters are removed after their callers migrate. Native desktop input
and the independently used CUA core policy, timestamp and perception remain.
