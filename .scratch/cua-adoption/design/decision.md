# Adoption design decision

Use candidate A's small session interface and existing Dojo resource owner. Add candidate B's compositor peer identity, named-output capture, cancellation and capture-correlated scene requirements. Pi remains the scheduler and durable conversation owner.

Retaining the whole private CUA core/browser dependency depends on the embedded ownership experiment. Closed sessions must never be revived by an imported API. If that requires widespread lifecycle exceptions, extract coherent implementations with explicit Dojo context instead.

Stable Niri IPC layout does not prove rendered window geometry. Tiled actionability requires evidence associated with the actual capture render. External browser and accessibility mutations require the separate admission proof in issue 16.

Current implementation units are native Wayland capture, richer guarded native actions, embedded-core ownership, and deterministic verification. Source imports alone do not count as product capabilities.
