# On-demand desktop worker

## Notes

Keep only the MCP adapter present while idle. See [the spec](spec.md).

## Decisions-so-far

- [Lifecycle](issues/01-lifecycle.md): a lazily spawned child owns the seat and its idle deadline. EOF handles adapter crashes; exit notifications reap the child without periodic adapter polling. Nested raw/GPUI checks and the actual desktop lifecycle check pass.

## Fog

An existing T3 provider session must reload to pick up the rebuilt adapter. Smooth movement and cursor styling remain separate compositor work.
