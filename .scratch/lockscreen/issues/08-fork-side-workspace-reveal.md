# Niri fork: fade the workspace in on unlock

Status: needs-triage
Blocked by: 07

The shell now hides the workspace on every closing, so the fork no longer needs to fold it away. What remains is the unlock opening, where the locker cannot show the real windows and the opening reveals a blank workspace until the desktop returns. Have the fork fade the windows back in from the centre as the lock is released. It is optional polish. Triage whether the added fork drift is worth it before picking it up.

## Acceptance

- Unlock fades the windows in with no blank flash, and a normal unlock is never delayed by it.
- Under reduced motion the fade is skipped.
- The change is small and isolated in the fork so upstream merges stay cheap.

## Progress

Not started. Optional polish.

## Comments
