# Mesh panel: machine actions

Status: open

Blocked by: 03

Take cursor and keyboard, send a window (opens the switch panel on that machine), renew, remove with an inline confirmation. Each runs `dojo-mesh`; exit 3 shows the key glyph pulsing (touch your key, unlock 1Password).

## Acceptance

- A control-only session started from a card behaves exactly like `dojo-mesh remote`.
- Remove needs a second press on the same card; Esc cancels.
- The card reflects the new state after each action.
