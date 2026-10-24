# Shared Nix cache

One host compiles each Nix package once; the others download it. The host named by
`server` in `nix-cache.json` (Jupiter) runs `dojo-nix-cache`, a
[harmonia](https://github.com/nix-community/harmonia) user service from
`cache-home-manager.nix`. It serves that host's `/nix/store` over HTTP, signs each
package on the fly, and listens on the Tailscale address only (never the LAN).
`system-config/nix/nix.conf` lists `http://jupiter:5000` as an extra substituter with
the matching public key, so every host asks `cache.nixos.org` and Cachix first and
Jupiter for whatever neither has. `connect-timeout = 5` keeps a laptop that is off the
tailnet from waiting on it.

- The signing key is generated on first start in `~/.config/dojo/nix-cache/` and never
  tracked. Replacing it (a rebuilt cache host) needs the new `public-key` pasted into
  `extra-trusted-public-keys` in `system-config/nix/nix.conf`, then
  `scripts/install-system-config.sh` and a `nix-daemon` restart on each host.
- Anything on the tailnet that can reach the cache host's port 5000 can read every path
  in its store (firewalld trusts `tailscale0`). Keep secrets out of the store, or limit
  the port in the Tailscale ACL.
- The service starts with the user's systemd session. It stops when that session ends
  unless lingering is enabled (`loginctl enable-linger`).
- Check it: `curl http://jupiter:5000/nix-cache-info`, then
  `nix path-info --store http://jupiter:5000 <store path>`.
