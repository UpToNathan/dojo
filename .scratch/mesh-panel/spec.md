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
- **Adding a machine** is the full enrolment in the panel. Join requests (`*.join.json`) in the mesh inbox (`$XDG_DATA_HOME/dojo/mesh/inbox`, and `~/Downloads`) appear as a count. Opening one shows the machine's name and its **request code**: four groups of four characters computed over the whole request (name, node, input and host keys, routes), which the new machine prints when it makes the request. The person compares the code with the other screen and presses Approve. A short fingerprint of the node key alone, as before, would let someone alter the host or input key in transit; the code covers them all.
- The trusted signing keys are a second view of the same panel (a key glyph): each as hardware or software, with Remove, and Add for a key file or a key shown on another machine, which asks for its fingerprint to be typed as it was read there.
- Nothing is computed in the panel that `dojo-mesh` does not already do. It reads a snapshot (`dojo_mesh::panel`) and runs the CLI for every change, so the terminal and the panel cannot disagree.

## Switch panel (Super+F11, bottom centre)

- Machines are tiles with an icon (laptop or desktop), the name and a status dot, chosen with Left and Right or the mouse. Windows are a short list with the application's name and the title muted on the same line.
- A two-way mode toggle of glyphs replaces the `[x]` checkbox: *window* (send the window, optionally taking control along) and *control only* (the keyboard glyph). The toggle, `I` and a click do the same.
- **Control only changes the panel.** The window list fades out and collapses, the panel shrinks to the tiles and a large keyboard-and-mouse glyph, the title and the confirm button change to "Take control", and the accent colour moves from the primary colour to a warmer one so the mode is visible from across the room.
- One short status line, only when something needs saying (a machine offline, a file missing there, sent). No key-hint line: the glyphs and the focus ring carry it, and `?` shows the keys on demand.
- Everything after Enter (sending, linger, closing the original) is unchanged.

## Out of scope

- Moving a join request from the new machine to this one. The inbox is a folder; files arrive by whatever the person already uses (a stick, a file share, `tailscale file cp`).
- Editing `mesh.toml` (membership, accept-input, capabilities).
- The Tailscale control plane (`enroll`, `policy`).

## Decisions

- Panel actions run `dojo-mesh` as a child process, as the switch panel does. Results come back as exit codes (3 means a person has to act: touch a key, unlock 1Password), shown as a glyph, not text.
- The panel works under both methods. Tailscale nodes have no signing keys or requests, so those parts do not appear; the member list and remote/send actions do.
