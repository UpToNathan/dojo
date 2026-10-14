# Dojo accessibility observation

A read-only Rust AT-SPI extraction from CUA at
`88aff7e5424e9ed45b93ded85eb4111472ca5190`. Source evidence and patches are
recorded in provenance.json. The inherited MIT notice is in LICENSE.md.

The public asynchronous API is `observe_process(pid, process_start_time,
deadline, cancelled)`. `observe(&ObserveRequest)` additionally accepts budgets
and an optional query. A Tokio runtime drives D-Bus. There is no internal agent
scheduler, process-global observation cache, listener, snapshot authority,
mutation method, or action token.

The service supplies PID and Linux `/proc/PID/stat` start time from native
identity evidence. The observer checks that process before discovery, during
traversal, and before returning. It obtains the accessibility bus through the
current session's org.a11y.Bus service, proves service UID, requires one local
Unix bus address, proves registry UID, pins object bus names to unique owners,
and proves each application's owner PID and UID. Child objects from other
processes are omitted with an explicit incomplete-tree issue. Every observed
owner is checked again after traversal. There is no process-name, title,
first-window, or executable-name fallback.

The result reports application scope and always has `actionable:false` and
`window_scoped:false`. It walks all matching application roots. Returned bus
owners and object paths are observation evidence. Indices are local display
indices and cannot authorize later effects. Application-reported screen bounds
always have `bounds_authoritative:false`; native Wayland clients can report
window-relative geometry despite requesting screen coordinates. No origin
correction or exact Niri frame match is inferred.

`available` means a proven application root was found. `complete` means the
bounded traversal reported no omissions or read failures. It does not prove an
atomic tree snapshot or exact-window absence. `issues` names deadline, node,
depth, child, text or action limits, privacy-filtered subtrees, unavailable data,
repeated objects, cross-process content, and unresponsive calls. Query projection
preserves original indices and parents; `query_filtered` records this projection.
`nodes_pending` is a lower bound for enumerated work left on the traversal stack.
No matching app or denied/unavailable accessibility returns an unavailable tree,
never a claim that elements do not exist. Cancellation and process reuse refuse
the result.

Each D-Bus call has a 500 ms backstop under the caller's absolute deadline.
Cancellation is checked every 20 ms while waiting. Futures borrow local
connections; there are no spawned traversal tasks or abandoned blocking workers.
Connections are closed or dropped when observation ends. Default limits are
5000 visited nodes, depth 64, 2048 children per container, 4096 text characters,
and 32 actions per node. Properties use `CacheProperties::No`, avoiding toolkit
GetAll crashes and unwanted password-field reads.

Password controls expose no name, description or text, and their subtrees are
not entered. Unknown/invalid roles also refuse content. Known credential labels
suppress name, description, text, and descendant content. No clipboard is read.
Strings discard control characters except tab/newline and have character limits.
The imported text_sanitize helper cleans mutation arguments by stripping agent
protocol closing tags. Observation text keeps literal closing tags, which may be
legitimate document content; its test proves this deliberate incompatible rule.
This filters known semantic fields, not arbitrary secrets in ordinary labels.
Errors contain classified reasons instead of raw application D-Bus error text.

## Adaptations

The coherent read-only path uses the imported dojo-cua-primitives WalkBudget for
node admission and element_query projection through a local-index adapter. The
request deadline additionally bounds setup and D-Bus reads; the upstream walk
clock starts after setup and cannot supply that guarantee alone. It preserves
upstream preorder, bounded traversal,
unique bus owner pinning, password refusal, plausible extent checks, and query
matches with ancestors. It replaces the native backend's shared connection,
listener, X11/Hyprland/window guesses, action routes and side index with explicit
request identity and local observation lifetime. It fetches children by bounded
GetChildAtIndex rather than potentially enormous GetChildren replies. It reports
all nodes, including containers and hidden controls, rather than creating an
upstream actionable control index. Roles use the AT-SPI numeric enum's canonical
names, so `button` is returned instead of toolkit-local `push button` names.
Text uses bounded GetText; numeric Value fields retain upstream debug formatting
and take precedence over editable text. Bounded LABELLED_BY and preceding sibling
labels name unnamed controls, with owner/role checks before reads. Credential
labels are resolved before any text/value call. Depth limits and field truncation always mark incompleteness.

Run `devenv shell -- cargo test --manifest-path crates/dojo-accessibility/Cargo.toml`.
Rust D-Bus fixtures exercise actual proxy calls, password read counters, duplicate
names, disabled controls, numeric values, credential labels, query ancestors,
wrong process ownership, huge containers and depth/node
budgets. Additional tests cover PID reuse, cancellation, deadlines, control
characters and extent bounds. No installed application or desktop is driven.
