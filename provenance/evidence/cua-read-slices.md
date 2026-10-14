# CUA read contract slices

Baseline: `88aff7e5424e9ed45b93ded85eb4111472ca5190`.

The Rust MCP unit suite passes 42 tests. Catalogue tests pass four tests;
the systemd application-launch test remains ignored. Ledger verification checks
474 files, including 264 unchanged copies and 28 reproduced modifications,
plus 54 pinned source objects for translations. These results establish partial
coverage only.

## Discovery

`list_windows`, `get_screen_size`, and `list_apps` have the pinned input schemas
and read-only annotations. Native geometry projection tests cover exact PID
filters, active-workspace visibility, offset outputs, fractional scale and missing
geometry refusal. Screen selection uses the focused output or a unique enabled
output. Named lifecycle sessions remain unsupported.

Application metadata uses the shared launcher catalogue, strips `.desktop` from
bundle IDs, projects `Exec` without field codes, and uses the Linux `desktop`
kind and reserved false `active` flag. Process discovery reads bounded status
and executable metadata, excludes zombies, and never exports full arguments.
The deterministic background-process fixture passes. Launcher ambiguity now follows the pinned exact-path and latest-mtime rules.
Per-app window records and shared-process multi-launcher matching are covered
by deterministic fixtures. Exact error conformance and the broader host matrix
remain open.

## Capture-bound perception

The public schema comes directly from the pinned manifest. The source worker is
built from the baseline, with separately hash-pinned models and ONNX runtime.
An immutable local Nix installation provides exact interpreter and library
files. Production execution uses a contained cold process with cancellation,
reaping and capture lifetime bounds. The contained known-answer test finds real
OCR text; the nested desktop test proves capture identity, concurrent reads and
non-consumption. Neither observation nor perception grants action authority.

Desktop-wide parsing, warm behavior, complete quality metrics, additional host
adapters and exact error envelopes remain open. The staged local installation
is not a complete redistributable or reproducible Nix package. MCP cancellation has a passing disconnect-guard unit test. The complete live
acceptance passed with real perception, capture preservation, concurrent reads,
cancellation retirement/recovery, typing, holds, drag, takeover and disconnect
cleanup. Evidence was retained at `/tmp/dojo-capture-live.JpTLYF`; private
images and application inventories are not copied into this repository.

The earlier inactive-workspace failures were traced to the nested EGL surface's
default swap interval. Setting it to zero after binding prevents the event loop
from waiting for the hidden host's frame callbacks. The successful run used the
source-built fork on inactive `dojo-tests`, with no widened input deadlines.
This nested backend correction does not install or restart the user's compositor.
