# Shell-nested quick-access Kitty

Ctrl+F11 opens a persistent Kitty session inside an extension of the desktop frame on the focused output. Use the existing 150ms shell reveal with joined shoulders and solid theme background. Inset Kitty's rectangle within the extension so its edges never replace the rounded shell outline. Dojo's background accepts no input; Kitty retains its own keyboard/terminal behavior. Toggling hides the terminal, preserves its session, and retracts the outline. Exiting the shell retracts the outline too. Disabling Dojo hides the terminal. Normal Kitty windows remain unchanged.

Use Kitty's existing quick-access instance grouping and panel geometry. Query Niri only at activation to select the output, without idle polling. An EXIT/HUP handler reports terminal shutdown through Dojo's existing control socket. Keep process arguments out of shell interpolation.
