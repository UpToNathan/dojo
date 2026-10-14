# Suppress focus follows mouse during keyboard activity

Status: resolved
Type: task

Implement the [typing focus spec](../spec.md). User requested focus follows
mouse except while typing, including monitor/workspace focus behavior.

## Answer

The primary keyboard path records recent activity. Focus follows mouse returns
before changing any window, output, or layer while primary keys are held or
keyboard activity occurred within 750 ms. Click focus and independent agent
input remain separate. No timer applies deferred focus.

The compositor fixture reproduced the old behavior (window 1 stole focus from
window 2 after typing), then passed with the guard. It verifies both window and
monitor focus suppression and recovery. All 17 agent-input tests passed.
Release built with full desktop features, validated the live configuration, and
installed as 786882e3dec1ae37 for the next login. Current session was preserved.
