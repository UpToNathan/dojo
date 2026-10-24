# Clipboard History command

Status: resolved

Blocked by: 01

A view over what `clipvault` already records. Search, re-copy on Enter, pin. Decide how password-manager copies are excluded. No new recording daemon.

## Comments

Built over clipvault's CLI. Images are not listed; entries that look like secrets are hidden; pins last as long as clipvault keeps the entry. Gap: a secret copied from an app that sets no password hint and matches no known shape is still recorded; adding `--ignore-pattern` to the clipvault service would close it.
