# Migration and documentation

Type: task
Status: resolved
Blocked by: 03, 04, 06

Choose the membership method in `mesh.toml`, keep the Tailscale method working, and describe moving Venus and Jupiter from tags to a roster without a gap. Update `docs/mesh.md`, ADR-0005/0006 pointers and the README. Remove any wording that assumes Tailscale from the core's documentation.

## Resolution

`docs/mesh.md` now covers the method choice, the signed roster, reaching peers over OpenSSH, enrolment and key custody, host setup, and a step-by-step move from a Tailscale mesh to a roster that can be undone at any point; ADR-0005 and ADR-0006 point to ADR-0009, which is accepted; the glossary has the input key and the join request. `roster sync` now holds back an update that makes entries stop verifying, which closes the "a remote can remove members from view" gap. Tailscale remains the default method and its tests are unchanged and green.

What no test here could exercise, and a person must still do: sign in to 1Password on Jupiter and turn on its SSH agent, then `dojo-mesh init` on both machines; create or choose a signing key; give Jupiter's `sshd` a reachable route (its firewall drops the LAN and Tailscale owns port 22 on the tailnet address); and install the new Niri on the receiving machine and log in again.
