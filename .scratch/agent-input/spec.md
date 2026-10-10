# Independent agent input

An agent can move its own visible cursor, click a mapped application, and type into an independently focused window while the human continues using another window. The agent uses a real Wayland seat. It must never replace the human seat or change the human keyboard focus through Niri's shared layout activation.

## First implementation

Niri creates an agent seat for a persistent IPC controller connection. Typed requests create the seat, select a mapped window, move its pointer, inject buttons, scrolling and evdev keys, and reset or destroy it. Connection loss releases held input and removes the seat and cursor. Agent input is opt-in during development.

Locking suspends agent input, releases held keys/buttons, clears focus and hides its cursor. Unmapped targets lose agent focus. Agent requests cannot type into lock surfaces or invoke human compositor shortcuts. Unsupported shell move/resize requests must not use the human seat's state.

GPUI binds every announced seat and maintains device, focus and input state per seat. A newly announced agent seat cannot release or replace the human seat. Window activation reflects whether any keyboard seat focuses the window. Different GPUI windows can receive concurrent human and agent input. Shared widget documents and carets remain application state.

## Architecture decision

Use the existing Niri IPC transport for a first controller, with connection ownership and direct Smithay input dispatch. This avoids Smithay's virtual-keyboard path, which bypasses compositor grabs. Keywork remains a reference for seat independence. Standard transient-seat interoperability can follow once the input model is working.

Pinned dependencies live in Git subtrees. Niri is at d0cd58ec5ac6b2d41d42478fb4f3cc7c0c687db9. GPUI is at Zed 9ab0715969e9854f2efe61a9d782e7698e5fe6d2. Dojo imports GPUI through local paths.

## Verification

Build both forks through devenv. Use a nested compositor and a client with two windows to demonstrate independent focus, cursor motion, click/key delivery, controller disconnect cleanup and lock rejection. Exercise GPUI seat addition/removal without disrupting human input. Do not replace the running desktop before the nested implementation works.
