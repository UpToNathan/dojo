# Request code for join requests

Status: open

A join request is checked today by the node key's fingerprint alone, so the host key or input key in an unsigned request file could be swapped in transit while the node key still matches. Add a short **request code** over the whole request (name, node key, input key, host key, routes): sixteen characters in four groups, printed by `dojo-mesh join` and accepted by `dojo-mesh approve --code`. `approve` keeps accepting the node key's fingerprint.

## Acceptance

- The same request gives the same code on both machines; changing any field changes it.
- `approve` without a matching code or fingerprint refuses, as now.
- `join` prints the code first.
- Tests cover canonical ordering of routes and absent input key.
