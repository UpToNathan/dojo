# CUA Driver in Dojo's activated service

The consumed upstream revision is `88aff7e5424e9ed45b93ded85eb4111472ca5190`.
`copy-plan.json` pins the fork patch and every changed file. The original
Driver workspace keeps its own derived Cargo.lock. The small copied worker
transport and shared native seat library have independent locks.

Build from the repository root:

```sh
devenv shell -- scripts/build-cua-driver.sh
devenv shell -- cargo build --offline --locked --bin dojo-agent
```

The build script verifies the source copy, builds a release binary offline with
its lock, installs it as `$XDG_DATA_HOME/dojo/bin/cua-driver` (default
`~/.local/share/dojo/bin`), and writes the binary digest plus source digests
beside it. The service checks these pins before starting the worker.
`DOJO_CUA_DRIVER_BINARY` selects another built binary with the same adjacent
`.dojo.json` manifest. The Cargo target directory is managed by mbx like any
other; only the installed binary and manifest are used at run time. The `dojo-agent`
that Niri's `environment.sh` pins through `DOJO_SEMANTIC_AGENT_EXE` is a separate
immutable copy, so it only reads this installed driver after it is re-pinned from a
build that has this path; see [the Niri session notes](../../../docs/niri-session.md#re-pin-the-semantic-agent).

The existing MCP adapter connects to the existing on-demand Unix service.
Each client connection owns a private inherited-stdio Driver worker and one
trusted CUA session. There is no new public socket or task scheduler. The
service authenticates its socket clients and applies DesktopPolicy to the
original tool name and arguments before forwarding. Driver artifact metadata
also enters the service socket identity, preventing reuse across builds.

`desktop_driver_tools` returns admitted tools from the original runtime catalog,
preserving their schemas. The service also rejects calls to unadmitted names.
CUA task runners, session management, installers, and mutations without Dojo
host admission are excluded. Pi remains the task owner.
`desktop_driver_call` forwards its tool name and arguments, preserving original
MCP image blocks and structured results. Pi registers the catalog as `driver_*`
tools and forwards each call through the same service. These registrations
remain sequential and unsafe to replay. Reconnection checks the catalog hash.

On Niri, click, press_key, type_text, scroll and bring_to_front use the shared
Dojo AgentController. Every attempt requires a fresh capture_id, exact PID and
window ID. Captures retain compositor/process generation, exact floating-point
render geometry, output geometry and input ownership revision. CUA's integer
inventory positions round down; they do not authorize native input. Fractional
origins survive in capture evidence and sampling. Input receipts do not prove
the application changed; original CUA execution records report that distinction.

Driver and legacy native input alternate by retiring the previous controller.
Disconnect, cancellation, idle expiry and explicit release retire the owned
worker independently of blocked request I/O. Niri then releases its seat.
CUA's overlay is disabled for Dojo workers; Niri owns the agent cursor.
The full Driver SDK stays in the child, keeping its AT-SPI/zbus runtime
features out of Dojo's tray process.

This integration does not establish full Driver parity. Native Unicode text,
multi-output desktop capture, clipped-window capture, and general semantic/browser
mutations remain unsupported or refused. The current task covers this host;
VM and QEMU products are outside it.

Live acceptance runs a nested native Niri process in the inactive `dojo-tests`
workspace and leaves the user's active workspace unchanged:

```sh
DOJO_TEST_CUA_DRIVER=1 devenv shell -- cargo test --offline --locked \
  --test capture_desktop -- --ignored --nocapture
```

## Running compositor gate

The host read check at `/tmp/dojo-driver-host-read.ia5Icd` used the actual running
Niri `26.04 (7fb1e4ffa7-modified)`. Driver startup/catalog and screen discovery
worked, but window discovery returned `error parsing request`: this running
binary lacks the new WindowPlacements request. Its window inventory also lacks
workspace-view tile positions. The isolated native compositor tests use the
updated fork. They do not establish that the old running session can accept
capture-bound Driver actions. The updated compositor requires a new login;
changing the service cannot replace an existing Wayland compositor connection.

The full release compositor was rebuilt with the locked dependencies, validated
against the existing host config, and passed the Driver service acceptance at
`/tmp/dojo-capture-live.3p6yRz`. Its binary digest is recorded in
`provenance/evidence/cua-driver-service.json`. It has not been installed. Issue 13
requires separate deployment authorization; installation uses the existing
backup-preserving script, and the running session changes only after a new login.

## New-session host acceptance

After the approved installation and new login, the running host is
`26.04 (04b755fa14-modified)`. Host window discovery, screen discovery and
capture-bound window PNG delivery pass through the activated service at
`/tmp/dojo-driver-host-window.F5u7Ag`. Fractional rendered dimensions are sampled
into integer image dimensions and mapped proportionally back to the exact
native rectangle. Exact-window capture selects its workspace output when
multiple outputs are active. Whole-desktop multi-output capture still refuses.
The 28 Driver Niri tests and release-compositor input acceptance at
`/tmp/dojo-capture-live.a0MsVB` pass.
