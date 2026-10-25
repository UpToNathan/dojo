# Shell layer

Type: task
Status: resolved
Blocked by: 01

Add `dojo mesh exec` and `dojo mesh cp` with the stdin envelope, provenance, fail-fast on offline peers and exit 3 for needs-a-human. Show peers in a launcher view. Decide the transport first (see the map's fog).

## Answer

Added `dojo-mesh exec`, `cp` and `task` over `tailscale ssh` ([ADR-0006](../../../docs/adr/0006-peers-are-reached-with-tailscale-ssh.md)). Arguments are quoted and never shell-interpreted; the caller, session and origin reach the peer as `DOJO_FROM`, `DOJO_SESSION` and `DOJO_ORIGIN`; offline, unknown and unadmitted peers fail at once with exit 4; Tailscale's sign-in request is detected and exits 3 with the page instead of hanging; `--timeout` ends a slow call. `cp` sends one file with its permission bits and replaces atomically, and a failed pull leaves nothing behind. The stdin JSON envelope from Hona/mesh was dropped in favour of environment variables (nothing needs structured input yet). Every call goes to an audit log before and after it runs, and the agent's calls are refused if they cannot be logged. End-to-end tests run the real binary against a fake `tailscale`, and a live round trip to a real peer worked. The launcher has a Mesh command listing the machines.
