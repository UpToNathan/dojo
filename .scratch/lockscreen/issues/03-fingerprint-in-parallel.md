# Unlock with a fingerprint in parallel with the password

Status: needs-info
Blocked by: 02

Add the fingerprint worker beside the password worker so either unlocks, first to succeed wins and the loser is killed. Use `dojo-lock-fingerprint` from [the recommended service files](../spikes/pam/recommended/) and follow [ADR 0002](../../../docs/adr/0002-unlock-methods-race-in-separate-pam-conversations.md).

Treat an immediate `PAM_AUTHINFO_UNAVAIL` as no fingerprint available and do not retry in a loop. After `max-tries` no-matches, wait for user activity before restarting the worker. The dial shows the reader's state, and the PAM message becomes the hint line. Real retry hints such as "swipe too short" have never been observed, so show the message text verbatim until they are.

## Acceptance

- A real finger unlocks, and the password worker is killed and leaves nothing behind. This part needs the human; prepare it as a script like `spikes/pam/race-real.sh`.
- Typing a password still unlocks while the sensor is armed, and the fingerprint worker is killed.
- The sensor is claimable by `fprintd-verify` right after unlock or after a worker is killed.
- Wrong fingers do not feed `pam_faillock`.
- A missing or unenrolled sensor leaves password unlock working with no error shown.
- Owner decision pending in the spec: whether fingerprint stays usable during a password pause.

## Progress

Built: the fingerprint worker races the password worker, the loser is killed, and the sensor is released (stand-in stacks, and a real finger in the PAM spike). Not seen: a real no-match message from `pam_fprintd`, so the retry hints are unverified. Needs a human run of `race-real.sh` with a wrong finger.

## Comments
