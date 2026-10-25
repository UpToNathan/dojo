# Mesh over OpenSSH: map

Spec: [spec.md](spec.md). Decision: [ADR-0009](../../docs/adr/0009-a-signed-roster-is-a-second-membership-method-over-openssh.md) (proposed).

## Decisions so far

- 2026-10-25, operator: any reachable address (pluggable routes); members get a full user shell; root only through `sudo` (password; a YubiKey touch only where a node opts in); remote input uses a dedicated restricted key with no touch; node keys live in 1Password's agent; signing keys may be held on a YubiKey, in 1Password or anywhere an SSH agent or key file can sign, in any number and order (none required); Tailscale stays as an optional method.
- Open: where the roster remote lives, if anywhere; whether Cloudflare becomes a route later.

## Known state of the machines (2026-10-25)

- Venus: OpenSSH 10.6, 1Password agent running with keys; no `sshd`; no `pam_u2f`/`libfido2`; no YubiKey (the operator has none).
- Jupiter: OpenSSH 10.6; 1Password 8.12.40 installed through Nix and running but the SSH agent never enabled; `sshd` active on all interfaces with password auth not disabled; no `pam_u2f`.

## Order

01 seams, then 02 and 05 in parallel, 03, 04 and 06, then 07.
