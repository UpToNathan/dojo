# Native accessibility and task evidence

Type: task
Status: resolved

## Brief

Complete useful native accessibility and diagnostic integration from the pinned
CUA adoption. Follow ../spec.md. A source copy is not evidence of a working
window binding or successful task.

## Acceptance

- Exact binding has native surface evidence and rejects stale/reused targets.
- Unbound applications remain explicitly application-scoped and read-only.
- Driver and browser-control calls appear in redacted Pi trace exports.
- Unknown effects survive errors; verification is separate from delivery.
- Focused checks and isolated live evidence are recorded in result.md.

## Comments

The initial investigation found no generic AT-SPI/Niri join. GTK's existing
gtk_surface1.set_a11y_properties may provide a supported toolkit-specific join.

## Answer

Implemented GTK7 exact-surface read binding, native element verification and
operator-accessible Pi trace export. The final optimized live fixture passes
on inactive `dojo-tests`, including same-process/same-title separation, privacy,
stale generation and closed-window refusal. The versioned compositor is staged
for the next login. Semantic mutations remain explicitly disabled.

See [the result](../result.md) for pinned source identities, commands, evidence
and remaining toolkit/admission limits.
