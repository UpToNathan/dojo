# Browser theme updates

Dojo updates stock Helium through its built-in managed BrowserThemeColor policy, using the semantic background as the seed. The browser uses its normal desktop appearance handling. Chrome derives the remaining colors; exact agreement with T3 is intentionally sacrificed for compatibility with official browser updates. No patched binary, remote debugging or browser restart is required for routine theme changes.

Zen retains its existing browser.theme.update/native-messaging integration.

The custom Helium build was abandoned at the user’s request. Remove its source patch, build/install scripts and unfinished build workspace. Keep the installed official Helium.
