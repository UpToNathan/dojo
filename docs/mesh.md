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
dojo-mesh windows [--json]             # this node's windows
dojo-mesh send [--window ID] (--to PEER | --dry-run) [--copy-missing] [--keep-source] [--json]
dojo-mesh close-source --window ID [--tab-title TITLE]
dojo-mesh remote PEER                  # drive a peer with this cursor and keyboard
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

# Peers whose cursor and keyboard this node accepts (ADR-0007). Nobody is accepted unless named.
accept-input = ["laptop"]

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

## Sending a window

`dojo-mesh send` opens what a window shows on a peer and then closes it here. The panel (`dojo-desktop --send-window`, bound to Super+F11) does the same: it lists your windows and the machines that are online, and shows exactly what the selected window would send before you send it. `--dry-run` prints that line without calling anyone.

What moves depends on the application. A **Helium** window sends only the page in front, found by matching the window's title in the browser's history, and **only that tab closes** here: the window is checked to still show that page, then sent Ctrl+W through an input seat of its own (Niri's agent input, which needs `niri msg yield-human-input` first because you have just used that window; only your own calls do that, never the agent's). The window and its other tabs stay, and your focus doesn't move. **kitty** sends its directory, and nvim's file and line, or yazi's directory, when one of those is in front; the window closes whole, the shell starts fresh and anything else that was running is named as staying behind. Any other application is relaunched from its desktop entry. kitty is read through its remote control, which is limited to `ls` with a per-machine password in `~/.config/kitty/dojo-remote-control.conf`; see the spec in `.scratch/mesh-boundaries/spec.md`.

The peer only opens a web page, a directory, a file at a line, or an application it already has; it never runs a command it is sent, and it refuses while its screen is locked. It reports success only once a new window appears (a page opens as a tab in a running browser, which makes no new window). If the peer lacks a file, nothing is sent; `--copy-missing` copies the file across first. The panel sends with `--keep-source`, shows what was sent, then closes the original after a few seconds unless you press Esc, and lets go of the keyboard before closing it.

## Taking the cursor and keyboard

`dojo-mesh remote PEER` (or the "Take my cursor and keyboard" switch in the send panel, which starts it after the send) hands this node's pointer and keyboard to a peer until you press **Super+F11** again, or **Ctrl+Alt+Shift+Esc**, which is always there for when something goes wrong. Neither chord reaches the peer.

On this node `dojo-input-capture` covers the focused output with a transparent surface that has the keyboard exclusively, locks the pointer (so motion keeps coming at the screen's edge) and hides the cursor. Nothing reaches local windows meanwhile. It writes one JSON event per line (relative motion, buttons, scrolling, evdev keys), which `dojo-mesh remote` carries over `tailscale ssh` to `dojo-mesh inject` on the peer. Whatever ends the session returns your input at once: the chord, the connection dropping, the peer refusing, a lock screen taking the surface, or the helper exiting. Held keys and buttons are released on both sides.

The peer replays the events through an agent input seat in Niri: a cursor of its own that moves, clicks, scrolls and types into whatever it is over, without changing the peer's own focus. Two consequences of using the seat: it is a second cursor, drawn in the seat's style, not the peer's own; and it cannot type Niri's own shortcuts (Super binds), since seat keys never reach compositor shortcuts. Typing into applications works.

The peer accepts this only from nodes it names in `accept-input`, and learns who is sending from Tailscale's answer about the connection (`tailscale whois` of the SSH session), never from what the sender says. It refuses while its screen is locked, refuses the agent, and refuses anything not started over a peer's connection. It asks Niri to end its pause on windows its person has used (`yield-human-input`), as the operator's own call. The agent has no tool for any of this.

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

Tailscale does not report peers' client versions, so version drift is not checked.

## Keeping real names out

`scripts/check-mesh-names.sh` (run it inside `devenv`) reads this machine's own device names, tailnet name and user name and fails if any appears in the mesh source, docs or tests. Tests and examples use invented names.

## Install and test

```sh
devenv shell -- cargo test --manifest-path crates/dojo-mesh/Cargo.toml
devenv shell -- scripts/install-helpers.sh dojo-mesh      # installs to ~/.local/share/dojo/bin
node --test tools/speech/mesh-extension.test.mjs
```
