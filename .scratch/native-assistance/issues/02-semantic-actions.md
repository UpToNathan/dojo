# Semantic actions and non-GTK identity

Type: task
Status: done

## Brief

Complete semantic actions and exact native-window bindings across the toolkit
families runnable on this host. Do not maintain an app allowlist. Helium web
content continues through browser-control.

## Acceptance

- Invoke, text replacement and numeric value changes use a fresh session-owned
  reference, live root membership and capability checks, and one Niri actor send.
- Human takeover before dispatch sends no mutation. Partial or unconfirmed
  dispatch is unknown and cannot be retried automatically.
- GTK and generic annotations identify the actual surface and AT-SPI root.
- Qt and GPUI producers publish that identity from the toolkit owning both.
- Unbound observations never claim exact-window authority. Application-scoped
  mutation requires a proven complete process-generation fence.
- Copied protocols and toolkit code have exact source revisions and provenance.
- Real transport and native application checks run in the isolated workspace.

## Design decision

Use the exact-window design as the implementation base. Add application scope
only after complete client membership can be proved. A missing producer is an
explicit capability limit, never permission to match titles, geometry or PIDs.
The existing Niri event actor owns the final write. Pi owns the session and
history; this adds no scheduler or durable approval database.

## Answer

Implemented and live-verified for GTK, Qt5, Qt6 and repository GPUI. Missing toolkit producers remain an explicit capability limit. See [delivery and evidence](../semantic-result.md).

## Completion audit follow-up

The four-runtime delivery proves those producers, not every runnable toolkit. Keep this issue claimed while investigating Electron/T3 and other installed runtime families. The current T3 binary reports Electron 44.4.2 and Chromium 152.0.7977.130; earlier Electron 43.6.0 research does not pin this deployed runtime. Exact bindings for missing producers remain outstanding.

## Closure (2026-10-15)

Scope settled: exact semantic actions and window binding cover GTK, Qt5, Qt6,
GPUI and any generic-annotation producer. Chromium/Electron is excluded by
decision (see [issue 03](03-electron-producer.md)); those apps use OCR/vision and
native input. XWayland and unbound runtimes remain explicit capability limits.
