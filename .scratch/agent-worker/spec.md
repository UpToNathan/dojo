# On-demand desktop worker

Keep the stdio MCP adapter available to its client, but run no desktop-control worker or agent seat while idle. The first valid input action starts a worker that owns the Niri controller connection. Context queries and idle reset/release calls do not start a worker.

Release stops the worker immediately. After 60 seconds without an input action, the worker closes its controller and exits without polling from the adapter. Expose a positive `--idle-timeout-seconds` option for deployments and verification. Context reports the active worker PID and seat, or null after shutdown. Invalid arguments leave an existing worker unchanged.

Worker failure, input execution error, adapter exit, and adapter crash remove the seat and release held keys/buttons. The adapter reaps stopped children and creates a new worker for a later action. Never replay an input action automatically after ambiguous failure. Preserve reset's existing semantics: release held input and focus while retaining a currently active seat.

Verify no worker at initialization/context, worker creation on action, independent controller ownership, idle expiry without context polling, held-input and cursor cleanup, restart after expiry/crash, repeated release, and graceful and abrupt adapter disconnect. Run the existing raw Wayland and GPUI MCP scenarios against the changed implementation.
