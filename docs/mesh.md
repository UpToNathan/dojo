# Mesh

A Dojo node can see which other devices belong to its mesh, reach them, and use what they offer. One node on its own is a complete mesh. Terms are in [GLOSSARY.md](../GLOSSARY.md#mesh); the decisions behind them are [ADR-0005](adr/0005-tailscale-identity-is-node-identity-and-permission.md) (identity and permission are Tailscale's) and [ADR-0006](adr/0006-peers-are-reached-with-tailscale-ssh.md) (how peers are reached), for a mesh formed by Tailscale. A mesh can instead be formed by a signed roster over the system OpenSSH ([ADR-0009](adr/0009-a-signed-roster-is-a-second-membership-method-over-openssh.md)); `method` in the configuration chooses, and Tailscale is the default.

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
dojo-mesh roster [check | sync | authorize | export | receive]   # signed-roster method: what counts and what is ignored; take it from git or from members; let the members in; carry it as a bundle
dojo-mesh onboard [--sign yubikey|1password|KEY]... [--route HOST[:PORT]]...   # signed-roster method: set this node up in one go (see Onboarding a machine)
dojo-mesh init | join | approve | renew | rm | leave | trust   # signed-roster method: enrolment and trust (see below)
dojo-mesh enroll [--tag TAG] [--expires SECS]            # operator only
dojo-mesh policy show | validate FILE | apply FILE       # apply is operator only
dojo-mesh --config PATH <command>      # use another config file
```

Exit status: `0` done; `1` the command ran and failed (the message names the peer's status); `2` usage or configuration; `3` a person has to act; `4` refused or unreachable.

Status 3 means a person has to act: Tailscale asked for a fresh sign-in (the page is in the message), the SSH agent holding this node's key is locked or off (the step is in the message), a signing key needs its touch or unlock, or an operator-only step had no operator. Status 4 covers a peer that is offline, is not in the mesh, is this node, has its call timed out, or an agent call while peer access is paused. An offline peer is refused at once, not after a timeout.

The launcher has a **Mesh** command listing the same machines; Enter copies `tailscale ssh <name>` (it knows only the Tailscale method).

## Configuration

`$XDG_CONFIG_HOME/dojo/mesh.toml` (`~/.config/dojo/mesh.toml`). A missing file is an empty configuration. Every key is optional and every node decides alone whom it admits.

```toml
# How this node forms its mesh: "tailscale" (the default) or "roster" (see Membership).
method = "tailscale"

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

The roster method adds `roster-dir`, `signers-file`, `node-name`, `node-key-file`, `input-key-file` and `agent-socket`; see [The signed roster](#the-signed-roster).

## Membership

A mesh method fills three seams (`crates/dojo-mesh/src/method.rs`): **membership** (who the members are and how to reach them), **identity** (which node an incoming connection is, taken from the network, never from the caller) and the **channel** (running a command or copying a file on a peer, over a **route**: a name and port, or a command that opens the connection). Everything above them sees only these. Tailscale is the one method today; a node with no method is a mesh of one. [ADR-0009](adr/0009-a-signed-roster-is-a-second-membership-method-over-openssh.md) proposes a second.

The Tailscale method reads `tailscale status --json` from the local client, so it needs no API credentials. It looks only at host names, tags, online state and key expiry. A stopped, logged-out or missing client leaves the node alone in its mesh, and `doctor` says why. The tailnet's policy decides what a node may then do on a peer: Dojo adds no permission layer.

### The signed roster

With `method = "roster"` the members are not read from a network but from signed files, so the mesh works on any network ([ADR-0009](adr/0009-a-signed-roster-is-a-second-membership-method-over-openssh.md)). Calls to members go over the system `ssh` ([Reaching peers over OpenSSH](#reaching-peers-over-openssh)).

```toml
method = "roster"
roster-dir = "/home/me/.local/share/dojo/mesh/roster"        # the default if omitted
signers-file = "/home/me/.config/dojo/mesh/allowed_signers"  # the default if omitted
node-name = "orchard"                                        # this node's name in the roster; default: the host name
node-key-file = "/home/me/.config/dojo/mesh/node_key.pub"    # public half of this node's key; the default if omitted
input-key-file = "/home/me/.config/dojo/mesh/input_key.pub"  # public half of the remote-input key; the default if omitted
agent-socket = "/home/me/.1password/agent.sock"              # the SSH agent holding both; 1Password's by default
```

The roster directory holds `devices/NAME.json` and `revocations/NAME.json`, each with its signature beside it as `NAME.json.sig`. A device entry:

```json
{"v": 1, "kind": "device", "name": "orchard", "node_key": "ssh-ed25519 AAAA…", "host_key": "ssh-ed25519 AAAA…",
 "routes": [{"host": "192.0.2.10", "port": 22}], "issued": "2026-10-25T12:00:00Z", "expires": "2027-01-23T12:00:00Z"}
```

A revocation is `{"v": 1, "kind": "revoke", "key": "ssh-ed25519 AAAA…", "issued": "…"}`; any entry carrying that key stops counting. Entries are signed with `ssh-keygen -Y sign -f SIGNING_KEY -n dojo-mesh-roster FILE`, which writes `FILE.sig`; a key on a YubiKey, in 1Password's agent or in a file all work the same way.

The trusted signing keys are this node's own file, never part of the roster: one line each, `operator namespaces="dojo-mesh-roster" ssh-ed25519 AAAA…`. Without it the node is a mesh of one and says why. An entry counts only if it is signed by one of those keys, has not expired, is not issued in the future, carries no revoked key, is well formed (a lowercase name of two or more characters, keys of one known type on one line, plain route hosts) and is the newest for its name. Everything else is ignored, listed with its reason by `dojo-mesh roster check` and `doctor`, and `roster check` exits 1 while anything is ignored.

**Keeping the roster in step.** The roster is signed files, so the carrier does not have to be trusted: a node keeps a file only if it verifies against its own trusted signing keys, and for each name it keeps the newest. A roster that is not a git repository moves between members by itself. `dojo-mesh roster sync` asks each member that is online for its files over the OpenSSH channel (`roster export` on the other side) and keeps what verifies and is new; `approve`, `renew` and `rm` send the changed roster to the members that are online (`roster receive` on the other side), and one that was offline takes it with `roster sync` when it is back. Nothing a merge does deletes a file, since a node is removed by a signed revocation, so a member can withhold files from another but never remove or forge one; files that arrive without a trusted signature are not written and `sync` exits 1 naming them. The first copy to a new node cannot go this way, because the host keys it would pin come from the roster itself: carry it once with `dojo-mesh roster export > roster.json` where the roster is and `dojo-mesh roster receive < roster.json` on the new node (any way of moving the file does, a stick included), after the new node has trusted the signing keys. A roster that is a git repository with a remote still uses it, as follows.

For a roster kept in git, `dojo-mesh roster sync` pulls the directory from its remote, fast-forward only; a rewritten remote history is refused. Whoever can write to that remote cannot add a member or change one, because they cannot sign. They could try to make members drop out of view by altering or removing entries, so `sync` checks the result and, when files that did not verify before now do not, puts the roster back where it was and says which files (an ordinary revocation or expiry is accepted). A peer's reachability is a short TCP connection to its first route, so an offline peer fails at once; an entry with no routes shows as offline.

### Reaching peers over OpenSSH

With the roster method, `exec`, `cp`, `task`, `send` and `remote` run the system `ssh` with a complete set of options of their own and `-F /dev/null`, so nothing from the user's `~/.ssh/config` (proxies, forwarded agents, control sockets) applies. Every connection is `BatchMode` with public-key authentication only, `IdentitiesOnly` with this node's *public* key file and the agent that holds the private half (1Password's by default, so the key never touches disk), `StrictHostKeyChecking=yes` against the host key the signed roster pins for that peer, a 5-second connect timeout and keepalives. The pinned key is written to a private file under `$XDG_DATA_HOME/dojo/mesh/known_hosts/NAME` and addressed by the alias `dojo-NAME`, so a peer's address can change without its pin changing. A peer's route is the first of its roster routes: an address and optional port, or a command that opens the connection (an SSH `ProxyCommand`).

Before it connects, the channel asks the agent whether it offers the key. If 1Password is locked, not running, or the key is missing, no connection is attempted: the command exits 3 and says what to do ("unlock 1Password on this machine", "turn on Settings > Developer > SSH Agent"), and `doctor` warns the same way. `remote` uses the node's separate input key instead of its node key.

`dojo-mesh roster authorize` writes the members into the user's `~/.ssh/authorized_keys` as a block between `# BEGIN dojo-mesh` and `# END dojo-mesh` lines (and `roster sync` does it after pulling). Each member's node key gets a full shell as this user; each member's input key is `restrict,command="…/dojo-mesh inject --as NAME"`, so it can only send input, whatever it asks for. The block is replaced or removed whole, the lines around it are never touched, a copy of the previous file is kept beside it as `authorized_keys.dojo-mesh.bak`, and a revoked or expired member disappears from it at the next run. The mesh does not edit `sshd_config`. Who a connection is under this method comes from that forced command, checked against the roster: a name that is not a current member is nobody.

### Onboarding a machine

Setting up a machine with Dojo ends with joining the mesh: `scripts/bootstrap-host onboard` (`bootstrap-host all` offers it at a terminal, and skips it for a machine already on the roster). It is `scripts/onboard-mesh.sh`, which asks for the machine's name, the address the others reach it at and the keys it signs with, then does the steps below in order, and repeating it changes nothing that is already done.

1. Checks that 1Password's SSH agent answers, before anything is changed.
2. Sets up `sshd` for the mesh (`install-mesh-sshd.sh`: public keys only, no root login) on its own port, 2222 by default so it never meets Tailscale SSH on port 22, and starts it if it is not running. This is the step that asks for `sudo`.
3. Runs `dojo-mesh onboard`: writes `method`, `node-name` and `signing-keys` into `mesh.toml` (the rest of the file is kept), gives the node its keys in 1Password, makes or finds the signing keys, trusts them, and signs the node into its roster.

`--sign` names the signing keys in the order to try them, so `--sign yubikey --sign 1password` makes the YubiKey the primary and 1Password the alternate. The YubiKey key is a resident `ed25519-sk` credential named `ssh:dojo-mesh`: on a second machine, onboarding takes it back from the key (`ssh-keygen -K`) instead of making another, so one YubiKey is one signing identity. The 1Password one is an SSH key item there, used through the agent. A machine given no `--sign` signs nothing: it writes `NAME.join.json` for a machine that does, and the output lists the steps that follow (approve there, trust and sync here). A route is an address the others can reach and the port from step 2; if the machine's firewall drops that network, the port needs opening there (a tailnet address needs nothing where `tailscale0` is trusted).

### Enrolling a node and holding the keys

A signing key can be held anywhere an SSH agent or a key file can sign with it: a YubiKey (`ssh-keygen -t ed25519-sk -O resident`, which needs a touch), a key in 1Password's agent, another agent, or a file. A node lists the ones it signs with, in the order to try them, as `signing-keys` (absolute paths, none required): a private key file, or a *public* key file whose private half the agent holds. The first that signs wins; if none can, the command exits 3 and says why for each. Which keys a node *believes* is separate and its own: `dojo-mesh trust list` shows them with fingerprints and whether each is a hardware key or software; `trust add KEY --fingerprint SHA256:…` adds one only when the fingerprint matches the one the person compared on the other device, which is also how a replacement for a lost signing key is accepted; `trust remove SHA256:…` drops one. `doctor` warns when every trusted key is held in software, since a software key whose approval the agent remembers can sign while the computer is unlocked.

1. **Keys.** `dojo-mesh init` creates two keys in 1Password through its `op` command line (the node key and a separate input key, in `--vault` if given) and records their public halves at `node-key-file` and `input-key-file`. Keys made elsewhere are registered with `init --custody existing --node-key PUB [--input-key PUB]`. 1Password must be unlocked and its SSH agent on (Settings > Developer > Use the SSH agent), and the vault must be one the agent shares.
2. **Request.** On the new node, `dojo-mesh join --route HOST[:PORT]` writes `NAME.join.json` and shows the fingerprints of its node key, input key and SSH host key (`/etc/ssh/ssh_host_ed25519_key.pub`, or `--host-key-file`).
3. **Approval.** On a node that signs, `dojo-mesh approve NAME.join.json` shows the fingerprints and stops (exit 3). After comparing the node key's fingerprint with what the new node displayed, run it again with `--fingerprint SHA256:…` and `--days N` (90 by default, 366 at most). A name already in the roster with a different key is refused without `--replace`, and so is a request carrying a revoked key. The signed entry is committed to the roster (and pushed when it is a git repository with a remote), and this node's `authorized_keys` block is refreshed.
4. **The new node** trusts the signer (`trust add`, with the fingerprint it was shown), pulls the roster (`roster sync`, which also refreshes its `authorized_keys`) and the two nodes can reach each other.
5. **Later.** `renew NAME` signs the same entry for a fresh period before it expires (`doctor` warns when one is close), `rm NAME` removes a node and revokes its keys, and `leave` takes this node's `authorized_keys` block out.

### Setting up a host

A machine that members connect to needs an SSH server that accepts public keys only. `scripts/install-mesh-sshd.sh` (root-owned drop-in, timestamped backup, `sshd -t` before reload; see [system-config/README.md](../system-config/README.md#ssh-for-the-mesh)) turns off passwords and root login and can restrict `ListenAddress` or move `sshd` to another port (`--port`, for a machine where Tailscale already answers port 22 on its tailnet address), and `scripts/check-mesh-sshd.sh HOST [PORT]` run from another machine confirms that passwords are refused. A machine with a default-deny `firewalld` zone also needs the `sshd` port opened on the interface the route uses. Root on a peer is never an SSH login: a member opens a session as the ordinary user and runs `sudo`, which asks for the user's password. A machine whose owner has a security key can also make `sudo` ask for a touch with `scripts/enable-sudo-yubikey.sh`; no node requires it. The node's 1Password needs to be signed in on that machine with the SSH agent turned on (Settings > Developer > Use the SSH agent) and the approval duration chosen there; `dojo-mesh doctor` says "the SSH agent has no keys: unlock 1Password" until it is.

### Moving a Tailscale mesh to a roster

Nothing has to stop working while this is done: a node uses the roster only when its `mesh.toml` says `method = "roster"`, and until then it behaves exactly as before. Do it one node at a time, with Tailscale still carrying the node you are not working on.

1. On every node, install the current `dojo-mesh` and `dojo-input-capture` (`scripts/install-helpers.sh`). On a machine that members will connect to, run `scripts/install-mesh-sshd.sh` ([Setting up a host](#setting-up-a-host)).
2. On the operator's node, add `method = "roster"`, `node-name` and `signing-keys` to `mesh.toml`, run `dojo-mesh init`, then `dojo-mesh trust add YOUR_SIGNING_KEY.pub --fingerprint SHA256:…` for each signing key, and `dojo-mesh join --route HOST[:PORT]` followed by `dojo-mesh approve NAME.join.json --fingerprint SHA256:…` for itself. Choose the route each peer will be reached at: an address the others can reach, and a port that is the node's own `sshd` (not Tailscale's, which answers port 22 on a tailnet address).
3. On each other node: the same `method = "roster"` and `node-name`, `dojo-mesh init`, `dojo-mesh join`, hand the request to the operator, `dojo-mesh trust add` the operator's signing key after comparing fingerprints, `dojo-mesh roster sync` (or copy the roster directory), and `dojo-mesh doctor`.
4. Check with `dojo-mesh status` and `dojo-mesh exec PEER -- true` before relying on it. To go back to Tailscale at any moment, remove `method = "roster"` and run `dojo-mesh leave`; the `membership-tag` and `allow` lists were never touched.
5. Remote input between roster nodes needs the Niri built with the OpenSSH gate (`scripts/install-niri-fork.sh`, effective at the next login) on the **receiving** node, and `accept-input` naming the sender as before.

Mixed meshes are not supported: a node is on one method, and a Tailscale node and a roster node do not see each other.

## Reaching peers

`exec`, `cp` and `task` go through the node's channel: `tailscale ssh` for a Tailscale mesh, the system `ssh` for a roster mesh. The remote command gets `DOJO_FROM`, `DOJO_SESSION` and `DOJO_ORIGIN` in its environment. Arguments are quoted, never interpreted by a shell. `cp` sends one regular file, keeps its permission bits, and replaces the destination atomically; copying from a peer writes through a temporary file, so a failed transfer leaves nothing behind.

Every call is appended to `$XDG_STATE_HOME/dojo/mesh/audit.jsonl` before it starts and again when it ends. The agent's calls are refused if they cannot be logged. `dojo-mesh pause` stops the agent's calls (those marked `DOJO_MESH_ORIGIN=agent`) until `resume`; it is a brake, not a security boundary, because anything running as the user can still run `tailscale ssh` (or, with a roster, `ssh`, if it can reach the agent that holds the key) itself.

## Capabilities

`find` asks each online peer to `describe` itself (by running `dojo-mesh describe` there), so a peer needs `dojo-mesh` installed in `~/.local/share/dojo/bin`. A peer that cannot be asked is reported, not guessed. The Nix cache is the first consumer: see [nix-cache.md](nix-cache.md).

## Sending a window

`dojo-mesh send` opens what a window shows on a peer and then closes it here. The panel (`dojo-desktop --send-window`, bound to Super+F11) does the same: it lists your windows and the machines that are online, and shows exactly what the selected window would send before you send it. `--dry-run` prints that line without calling anyone.

What moves depends on the application. A **Helium** window sends the page in front by default, and **only that tab closes** here: the window is checked to still show that page, then sent Ctrl+W through an input seat of its own (Niri's agent input, which needs `niri msg yield-human-input` first because you have just used that window; only your own calls do that, never the agent's). The window and its other tabs stay, and your focus doesn't move. **kitty** sends its directory, and nvim's file and line, or yazi's directory, when one of those is in front; the window closes whole, the shell starts fresh and anything else that was running is named as staying behind. Any other application is relaunched from its desktop entry. kitty is read through its remote control, which is limited to `ls` with a per-machine password in `~/.config/kitty/dojo-remote-control.conf`; see the spec in `.scratch/mesh-boundaries/spec.md`.

**Whole browser windows.** With the *Dojo Tabs* extension connected, `--scope window` (**W** in the panel) sends every tab of the window, opened together in a new window of the peer's browser, in order. The extension (`extensions/dojo-tabs`, loaded by the `helium` wrapper next to Browser Control's) is read-only: it reports each window's tabs (address, title, which is in front, pinned) to a native-messaging host, `dojo-tabs-host`, which keeps the latest report in `$XDG_RUNTIME_DIR/dojo/browser-tabs.json` (mode 0600, in a 0700 directory) and deletes it when the browser lets go, so a file that exists is a live report. With the report, the front tab's address is exact rather than found in history, and the window is matched to the Niri window by the page its front tab shows (two windows showing the same page are not guessed between). The extension needs the `tabs` permission, so it can see every tab's address and title; nothing leaves the machine except what a send carries, and it cannot open, move, close or read a tab. A running Helium must exit normally and relaunch once to load it.

The original window closes after a whole-window send only if it still holds exactly the tabs that were sent: the list is re-read just before closing, and a tab opened meanwhile keeps the window open. Tabs that cannot be sent (browser pages such as settings) are left behind and keep the original window open too, as does anything the browser cannot confirm. Pinned state and tab groups do not carry over. At most 100 tabs go in one window.

The peer only opens a web page, a directory, a file at a line, or an application it already has; it never runs a command it is sent, and it refuses while its screen is locked. It reports success only once a new window appears (a page opens as a tab in a running browser, which makes no new window). If the peer lacks a file, nothing is sent; `--copy-missing` copies the file across first. The panel sends with `--keep-source`, shows what was sent, then closes the original after a few seconds unless you press Esc, and lets go of the keyboard before closing it.

## Taking the cursor and keyboard

`dojo-mesh remote PEER` (or the panel: **I** switches to "only my cursor and keyboard", no window sent, and Enter starts it; the "Take my cursor and keyboard" switch, Space, starts it after a window send instead) hands this node's pointer and keyboard to a peer until you press **Super+F11** again, or **Ctrl+Alt+Shift+Esc**, which is always there for when something goes wrong. Neither chord reaches the peer.

On this node `dojo-input-capture` covers the focused output with a transparent surface that has the keyboard exclusively, locks the pointer (so motion keeps coming at the screen's edge) and hides the cursor. It also asks Niri to inhibit its own shortcuts for the surface, so Super+Shift+Space and the rest go to the peer instead of firing here (Niri's `allow-inhibiting=false` binds, if you add any, still fire). Nothing reaches local windows meanwhile. It writes one JSON event per line (relative motion, buttons, scrolling, evdev keys), which `dojo-mesh remote` carries over the node's channel (with a roster, using the separate input key) to `dojo-mesh inject` on the peer. Whatever ends the session returns your input at once: the chord, the connection dropping, the peer refusing, a lock screen taking the surface, or the helper exiting. Held keys and buttons are released on both sides.

The peer delivers the events to Niri as its own keyboard and pointer (`Request::RemoteInput`, [ADR 0008](adr/0008-remote-input-takes-the-real-seat-only-inside-a-peers-ssh-session.md)): the peer's key binds fire (the launcher, window moves, anything in its `config.kdl`), there is one real cursor that crosses all of its monitors, and keys go to whatever has focus there, including a launcher that just opened. Niri accepts this only from a process inside an SSH session from another machine (a Tailscale SSH session, or an OpenSSH session of the root `sshd` whose connection comes from an address that is not this machine's), only while the screen is unlocked, and lets go of everything held when the connection closes. The person at the peer can end it at any time with **Ctrl+Alt+Shift+Esc** on their own keyboard. A Niri without `RemoteInput` still works through an agent input seat (a second cursor, no peer shortcuts) and says so once; installing the new Niri (`scripts/install-niri-fork.sh`, effective at the next login) on the **peer** is what turns the real input on.

The peer accepts this only from nodes it names in `accept-input`, and learns who is sending from the method's answer about the connection (Tailscale's `whois` of the SSH session, or under the roster method the name the input key's forced command gives, checked against the signed roster), never from what the sender says. Before it takes the keyboard, `dojo-mesh remote` connects once and asks the peer (`inject --check`), so a key approval from 1Password or a refusal shows up as text while the keyboard is still yours. It refuses while its screen is locked, refuses the agent, and refuses anything not started over a peer's connection. It asks Niri to end its pause on windows its person has used (`yield-human-input`), as the operator's own call. The agent has no tool for any of this.

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
