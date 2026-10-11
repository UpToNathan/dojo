# Shared pointer coordination

Preserve independent compositor cursor positions and seats. Apps that bind the
agent seat receive native events. Apps that bind only the primary seat receive
agent input through their existing client resources, without changing the human
seat's compositor pointer position or layout keyboard focus.

Human pointer interaction inside a client, or keyboard interaction with its
focused surface, takes ownership before delivery. Cancel conflicting agent
motion and held input, retain the visible agent cursor, restore human protocol
focus/modifiers, and notify observers. Pointer ownership ends when the human
leaves the client; keyboard activity ownership ends when pointer interaction
resumes after held keys are released. Coordinate at client connection scope,
since separate windows can share one input pipeline.

Serialize held agent interactions using a client's shared primary input resources.
Native agent seats retain independent held input. Reject actions that cannot
be delivered; never acknowledge an event for an unbound input resource. Expose
coordination revision and human-owned window IDs through IPC and desktop context.
Guard agent actions against the last observed revision and return structured
paused results without destroying the cursor on human takeover. Publish changes
on Niri's event stream. No decision model is integrated in this implementation.

Verify independent native seats and an isolated copy of T3's Electron runtime,
including clicks, scrolling, keyboard/modifier delivery, human takeover,
stale-action rejection and disconnect/lock cleanup. Install the release for the
next login without stopping the user's running compositor.
