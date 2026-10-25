# Mesh

A Dojo node can see which other devices belong to its mesh and whether they are healthy. This is read-only: nothing here connects to a peer or changes the network. Terms are in [GLOSSARY.md](../GLOSSARY.md#mesh); the decision behind them is [ADR-0005](adr/0005-tailscale-identity-is-node-identity-and-permission.md).

```sh
dojo-mesh status [--all] [--json]   # this node and the peers it admits
dojo-mesh doctor [--json]           # what is wrong or doubtful; exits 1 on a problem
dojo-mesh --config PATH <command>   # use another config file
```

`status --all` also lists the devices on the network that the node does not admit, and why. Both commands work with no configuration and without Tailscale installed: the answer is then a mesh of one.

## Configuration

`$XDG_CONFIG_HOME/dojo/mesh.toml` (`~/.config/dojo/mesh.toml`). A missing file is an empty configuration. Every key is optional, and every node decides alone whom it admits.

```toml
# The Tailscale tag that marks a device as a candidate node.
membership-tag = "tag:dojo"

# Admitted whether or not they carry the tag. Short host names or full network names, any case.
allow = ["laptop"]

# Refused whether or not they carry the tag. Wins over allow. A name cannot be in both.
deny = ["old-box"]
```

A node is always a member of its own mesh. Without a tag or an allow list it admits no one.

## How membership is read

The Tailscale method reads `tailscale status --json` from the local client, so it needs no API credentials and no network access of its own. It looks only at what the client reports: host names, tags, online state and key expiry. A stopped, logged-out or missing client leaves the node alone in its mesh, and `doctor` says why.

## What `doctor` checks

- this node does not carry the membership tag, so peers using it will not admit this node (problem)
- a member's Tailscale key has expired (problem)
- a member is offline, or its key expires within 14 days (warning)
- a peer is admitted by the allow list only, not by the tag (warning)
- an allow or deny entry matches no device (warning)
- no tag or allow list is configured, or the network cannot be read (warning)

Tailscale does not report peers' client versions, so version drift is not checked.

## Keeping real names out

`scripts/check-mesh-names.sh` (run it inside `devenv`) reads this machine's own device names, tailnet name and user name and fails if any appears in the mesh crate, this page, the ADR or `.scratch/mesh`. Tests and examples use invented names.

## Install and test

```sh
devenv shell -- cargo test --manifest-path crates/dojo-mesh/Cargo.toml
devenv shell -- scripts/install-helpers.sh dojo-mesh   # installs to ~/.local/share/dojo/bin
```
