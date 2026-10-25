# Mesh

A Dojo node can see which other devices belong to its mesh, reach them, and use what they offer. One node on its own is a complete mesh. Terms are in [GLOSSARY.md](../GLOSSARY.md#mesh); the decisions behind them are [ADR-0005](adr/0005-tailscale-identity-is-node-identity-and-permission.md) (identity and permission are Tailscale's) and [ADR-0006](adr/0006-peers-are-reached-with-tailscale-ssh.md) (how peers are reached).

## Commands

```sh
dojo-mesh status [--all] [--json]      # this node and the peers it admits
dojo-mesh doctor [--json]              # what is wrong or doubtful; exits 1 on a problem
dojo-mesh describe                     # what this node offers, as JSON
dojo-mesh find CAPABILITY [--json | --tsv] [--peers-only]
dojo-mesh exec [--timeout SECS] PEER -- COMMAND [ARG...]
dojo-mesh cp SRC DST                   # one file; exactly one side is PEER:PATH
dojo-mesh task PEER TEXT...            # hand a task to the peer's own agent
dojo-mesh pause | resume               # the agent's peer access
dojo-mesh audit [--last N] [--json]    # what this node has sent to peers
dojo-mesh enroll [--tag TAG] [--expires SECS]            # operator only
dojo-mesh policy show | validate FILE | apply FILE       # apply is operator only
dojo-mesh --config PATH <command>      # use another config file
```

Exit status: `0` done; `1` the command ran and failed (the message names the peer's status); `2` usage or configuration; `3` a person has to act; `4` refused or unreachable.

Status 3 means Tailscale asked for a fresh sign-in (the page is in the message), or an operator-only step had no operator. Status 4 covers a peer that is offline, is not in the mesh, is this node, has its call timed out, or an agent call while peer access is paused. An offline peer is refused at once, not after a timeout.

The launcher has a **Mesh** command listing the same machines; Enter copies `tailscale ssh <name>`.

## Configuration

`$XDG_CONFIG_HOME/dojo/mesh.toml` (`~/.config/dojo/mesh.toml`). A missing file is an empty configuration. Every key is optional and every node decides alone whom it admits.

```toml
# The Tailscale tag that marks a device as a candidate node.
membership-tag = "tag:dojo"

# Admitted whether or not they carry the tag. Short host names or full network names, any case.
allow = ["laptop"]

# Refused whether or not they carry the tag. Wins over allow. A name cannot be in both.
deny = ["old-box"]

# What this node offers. Names and attributes are lowercase words and dashes; values are text,
# numbers or true/false. An attribute named X-file is published as X, holding the file's contents.
[capability.nix-cache]
port = 5000
public-key-file = "~/.config/dojo/nix-cache/public-key"

[capability.voice-agent]
command = "/path/to/dojo-voice"      # what `dojo-mesh task` runs here, with `send TEXT`

# Screen edges joined to a peer's. Both nodes declare the matching boundary and accept the other's
# input; `dojo-mesh doctor` says what is missing. Outputs are the compositor's names (`niri msg outputs`).
accept-input = ["laptop"]

[[boundary]]
output = "eDP-1"
edge = "right"                       # left, right, top or bottom
peer = "laptop"
peer-output = "DP-1"
peer-edge = "left"

# Only for the operator-only commands. These name where the credential is; they never hold it.
# A reference is env:NAME, file:/absolute/path or op://vault/item/field (1Password).
[control-plane]
tailnet = "-"                        # "-" is the tailnet the credential belongs to
client-id = "op://Personal/Tailscale OAuth/username"
client-secret = "op://Personal/Tailscale OAuth/credential"
# or: api-token = "env:TAILSCALE_API_TOKEN"
```

A node is always a member of its own mesh. Without a tag or an allow list it admits no one.

## Membership

The Tailscale method reads `tailscale status --json` from the local client, so it needs no API credentials. It looks only at host names, tags, online state and key expiry. A stopped, logged-out or missing client leaves the node alone in its mesh, and `doctor` says why. The tailnet's policy decides what a node may then do on a peer: Dojo adds no permission layer.

## Reaching peers

`exec`, `cp` and `task` use `tailscale ssh`. The remote command gets `DOJO_FROM`, `DOJO_SESSION` and `DOJO_ORIGIN` in its environment. Arguments are quoted, never interpreted by a shell. `cp` sends one regular file, keeps its permission bits, and replaces the destination atomically; copying from a peer writes through a temporary file, so a failed transfer leaves nothing behind.

Every call is appended to `$XDG_STATE_HOME/dojo/mesh/audit.jsonl` before it starts and again when it ends. The agent's calls are refused if they cannot be logged. `dojo-mesh pause` stops the agent's calls (those marked `DOJO_MESH_ORIGIN=agent`) until `resume`; it is a brake, not a security boundary, because anything running as the user can still run `tailscale ssh` itself.

## Capabilities

`find` asks each online peer to `describe` itself (by running `dojo-mesh describe` there), so a peer needs `dojo-mesh` installed in `~/.local/share/dojo/bin`. A peer that cannot be asked is reported, not guessed. The Nix cache is the first consumer: see [nix-cache.md](nix-cache.md).

## Boundaries

A boundary joins an edge of one of this node's outputs to an edge of a peer's output (see [the mesh-boundaries spec](../.scratch/mesh-boundaries/spec.md)). It is configuration only so far: `doctor` checks both ends, and `describe` publishes this node's outputs, boundaries and `accept-input` so a peer can check. Nothing crosses the edge yet.

## The agent

When `dojo-mesh` is installed, the agent has `mesh_status`, `mesh_find`, `mesh_exec`, `mesh_copy` and `mesh_task`, which call it as the agent. It has no tool to pause or resume peer access, enrol a node or change the policy. See "Other Dojo nodes" in `tools/speech/assistant.md`.

## The control plane (operator only)

`enroll` mints a one-use, pre-approved key for a device with a tag. `policy apply FILE` checks a candidate with Tailscale, shows the difference from the current policy, and replaces it only if you say yes and nobody changed the policy meanwhile. Both refuse unless a person is at a terminal and the call is not the agent's. `policy show` and `policy validate` change nothing and are open to the agent.

Credentials are read from their reference for the one command and given to `curl` on its standard input, so they never appear in a process listing. An OAuth client needs the scopes `policy_file:read` (show, validate), `policy_file` (apply) and `auth_keys` (enroll). A personal API token carries your own permissions.

## What `doctor` checks

- this node does not carry the membership tag, so peers using it will not admit this node (problem)
- a member's Tailscale key has expired (problem)
- a member is offline, or its key expires within 14 days (warning)
- a peer is admitted by the allow list only, not by the tag (warning)
- an allow or deny entry matches no device (warning)
- no tag or allow list is configured, or the network cannot be read (warning)
- a boundary names an output a node does not have, a peer that is not in the mesh, a peer that does not declare the matching boundary back, or input that either end does not accept (problem); a boundary whose peer is offline or cannot be asked is a warning, since the other end was not checked

Tailscale does not report peers' client versions, so version drift is not checked.

## Keeping real names out

`scripts/check-mesh-names.sh` (run it inside `devenv`) reads this machine's own device names, tailnet name and user name and fails if any appears in the mesh source, docs or tests. Tests and examples use invented names.

## Install and test

```sh
devenv shell -- cargo test --manifest-path crates/dojo-mesh/Cargo.toml
devenv shell -- scripts/install-helpers.sh dojo-mesh      # installs to ~/.local/share/dojo/bin
node --test tools/speech/mesh-extension.test.mjs
```
