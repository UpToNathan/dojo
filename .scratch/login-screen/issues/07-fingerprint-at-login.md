# Fingerprint at login where a reader exists

Status: ready-for-agent
Blocked by: 05

Offer fingerprint at login only when a reader exists and the typed user has an enrolled print (ask fprintd, through the helper path ADR 0002 uses). Jupiter has no reader and must show a plain password screen. Document the keyring consequence in the hint-free scene and in `system-config/README.md`.

## Acceptance

- No reader or no print: no sensor state, no fingerprint worker.
- A fingerprint login starts the session; the keyring stays locked and nothing claims otherwise.

## Progress

Built on branch `greeter` with 05.

- [x] `UserLook` and `EventKind::LookReady` carry `fingerprint`, taken from the display manager's `User` reply. The model starts, keeps or releases the fingerprint worker from it, per user. Typing another name releases the old user's sensor and waits for the new user's answer.
- [x] No print or no reader: no worker, no sensor state in the dial, the plain password hint (tested, and seen in the nested smoke for a user the fake display manager reports without a print).
- [x] A fingerprint win kills the password worker and starts the session for the fingerprint conversation's generation. Nothing on screen mentions the keyring (tested).
- [x] `system-config/README.md` says a fingerprint login leaves the keyring locked.
- [ ] A real reader and a real enrolled print on the host: unverified.

## Comments
