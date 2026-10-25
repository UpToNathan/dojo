# Read-only mesh

Type: task
Status: resolved

Add node-local mesh configuration (membership tag, allow and deny overrides), Tailscale tag membership from the local client, and `dojo-mesh status`. Add `dojo-mesh doctor`: offline peers, key expiry and stale overrides (Tailscale does not report peers' versions, so drift is not checked). Add the no-real-names check. Cover the mesh of one and the no-Tailscale case in tests.

## Answer

Added the standalone `crates/dojo-mesh` crate (pure config, device, roster and doctor core; a Tailscale adapter over `tailscale status --json`; the `dojo-mesh` binary) with 26 unit tests and 4 CLI tests that run without Tailscale. `scripts/check-mesh-names.sh` fails if this machine's own device, tailnet or user names appear in the mesh source, and was shown to fail on a planted name. Run against the live tailnet: a tag-less config gives a mesh of one with a warning, an allow list admits the named peer, and an unavailable client gives a mesh of one. Documented in [docs/mesh.md](../../../docs/mesh.md).
