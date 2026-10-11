# Coordinate independent cursors through shared client input

Type: task
Status: resolved

Implement [the spec](../spec.md), using the Chromium fixture from
[the compatibility bug](../../agent-input/issues/04-chromium-seat-compatibility.md).

## Implementation

The compositor owns connection-scoped human input coordination and publishes
revisioned snapshots through IPC and the event stream. Single-seat clients
receive agent input through their existing primary resources. Native seats
retain independent input resources. Human takeover releases conflicting held
input, cancels pending arrival waits, restores protocol focus and modifiers,
and retains the visible agent cursor. Changes beneath a stationary human
pointer also update ownership.

The Rust controller guards actions and waits with its observed revision. MCP
returns structured paused results without destroying the worker or seat. The
worker still exits on idle timeout and starts only when an action needs it.

## Verification

The compositor suite passes 220 tests, including shared input delivery, held
key/button cleanup, frame ordering, competing shared agents, native seat
independence, keyboard takeover, stationary-pointer ownership cleanup and
cancellation of pending guarded waits. The root SDK and MCP suite passes 32
tests. Logs are `/tmp/dojo-coordination-final220-tests.log` and
`/tmp/dojo-coordination-sdk-final.log`.

Final release evidence:

- Native motion, raw input, modifiers and disconnect:
  `/tmp/dojo-agent-nested.aY4T96`.
- GPUI input, independent modifiers and key repeat:
  `/tmp/dojo-agent-nested.xQ6qk1`.
- Electron primary-resource delivery and human takeover:
  `/tmp/dojo-electron-coordination.gt9a9b5m`.
- MCP native worker lifecycle:
  `/tmp/dojo-agent-nested.fbW3Lo`.
- MCP GPUI worker lifecycle:
  `/tmp/dojo-agent-nested.7t6rxh`.

Both lifecycle runs cover idle expiry without polling, held-input cleanup,
explicit release, worker crash, adapter disconnect and restart. No worker or
seat remains after cleanup.

The release was installed at
`~/.local/share/dojo/niri/bd3c755dd5eb872b/niri`. The installed and built SHA-256
hashes match. The running compositor remains on its existing binary; the fork
becomes active at the next Niri login. Reload the T3 provider session after
login to load the updated desktop adapter. The current shell and T3 were not
restarted.

Decision model and cost research is recorded separately in
[decision-models.md](../decision-models.md) and [clef-cost.md](../clef-cost.md).
No hosted inference requests were made.

## Live desktop confirmation

After login into the installed fork, the T3-connected desktop tools successfully
closed and reopened T3's actual sidebar at output-local coordinates `(107, 60)`.
The user moved their mouse onto the desktop frame before the clicks. Entering
T3 again changed ownership from revision 12 with no owned windows to revision
13 with window 2 human-owned. The stale action paused, and a fresh focus request
also paused with `human owns this application`.

The cyan cursor remained visible at `(850, 480)` after takeover, with 632 matching
pixels both before and after. Releasing the desktop seat stopped worker PID
138366; desktop context then reported both worker and seat as null. Evidence is
`/tmp/dojo-live-coordination-136452`, including sidebar, takeover and release
screenshots. Capture with Niri's `show_pointer` option when checking cursors;
plain grim screenshots omit cursors.
