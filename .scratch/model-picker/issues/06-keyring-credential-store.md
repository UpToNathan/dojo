# Store Dojo credentials in the system password store

Status: resolved

Implement the credential storage in [the spec](../spec.md). Add a keyring-backed
credential store behind the interface `DojoCredentials` already exposes (`read`,
`list`, `modify`, `delete`), using `secret-tool` with attributes `service=dojo` and
`account=<id>` and the credential on standard input and output.

Check the collection's `Locked` state before any lookup so discovery never waits on
an unlock dialog, bound every call with a timeout, and cache reads briefly. Keep
writes on the existing serialized chain. Fall back to the private 0600 file only
when no Secret Service is present, and expose which backend is active so the account
row can say so. Migrate existing `credentials.json` entries once, verifying the
keyring read-back before removing them.

Verify with a fake `secret-tool` executable: store and lookup round trip, a missing
item, a locked collection, a timeout, concurrent writes, secrets never in argv, and
migration that leaves the file untouched when the read-back fails. Add one gated
live check against the real keyring that cleans up its item.

## Result

Moved `DojoCredentials` behind `tools/speech/credentials.mjs` and wired all account
reads/writes through it.

- The store probes Secret Service's default collection and reads its `Locked` state
  before every keyring lookup. Calls have 1.5-second timeouts; reads are briefly
  cached and mutations/migrations share one serialized chain.
- When available, secrets use `secret-tool` with `service=dojo` and
  `account=<account-id>` attributes. Credential JSON is stdin/stdout only; account
  IDs are the only credential-related arguments. Borrowed subscriptions remain
  read-only and are not copied to the keyring.
- With no Secret Service, the private 0600 file is the visible fallback. Existing
  owned entries migrate only after keyring read-back verification; failed migration
  leaves the source file untouched. Account metadata reports the backend.

Validation: fake command-runner tests cover missing/locked service, lock transitions,
timeouts, serialized concurrent writes, file mode, borrowed snapshots, and failed
migration. A separate fake `gdbus`/`secret-tool` executable test exercises the real
child-process stdin/argv boundary. The full speech suite passes (178 passed, 1 gated
live-keyring check skipped). A real round trip can be enabled with
`DOJO_TEST_LIVE_KEYRING=1`.

## Comments
