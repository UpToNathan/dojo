# Dojo

Dojo is a Niri desktop shell that frames the user's workspace and hosts shell surfaces.

## Language

**Desktop frame**:
The rounded surround that visually frames the usable desktop area while leaving the workspace visible inside.

**Reserved edge**:
An edge inset that keeps tiled windows within the desktop frame.

**Frame state**:
Whether the desktop frame and its reserved edges are enabled or disabled.

**Account**:
One credential source that can carry agent model requests, such as a Claude subscription or an OpenCode key. Accounts are discovered independently and any subset may be present.

**Model picker**:
The shell menu that lists the models an available account can serve and sets the agent's global model selection.

### Locking

**Occasion**:
The reason the desktop frame opens or closes: lock, unlock, login, logout, suspend, hibernate, restart or shutdown.
_Avoid_: Event, mode

**Session lock**:
The state in which the desktop is hidden and input is withheld from every application until the user proves their identity.
_Avoid_: Screen lock, lock state

**Lock screen**:
The full-screen surface Dojo shows on every output while the session is locked.
_Avoid_: Screensaver, greeter

**Login screen**:
The lock screen's form shown on a virtual terminal before any session exists. It is the same surface with a user chip and a session chip, drawn with the look of the user named in the user chip.
_Avoid_: Greeter UI, display manager screen

**Display manager**:
Dojo's own root service, `dojo-dm`, that owns the login virtual terminal, authenticates through PAM, opens the logind session and starts the chosen session.
_Avoid_: Greeter, ly

**Look snapshot**:
The copy of a user's palette, pre-blurred wallpaper, display name and greeting settings that the display manager keeps for the login screen. It is taken when a session starts and ends, so the login screen never reads a home directory.
_Avoid_: Theme cache, profile

**Unlock method**:
A way of proving identity at the lock screen: fingerprint or password. Both are live at the same time and the first to succeed unlocks.
_Avoid_: Credential, factor

**Dial**:
The clock inside a tick ring at the centre of the lock screen. It also shows the fingerprint reader's state.
_Avoid_: Ring, clock widget

**Session transition**:
The movement in which the desktop frame closes in over the workspace, or opens back out, when the session changes occasion.
_Avoid_: Lock animation, shutters, iris

**Greeting**:
The one short line shown as the desktop frame opens, on unlock or login.
_Avoid_: Welcome message, splash

**Farewell**:
The one short line shown as the desktop frame closes in, on lock, logout, suspend, hibernate, restart or shutdown.
_Avoid_: Goodbye message, splash

**Active output**:
The output whose lock screen currently takes keyboard input, which is the one under the pointer. Other outputs show only a dim clock.
_Avoid_: Focused monitor

**Password pause**:
A timed refusal of password attempts after repeated failures.
_Avoid_: Lockout

### Global shortcuts

**Global shortcut**:
A key combination an app asks for through the GlobalShortcuts portal, which acts whichever window has focus. The app learns when it goes down and up.
_Avoid_: Hotkey, keybinding (those are configured Niri binds)

**Shortcut grab**:
A global shortcut's key held by Niri on behalf of an IPC connection. Niri consumes the key, so the focused app never sees it, and releases it when the connection closes.
_Avoid_: Bind, registration

**Binding panel**:
The shell surface where the user picks the keys an app's global shortcuts get.
_Avoid_: Shortcut dialog

### Launcher

**Command**:
A built-in launcher entry that opens a view, such as Emoji, Clipboard History or Windows. Commands show as suggestions in the empty launcher and rank like any result when typed.

**View**:
A screen pushed inside the launcher with its own search box, list and action. It reuses the launcher's surface; Esc pops it.

**Inline row**:
A launcher result produced by an explicit trigger in the query, such as a calculation, a bang or a `?` agent prompt. It never appears for ordinary text.

### Mesh

**Node**:
One Dojo installation on one machine. A node is complete on its own and needs no peers.
_Avoid_: Host, machine, instance, server

**Mesh**:
The nodes that admit one another as peers. A node with no peers is a mesh of one.
_Avoid_: Cluster, fleet, tailnet

**Peer**:
Another node in the same mesh, as seen from one node.
_Avoid_: Remote, host

**Membership method**:
A way of finding candidate nodes, deciding which belong to the mesh and knowing which node is which. Tailscale tag membership is the first; a signed roster is the second.
_Avoid_: Discovery (only one part of it), provider

**Roster**:
The signed list of a mesh's members: each one's name, node key, host key and expiry, plus the operator signing keys and revocations. A node trusts only entries signed by a signing key it holds.
_Avoid_: Registry, directory

**Operator signing key**:
A key whose signature makes a roster entry count. A node trusts any number, held on a YubiKey, in 1Password or anywhere an SSH agent or key file can sign. It is used to enroll, renew, rotate and revoke, never for ordinary connections.
_Avoid_: Admin key, root key

**Node key**:
The key a node uses to prove itself to peers. Its custody is the node's setting; 1Password's SSH agent by default.
_Avoid_: Machine key, host key (that one identifies the SSH server)

**Route**:
How one node reaches a peer: an address and port, or a command that opens the connection.
_Avoid_: Address (it may be a command), link

**Membership tag**:
The Tailscale tag that marks a device as a candidate node. Each mesh chooses its own.

**Membership override**:
A node's own allow or deny entry for one device. It changes only what that node admits.

**Peer permission**:
What one node may do on a peer. It is whatever the network's policy grants the node, not something Dojo configures.
_Avoid_: ACL, role

**Capability**:
Something a node offers its peers, such as a Nix cache or a voice agent, as the node itself describes it.
_Avoid_: Role, service

**Peer task**:
Work handed to a peer's agent to carry out there, instead of operating the peer's desktop remotely.
_Avoid_: Remote control

**Operator**:
The person who owns a node. Changing the network, and lifting a pause on the agent, are theirs alone.
_Avoid_: Admin, user (when the agent is the other party)

**Audit log**:
The record each node keeps of every call it makes to a peer, with who made it and how it ended.
_Avoid_: History, trace

**Remote input**:
The operator's pointer and keyboard driving a peer after sending it a window, received there as the human's own input.
_Avoid_: Input sharing, KVM

**Handoff**:
Sending what a window shows to a peer: the peer opens the same thing and the local window closes. The process itself does not move.
_Avoid_: Migration, transfer

**Handoff adapter**:
The part of Dojo that knows, for one application, what its window is showing and how to open the same on a peer.
_Avoid_: Plugin, driver
