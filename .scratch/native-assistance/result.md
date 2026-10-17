# Native assistance result

## Implemented

- Niri supports GTK shell version 7 and retains each wrapper's accessibility
  announcement with the exact native surface. Lifecycle changes invalidate its
  generation. Invalid and ambiguous announcements refuse exact reads.
- `desktop_accessibility` uses a selected AT-SPI root only after proving its
  unique bus owner's process and UID. It rechecks the native target and process
  generation after traversal. Clients without annotations retain explicit
  application scope.
- `desktop_verify` consumes exactly bound native control labels, values and
  states. Embedded web content stays untrusted. Redaction, truncation and query
  filtering prevent complete semantic coverage. Polling preserves the last
  actual observation when no further polling interval fits its deadline.
- Pi trace export includes the installed Driver, browser-control, native,
  perception and step routes. Receipts use the originating assistant entry ID.
  Plain error text and unknown/partial effects survive export. Verification
  events remain separate from input delivery.
- `tools/speech/dojo-voice trace` exports the running voice runtime's bounded,
  redacted trace. It never starts a daemon or changes task/input ownership.
  Export errors do not mark the agent task failed.

## Reused sources

CUA remains pinned at `88aff7e5424e9ed45b93ded85eb4111472ca5190`.
The existing bounded reader, predicates, recording codec and Pi history remain
the implementation owners. No second agent runtime was added.

GTK's XML and license are copied from 4.22.4 commit
`7f99ab1a26408b6499a18f353f081e3c0598ea5c`. Their hashes are recorded in
`vendor/niri/resources/gtk-shell.provenance.json` and checked by the reference
pin verifier. The existing source-built perception worker already had live
acceptance; this change updates agent guidance to use it for inaccessible text
and icons.

## Verification

- Accessibility reader: 12 passing Rust tests, including actual D-Bus fixtures,
  same-process sibling exclusion, owner refusal, sensitive fields and budgets.
- Verification crate: 25 passing Rust tests, including incomplete native state
  coverage and the existing bounded-step cases.
- Agent binary: 38 passing tests after the final timeout-cadence guard and
  semantic-coverage projection.
- Voice, MCP and trace projection: 38 passing Node tests, including the actual
  Rust recorder/analyzer and canonical Pi transaction export.
- Native GTK protocol: six passing tests; Niri IPC: five tests and one doc test.
- Debug native acceptance: `/tmp/dojo-accessibility-live.lQ9WxW/acceptance.json`.
  Two stock GtkApplication windows share a PID and title but announce distinct
  roots. Reads exclude sibling controls. Verification satisfies known controls,
  refuses state/absence claims through redaction, and rejects stale generations
  and closed windows. The host's active workspace remains unchanged; the nested
  compositor runs on inactive `dojo-tests`.

## Limits

Exact native binding currently requires an announced GTK7 endpoint. This is not
generic support for every native toolkit. Bounds remain application-reported
and cannot authorize pointer input. Semantic AT-SPI mutations remain disabled
until a compositor-owned final writer is implemented and verified. Existing
capture-bound native input remains the effect route.

Trace export remains opt-in and bounded to 1000 events, 10000 entries and 8 MiB.
Redaction filters known fields and explicitly supplied secret values, not every
possible secret in arbitrary application text. No recorded action is replayed.

## Deployment

Release acceptance passed at
`/tmp/dojo-accessibility-live.5yf5vp/acceptance.json`, using the final optimized
compositor and rebuilt agent. It additionally asserts the exported
`semantic_complete` distinction. Positive native control verification succeeds
in both windows; state/absence checks remain unknown in the redacted window.

The release binary SHA256 is
`aac60fe9711137bc5337644673a535ec7062c2fc57900ed59d9cfd3c5b8ea11b`.
The existing installer staged it at
`~/.local/share/dojo/niri/55354d894755fb97`, preserving the previous generation
and the existing immutable browser broker and MCP pins. The running desktop has
not been restarted. The next Niri UWSM login activates the compositor; the voice
service restarts with the graphical session and reads the updated agent guidance.

The original CUA Driver was rebuilt through `scripts/build-cua-driver.sh` so its
artifact attestation includes the updated Niri IPC source. The CUA ledger
independently verifies 515 maintained files and reproduces 29 pinned-source
patches. The 20 source-pin/ledger tests pass.

## Follow-up semantic delivery

The read-only limitation above was superseded by the [semantic action and toolkit delivery](semantic-result.md). That report records supported runtimes, exact artifact evidence and remaining capability limits.
