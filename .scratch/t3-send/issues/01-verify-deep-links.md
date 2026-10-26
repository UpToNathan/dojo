# Verify that a thread deep link opens a thread, here and on the peer

Type: task
Status: open

Open `t3code://threads/<environment id>/<thread id>` with `env -u ELECTRON_RUN_AS_NODE xdg-open` on this machine for a thread other than the one on screen, and confirm the window navigates. Then do the same on the peer from its own session, for a thread that exists on this machine, and record whether it opens, what it shows if the environment is not linked there, and what the first launch does when T3 is not running. A first attempt on this machine produced no visible change; find out whether the handler, the URL form or the app is at fault before building on it.
