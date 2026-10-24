# Native accessibility actions

Dojo reads AT-SPI trees and performs semantic actions through its Niri input
actor. Helium web pages continue to use browser-control.

## Use the tools

1. Inspect `desktop_context` and choose a native window.
2. Call `desktop_accessibility` with its `window_id`.
3. Check the returned scope. Only an exact window tree can issue `element_refs`.
4. Call `desktop_accessibility_action` with one ref and an action.
5. Inspect a new tree or call `desktop_verify` to check the UI result.

```json
{
  "element_ref": "a:the-reference-from-the-observation",
  "action": { "kind": "replace_text", "text": "Example" }
}
```

Supported kinds are `invoke`, `select`, `replace_text`, and `set_numeric`.
Selection acts on the observed child of a validated direct Selection container.
Numeric changes require a finite advertised range. Disabled, private, defunct,
read-only or unsupported controls refuse the relevant action. There is no
arbitrary action-index or object-path parameter.

References expire after 15 seconds and belong to one desktop connection. Every
attempt consumes the snapshot. New observations replace refs, and Niri's input
generation invalidates them across sessions after human activity or agent input.
A ref authorizes an attempt; it does not grant ownership over a human-owned app.

`complete` describes traversal coverage. `semantic_complete` additionally
requires exact scope and no query projection or redacted nodes. Positive public
nodes can have refs even when another subtree is unavailable or private. An
incomplete tree cannot establish that a missing control does not exist.

## Delivery and uncertainty

zbus authenticates the accessibility connection and prepares one typed request.
It seals its writer before transferring the commit descriptor to Niri. Niri
accepts semantic handoffs only from the configured immutable Dojo agent binary
and its live native peer. It checks the surface annotation, process generation,
input generation and human ownership immediately before its kernel send.

`acknowledged` means the application returned success. It does not mean the UI
postcondition was verified. `rejected` is an application reply, not a promise
that the application made no changes. `refused` means this request did not reach
the application through the mutation writer. `unknown` must never be replayed.
Partial writes are not completed or retried.

An unresolved operation blocks semantic and native input to sibling windows in
the same process generation. The service keeps its reply decoder for late
receipts. Pi also retains its existing external-effect marker before dispatch.
A matching late receipt can retire that marker while keeping the earlier
uncertainty in the exported receipt. A fresh screenshot alone cannot clear it.
If the service is lost after dispatch, automatic recovery is deliberately
unavailable without terminal or process-retirement evidence.

AT-SPI has no conditional child-object generation operation. Rewalking the root
and comparing live identity/capabilities narrows stale-reference risks, but
cannot make application tree changes atomic with execution.

## Toolkit coverage

| Runtime | Exact native binding | Delivery route |
| --- | --- | --- |
| GTK exposing GTK shell v7 | Its existing surface annotation | Stock producer |
| Qt 6.12.0 from the pinned system package | Generic surface annotation | Version-checked generic plugin |
| Qt 5.15.19 from the pinned system package | Generic surface annotation | Patched QtWayland accessor, guarded QPA loader and generic plugin |
| Repo-built GPUI | Generic surface annotation | Forked AccessKit endpoint callback, published by the owning Wayland window |
| Other generic-annotation producers | Their supplied exact surface/root tuple | Validated protocol consumer |
| Chromium/Electron apps, XWayland and other unbound runtimes | Unavailable by decision | Application-scoped reads, perception worker (OCR/vision) and existing visual/native input |

Coverage follows toolkit capabilities, not an app allowlist. Bundled, sandboxed
or differently built runtimes need their own matching producer. PID, title,
geometry and Electron's opaque Wayland handle cannot prove a window/root join.
Apps exposing no accessibility tree require visual/native input.

Chromium and Electron apps are deliberately not patched. Dojo does not build or
maintain a Chromium runtime. These apps are supported only through the installed
perception worker (OCR/vision) and capture-bound native input; Helium web content
stays on browser-control. The abandoned producer source remains under
`tools/accessibility/electron/` for reference and is not installed or claimed.

The pinned Home Manager modules enable toolkit accessibility and install both
Qt producers for future launches. They preserve theme/style settings and reject
incompatible private ABIs. Existing apps need a later restart. The versioned Niri
installer stages the compositor and trusted agent for the next login.

## Source and acceptance evidence

[Source pins](../provenance/native-accessibility.json) record the protocol,
AccessKit and GPUI bases. Qt manifests record exact runtime revisions and the
small backend patch. CUA remains pinned at
`afed1fcbba64acadcc154b8d5d377549ae62bc7e`.

Run `tools/speech/check-semantic-accessibility-live.mjs --private-bus` through
devenv after building `dojo-agent`, `dojo-accessibility-fixture` and Niri. It uses
an inactive `dojo-tests` workspace, a separate runtime directory, and a real
private accessibility bus. It records literal runtime coverage and observed
control changes, reference refusals, takeover and workspace restoration.

## Read-only coverage inventory

Run `node tools/accessibility/audit-native-bindings.mjs` in the native session to
inventory currently mapped windows, process identity, loaded accessibility
library names, and surface annotation availability. It does not read window
titles or application trees, focus windows, or send input. Unsupported compositor
requests and retired windows remain `unknown`. An annotation is metadata only;
the agent must still validate bus credentials, traversal and action capabilities.

The running compositor may precede the staged build. In that case, a rejected
`AccessibilityTarget` request means the session cannot provide binding evidence;
it does not prove that the application lacks a producer. Activate a staged Niri
build at the next planned login rather than restarting a live desktop for this
audit. Use the isolated acceptance runner to exercise new compositor changes.
