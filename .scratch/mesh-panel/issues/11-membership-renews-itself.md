# Membership renews itself, and says so only when it matters

Status: resolved

`dojo-mesh renew --due` renews every entry within 30 days of expiring, unattended: it uses only signing keys that need no touch, and says (exit 3) when it could not. The shell runs it every few hours on a node that can sign. The panel shows days only within 14 of expiring; a card with an entry that has expired says so.

## Acceptance

- A hardware key is never waited on in the background.
- The panel shows no number for an entry with more than 14 days left.
