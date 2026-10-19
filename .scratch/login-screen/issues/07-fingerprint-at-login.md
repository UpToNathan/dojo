# Fingerprint at login where a reader exists

Status: ready-for-agent
Blocked by: 05

Offer fingerprint at login only when a reader exists and the typed user has an enrolled print (ask fprintd, through the helper path ADR 0002 uses). Jupiter has no reader and must show a plain password screen. Document the keyring consequence in the hint-free scene and in `system-config/README.md`.

## Acceptance

- No reader or no print: no sensor state, no fingerprint worker.
- A fingerprint login starts the session; the keyring stays locked and nothing claims otherwise.

## Comments
