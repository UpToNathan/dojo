# Shared Nix cache

One host compiles each Nix package once; the others download it. A node that declares
the `nix-cache` capability in its mesh configuration (see [mesh.md](mesh.md)) runs
`dojo-nix-cache`, a [harmonia](https://github.com/nix-community/harmonia) user service from
`cache-home-manager.nix`. It serves that host's `/nix/store` over HTTP, signs each
package on the fly, and listens on the Tailscale address only (never the LAN).

```toml
# ~/.config/dojo/mesh.toml on the cache host
[capability.nix-cache]
port = 5000                                    # default 5000
key-name = "myhost-dojo-1"                     # only used when the key is first generated
public-key-file = "~/.config/dojo/nix-cache/public-key"   # published to peers as public-key
```

Every other node asks the mesh who offers the capability and records the answer with
`scripts/update-nix-cache.sh`, which writes `/etc/nix/dojo-cache.conf`; `system-config/nix/nix.conf`
includes it. Nix still asks `cache.nixos.org` and Cachix first and the peers for whatever
neither has. `connect-timeout = 5` keeps a laptop that is off the tailnet from waiting on them.

A cache's key is trusted for every package it serves, so `update-nix-cache.sh` shows each new or
changed key and waits for your yes (`--yes` skips the question). Peers that are offline are kept;
`--prune` rebuilds the file from the peers that answer now.

- The signing key is generated on first start in `~/.config/dojo/nix-cache/` and never
  tracked. Replacing it (a rebuilt cache host) changes the published `public-key`: run
  `scripts/update-nix-cache.sh` on each host, approve the new key and restart `nix-daemon`.
- Anything on the tailnet that can reach the cache host's port 5000 can read every path
  in its store (firewalld trusts `tailscale0`). Keep secrets out of the store, or limit
  the port in the Tailscale ACL.
- The service starts with the user's systemd session. It stops when that session ends
  unless lingering is enabled (`loginctl enable-linger`).
- Check it: `dojo-mesh find nix-cache`, then `curl http://<host>:<port>/nix-cache-info` and
  `nix path-info --store http://<host>:<port> <store path>`.
