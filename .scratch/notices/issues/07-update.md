# Notify when a software update run finishes

Status: resolved
Type: task
Blocked by: 01

`scripts/dojo-update` calls `dojo-notify` from the end of `main()`: finished, or finished with failed or blocked steps, naming them. A missing `dojo-notify` is ignored.

## Result

`scripts/dojo-update` now ends a real run (not `--dry-run`) by calling `send_notice(runner.results)`. `update_notice()` is the pure function that builds the notice: all fine gives `--kind update --urgency low --icon software-update-available-symbolic "Software updated"` with a body counting finished steps; any failed or blocked step gives urgency normal, "Software update finished with problems", and a body naming them (for example `nix-inputs failed, nix-lock blocked`). `dojo-notify` is found on PATH or at `~/.local/bin/dojo-notify`; a missing or failing one is ignored and never changes stdout or the exit code.

Verified: `python3 scripts/test-dojo-update.py` (16 tests, 4 new: notice for clean and problem runs, one notice per real run and none for dry-run, missing/failing dojo-notify ignored). The existing `run_cli` helper now patches `send_notice` so the tests never notify for real. (`python3 -I` cannot be used here: the script imports `arch_packages` from its own directory.)
