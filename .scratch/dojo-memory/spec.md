# Dojo agent memory

Use the UniiChat gist as the memory specification. The user authorized a
Dojo-only Pi-durable runtime with OpenAI subscription execution first and a
Claude subscription adapter following the gist. Stay as close as possible for
both, with necessary provider differences documented and verified.

Daily append-only message/tree logs, permanent IDs, immutable binary nodes,
512-byte target, persisted foreground view of 128000→64000 bytes and compaction
view of 32000→16000 bytes. Merge age uses the pair's last message. Merge only
built parents, preserve unfinished batches, and never refit saved views.

Eight queued summary workers, fewer than eight preceding unbuilt leaves before
starting a leaf, built children before a parent. Use preceding context ending at
the task boundary. Retry oversized output up to five candidates in the same
conversation and retain the shortest. New independent turns wait for preceding
summaries and use fresh context; active input steers the existing run.

Fixed shared instructions/tools for foreground and summaries. No summary tool
execution, no default Pi compaction, no memory search, no reasoning archive.
Zoom/date retrieval, long-source pagination, owned original image restoration.
Clip tool-result text to 30000 Unicode characters combined head/tail; split
other long messages without loss. Single durable log owner and flushed writes.

OpenAI uses supported subscription cache identity and implicit message
boundaries. Claude retains required OAuth identity and explicit gist cache marks
on fixed prompt/tools, last whole four-line view block and request end. No
provider-wide configuration changes, no API-key fallback, no keep-alive pings.

Preserve the old Dojo archive, import it explicitly as notes with idempotent
receipts. Keep T3 and unrelated agents untouched. Verify durable recovery,
actual source retrieval, fixed payloads and protected-file hashes. Activate only
Dojo voice. Claims about performance or cache savings require separate measures.
