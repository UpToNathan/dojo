# Supply runtime libraries to the login service

Type: bug
Status: resolved

## Reproduction

After login, `app-dojo@autostart.service` failed with exit status 101. The journal
reported `failed to connect to Wayland compositor: The wayland library could
not be loaded`. Its generated ExecStart invoked the debug binary directly.
Manual startup inside devenv succeeded and mapped the desktop frame.

`devenv shell -- scripts/check-dojo-autostart.sh` returned exit 1 against the
failed service, even while the manually launched shell was running.

## Fix and verification

Home Manager now generates a wrapper that exports `config.env.LD_LIBRARY_PATH`
from the devenv definition and executes the built Dojo binary. The autostart
desktop entry uses that wrapper. The Nix closure retains its library paths.

Built and applied Home Manager plan `plan-oR4iYNoUjppDeaugfbdwNwTw`. Stopped the
manual shell through its quit command, then started the generated autostart
service. The same check passed with one Dojo process, service exit status 0
and a mapped frame. The current compositor remained running throughout.

Logs are `/tmp/dojo-autostart-plan.log` and `/tmp/dojo-autostart-apply.log`.
Session evidence is `/tmp/opencode/dojo-verification/login-130800`.
The corrected startup command has been exercised in the current session;
a second full logout/login has not been performed.
