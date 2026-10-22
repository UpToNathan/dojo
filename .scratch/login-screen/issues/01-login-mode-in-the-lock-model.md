# Login mode in the lock model

Status: resolved

Give `lock::LockModel` a login mode: user chip with editable username, session chip, password only by default, authentication through an authority that is not the local PAM workers, and a `SwapLook(user)` effect when the username is committed. The scene, dial, pill, shake, pause and power timeline are shared with the lock screen. No process or window code; this is the pure reducer and scene, with tests.

## Acceptance

- Typing goes to the password pill unless the user chip is being edited; Tab or a click edits it, Enter commits.
- Committing a name emits `SwapLook(name)`; the scene cross-fades (cut under reduced motion).
- Unknown names, empty names and over-long names are rejected in the model.
- Lock-screen scenes and tests are unchanged.

## Comments
