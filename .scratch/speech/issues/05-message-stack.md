# Session message stack and reading lifetime

Type: task
Status: resolved

Preserve user and agent messages across follow-ups. New messages enter at the
bottom and push earlier messages upward. Keep messages visible while recording,
transcribing or running an agent, then dismiss after 30 idle seconds. Keep the
T3 conversation open for that same idle interval so follow-ups retain context.

## Verification

Speech state tests cover ordered follow-ups, lifetime pauses while working and
recording, idle expiration, and removing expired history. Live captures are in
/tmp/dojo-message-stack. The desktop frame and resident voice service are active.

Live inspection confirmed all four messages persisted after 31 seconds of work
and disappeared after 31 seconds idle. Speech state tests passed.
