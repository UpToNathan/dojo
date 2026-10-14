# Add the terminal updater and reproducible host baseline

Status: resolved
Type: task

Implement the [software updater spec](../spec.md). Keep the default version buffer
at three days with Pi, OpenCode, Codex, Claude, and T3 Code exempt. Capture package,
service, and privileged-file inventories. Reuse the independently maintained
firewalld installer during restoration.

## Result

The updater, age-aware helpers, manifest capture scripts, and restoration preview
are implemented. The default update plan captures the actual post-update baseline.
Unsupported source/hash updates and Neovim's unsupported age filter remain visible
as manual or held sources. The inventory reports two unreadable directories.
No ISO or clean-machine reproduction is claimed.

## Verification

All 47 behavior tests pass in devenv. Home Manager's activation derivation
evaluates. Main command and restoration previews run without installs. Live input
and Cargo helper dry runs resolve upstream metadata without source changes or
installs. Both captured baseline drift checks pass, including the firewalld
package, service, and configuration metadata.
