# Mesh panel and a cleaner switch panel

Two things, one theme: the mesh gets a place in the shell, and moving between machines stops reading like a manual.

## What exists today

- `dojo-mesh` does everything (status, doctor, remote, send, enrol, approve, renew, rm, trust) from a terminal. The shell shows the mesh as a launcher command whose Enter copies a command.
- The send-window panel (Super+F11, bottom centre, `src/send_window_ui.rs`) lists windows and machines, a `[x]` checkbox for taking the cursor and keyboard, status lines and a long key-hint line. `I` switches to "only my cursor and keyboard" and changes almost nothing on screen.
- The monitor panel (`src/displays_ui.rs`) is the pattern for a top-right panel: a layer-shell overlay, `shell_extension` drawn as a growth from the frame, a 150 ms reveal, icon buttons from the bundled Nerd Font.

## Mesh panel (top right)

Opens from a button in the top bar and `dojo-desktop --mesh`, like the monitor panel. Sketch, at about 380 px wide:

```
Mesh                                     ● ✓
─────────────────────────────────────────────
● venus                             this machine
● jupiter          87 days         ⌨  ⧉  ↻  ⋯
○ studio           offline         ⌨  ⧉  ↻  ⋯
─────────────────────────────────────────────
⌁ 1 request                              + Add
```

- One card per machine: a status dot (online, offline, expiring soon, problem), the name, and one muted word or number (days until its entry expires, "offline"). No sentences.
- Icon buttons per peer, enabled only when they can work: take cursor and keyboard, send a window (opens the switch panel on that machine), renew, and a menu with remove. Remove asks once, inline, with the machine's name; renew and remove sign with the node's signing keys, so a hardware key may ask for a touch: the card shows the key glyph pulsing, not a message.
- The health badge at the top right is `dojo-mesh doctor` as a glyph: a check when healthy, an alert with a count otherwise. Pressing it lists the findings, one line each, which is the only place sentences appear. A locked 1Password shows as a lock glyph on this machine's card.
- **Adding a machine** needs no files and no typing. *Add machine* makes an **invite** and the new machine finds it by itself. The invite and everything that follows travel through the person's own 1Password (an item in a vault both machines can read): the operator's panel posts the invite (its name and the signing keys it trusts); the new machine, when it joins, finds the open invite and posts its signed request into the same item; the panel shows "studio wants to join" with Approve and Deny; approval signs the entry and posts the answer (the roster and the trusted signing keys), which the new machine installs. The item is deleted when the exchange ends or the invite expires (default 15 minutes). The vault is the trust anchor, as it already is for the nodes' keys, so no fingerprint is compared by eye and no `trust add` is typed on the new machine. A request is still signed by the new machine's own key, so its fields cannot be altered. The carrier is an interface (`Rendezvous`): 1Password first, a shared directory for tests and for people without 1Password, others later (Cloudflare).
- **Keys** are shown by purpose, not by fingerprint: the keys this node *signs with*, in order (the first is the primary, the rest are fallbacks), each as *Security key*, *1Password key* or *Key file*; and the keys it *trusts*. Order, removal and adding are done in the panel and are written to the configuration by `dojo-mesh keys …`, never by editing a file. Adding a security key needs its PIN, asked by a small Dojo dialog.
- **Membership** lasts 90 days and renews itself: a machine that can sign without a touch renews entries when they are within 30 days of expiring. The panel shows nothing until an entry is within 14 days of expiring, then an amber note and a highlighted Renew.
- Nothing is computed in the panel that `dojo-mesh` does not already do. It reads a snapshot (`dojo_mesh::panel`) and runs the CLI for every change, so the terminal and the panel cannot disagree.

## Switch panel (Super+F11, bottom centre)

- Machines are tiles with an icon (laptop or desktop), the name and a status dot, chosen with Left and Right or the mouse. Windows are a short list with the application's name and the title muted on the same line.
- A two-way mode toggle of glyphs replaces the `[x]` checkbox: *window* (send the window, optionally taking control along) and *control only* (the keyboard glyph). The toggle, `I` and a click do the same.
- **Control only changes the panel.** The window list fades out and collapses, the panel shrinks to the tiles and a large keyboard-and-mouse glyph, the title and the confirm button change to "Take control", and the accent colour moves from the primary colour to a warmer one so the mode is visible from across the room.
- One short status line, only when something needs saying (a machine offline, a file missing there, sent). No key-hint line: the glyphs and the focus ring carry it, and `?` shows the keys on demand.
- Everything after Enter (sending, linger, closing the original) is unchanged.

## Out of scope

- A rendezvous other than 1Password and a shared directory (Cloudflare is the likely next one).
- Editing `mesh.toml` (membership, accept-input, capabilities).
- The Tailscale control plane (`enroll`, `policy`).

## Decisions

- Panel actions run `dojo-mesh` as a child process, as the switch panel does. Results come back as exit codes (3 means a person has to act: touch a key, unlock 1Password), shown as a glyph, not text.
- The panel works under both methods. Tailscale nodes have no signing keys or requests, so those parts do not appear; the member list and remote/send actions do.
