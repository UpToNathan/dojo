---
status: accepted
---

# Fingerprint and password run as two separate PAM conversations

The lock screen starts one PAM conversation for the fingerprint (`pam_fprintd`) and another for the password (`pam_unix`) in separate worker processes, each with its own service file. The first to succeed unlocks, and the other worker is killed. A single PAM stack cannot do this: `pam_fprintd` blocks, so the password prompt would only appear after the fingerprint attempts time out. Keeping PAM as the authority for both avoids trusting a fingerprint match that the locker checked itself over D-Bus.

## Considered options

- **One stack with `auth sufficient pam_fprintd.so` ahead of the password.** Rejected: finger first, then password, never either-at-once.
- **Talk to `net.reactivated.Fprint` over D-Bus in parallel with a PAM password check.** Gives more UI control, but the locker would unlock on a match it verified itself.

## Consequences

- Workers must be separate processes so the loser can be killed and the fingerprint sensor released. A spike confirmed the sensor is claimable again right after the kill.
- The fingerprint service deliberately has no `pam_faillock`, so wrong fingers do not lock the account, while the password service shares the faillock tally with ly and sudo. Whether fingerprint stays usable while the password is paused is still undecided (see the spec).
- `/run/faillock` is root-owned, so a non-root locker cannot create a missing tally file. A wrong password still fails and a correct one still succeeds, but failures are not counted until root has created the file.
