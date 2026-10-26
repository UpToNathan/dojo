# Rendezvous: 1Password carries the invite

Status: resolved

Replaces the join-request file, the mesh inbox and Downloads. A small `Rendezvous` interface (create, list, get, update, delete an item of text fields) with two carriers: 1Password (`op item …`, a vault from `rendezvous-vault`) and a directory (for tests and for people without 1Password). The protocol on top: an invite item (operator, trusted signers, expiry, status), a request field the new machine posts (the signed join request), a response field the operator posts (the roster and the signers), a status that moves open, requested, approved or denied, and deletion at the end.

## Acceptance

- `dojo-mesh invite` posts an invite; `dojo-mesh join --invite` on a node with keys finds it, posts its signed request and waits; `dojo-mesh requests` lists requests waiting; `dojo-mesh approve --invite ID --yes` signs, posts the answer and the new node installs the roster and trusts the signers; both ends clean up.
- An expired invite, a second request to the same invite, a request whose signature does not verify, and an invite from a vault the node cannot read are refused with a reason.
- Tests run the whole exchange on the directory carrier, and the 1Password carrier against a fake `op`.
