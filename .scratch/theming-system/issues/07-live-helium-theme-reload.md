# Reload Helium's unpacked theme on Apply

Status: resolved

Publish exact semantic colors and reload the installed Dojo theme without restarting Helium or navigating user tabs. Clear the legacy seed policy so it cannot override the extension. Require the browser's own one-time local debugging connection setup and report missing setup explicitly.

The isolated regression passes: `node scripts/check-chromium-theme-reload.mjs` loads a Dojo fixture, changes its colors, invokes the production hook, and verifies the existing tab updates while the browser process remains running. Evidence: `/tmp/dojo-theme-reload-check.BZfU9h`. The live user profile still needs remote debugging enabled before its connection can be verified.

## Comments

The user rejected the general debugging interface. This approach is superseded by `.scratch/browser-theme-reload/issues/01-helium-native-color-reload.md`. The CDP hook and regression script have been removed. The native patch was then explored in the replacement issue and abandoned because every browser release would require rebuilding.

The user subsequently rejected custom browser builds. The native patch was removed; stock Helium now receives the built-in browser-color policy. Exact multi-role colors are no longer required.
