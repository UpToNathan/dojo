# Dojo memory implementation result

The earlier T3 adaptation has been replaced for Dojo by a local Pi-durable
runtime. T3 remains available for unrelated work. The shared engine implements
UniiChat's daily logs, permanent IDs, binary summaries, persisted foreground and
compaction views, age-ranked batched merges, eight summary workers, per-turn
readiness, zoom/date navigation and same-conversation summary correction.

Dojo uses its existing OpenAI subscription through the Codex adapter. The live
endpoint rejected explicit cache options, so the implementation uses stable
identity and four-line implicit message boundaries. The Claude OAuth request
builder preserves required identity and adds the gist's explicit marks, with
one-hour subscription retention. Live Claude authentication has not been tested.

Verification evidence from this run:

- `/tmp/dojo-memory/pi-tests-final.log`: 50 passing behavior tests.
- `/tmp/dojo-memory/pi-live-verified/result.json`: actual OpenAI zoom retrieval,
  exact source and saved-view preservation across restart, fixed instructions,
  tools and cache identity, protected source credentials/configs unchanged.
- `/tmp/dojo-memory/pi-desktop.json`: actual desktop_context tool connection.
- `/tmp/dojo-memory/pi-service-result.json`: installed voice socket using Pi.
- `/tmp/dojo-memory/pi-review-final.md`: independent final review.
- `/tmp/dojo-cache-check-8oXq5F/result.json`: actual OpenAI cache reuse
  observed with a growing synthetic view.
- `/tmp/dojo-memory/decisions.tsv`: append-only decisions and verification trail.

The initial final live trial returned `stored.` and failed a stricter `stored`
assertion. The revised test accepts punctuation while still requiring the exact
marker and an actual zoom call. The fresh revised trial passed.

Review corrections cover stale archive reads, active steering record order,
summary shutdown cancellation, failed-summary readiness retry, clipped-output
completeness, cache-writer handoff, and raw transcript preservation. All were
verified with behavior tests. View thresholds are batch triggers; as the gist
specifies, unfinished batches may continue once the required parents are built.

The 29 previous Dojo memory records were explicitly imported as notes with
source receipts. The old archive remains unchanged. Borrowed subscription
access is read without copying or rotating another app's refresh token. Own
Dojo logins use private credentials and the voice ownership lock. Temporary
verification credential files were removed.

The active Dojo voice service was restarted while idle. Native transcription,
speech bubbles, desktop tools and input ownership remain in place. No cache-hit
percentage, subscription quota saving, or battery improvement is claimed.

## Subsequent cleanup

At the user's request, the old archive was moved to Trash after all 29 imports
were verified against their original sources. The imported notes were then
removed from active memory, including derived views and durable summary
sessions. Six native setup-test records were preserved; credentials and
configuration remained in place. The obsolete legacy import script was removed.
Evidence: `/tmp/dojo-memory/imported-notes-removal.json`.

The user subsequently requested removal of the six remaining setup-test
records. Active memory and durable context were cleared completely; credentials
and configuration were retained.
