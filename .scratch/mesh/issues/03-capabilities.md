# Capabilities

Type: task
Status: resolved
Blocked by: 02

Let a node describe its capabilities. Replace the named server in `nix-cache.json` with whichever peer offers the Nix cache capability, so the cache is the first consumer.

## Answer

A node declares capabilities in its mesh configuration; `dojo-mesh describe` publishes them (an `X-file` attribute is published as `X` with the file's contents), and `dojo-mesh find CAPABILITY` asks each online peer. `nix-cache.json` is gone: the cache host is whichever node declares `capability.nix-cache` (`cache-home-manager.nix` reads it), and every other node runs `scripts/update-nix-cache.sh`, which records the offers in `/etc/nix/dojo-cache.conf` after the operator approves each new or changed signing key. See [nix-cache.md](../../../docs/nix-cache.md).
