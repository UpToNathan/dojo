# Restore stock Helium theme updates

Status: resolved

The user rejected rebuilding Helium for every release and accepted built-in Chrome theming. Remove the native patch and build tooling. Restore policy publication from the semantic background, preserve automatic appearance switching, verify the deployed Dojo integration, and keep the official browser installed.

## Comments

Removed the native patch, Helium build/test/install scripts, launcher override, rootless build image, and abandoned source/build cache. The normal browser still resolves to the official Helium 0.19.2.1 package. Dojo publishes BrowserThemeColor from the semantic background and requests Chromium’s existing --refresh-platform-policy command when Helium is running.

The policy-helper checks and five theme-apply tests pass. An isolated CLI check verifies the background seed and that helper failures are reported. The optimized shell is installed at ~/.local/share/dojo/desktop/26e8956eedf5998c-mmap128k and its service was restarted. The frame and four reserved edges are mapped on the currently active eDP-1 output; saved tiled-column widths were restored and a screenshot inspected. Current policy seed is #1a1b26. Evidence is in /tmp/dojo-stock-helium-verification; build/test logs are /tmp/dojo-stock-helium-build.log, /tmp/dojo-stock-helium-deploy.log and /tmp/dojo-stock-helium-final.log. Helium was closed during final verification, so the hook reports ready for next launch; a live browser repaint was not observed.
