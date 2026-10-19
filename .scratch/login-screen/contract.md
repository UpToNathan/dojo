# Build contract

Shared by every workstream so they can run in parallel. Read the [spec](spec.md), [ADR 0003](../../docs/adr/0003-dojo-ships-its-own-display-manager.md) and the [glossary](../../GLOSSARY.md) first.

## Names

| Thing | Name |
| --- | --- |
| Display manager (root service) | `dojo-dm`, `src/bin/dojo-dm.rs` (wave 2) |
| Root PAM helper | `dojo-dm-pam`, `src/bin/dojo-dm-pam.rs` |
| Greeter (the lock program in login mode) | `dojo-greeter`, `src/bin/dojo-greeter.rs` (wave 2) |
| Greeter user, group | `dojo-greeter` |
| PAM services | `dojo-login` (password, the old `ly` stack), `dojo-login-fingerprint`, `dojo-greeter` (the greeter's own logind session, class greeter) |
| Greeter socket | `/run/dojo-dm/greeter.sock`, SOCK_SEQPACKET, peer uid must be `dojo-greeter` |
| Look snapshots | `/var/lib/dojo-dm/users/<login>/`, written by `dojo-dm`, readable by group `dojo-greeter` |
| Installed release | `/usr/local/lib/dojo-dm/<release>/` plus a `current` symlink, root-owned, with `environment.sh` |
| Greeter Niri config | `system-config/dojo-dm/greeter.kdl`, installed beside the release |
| Rust modules | `dojo::dm` (`src/dm/`), `dojo::session::snapshot` |

## Shape

The login screen reuses `lock::LockModel`. The display manager relays each PAM conversation to the greeter as the same `lock::pam::Report` stream the locker's workers produce, so the model keeps `Effect::Spawn/Kill/Answer` and `EventKind::Worker`. In login mode the edge answers `ProbeShellOverlay` with `true` (the shell draws the login opening), and `Effect::Unlock` becomes "start the session". Passwords travel in the same tagged datagram style as `lock/worker/wire.rs`, never JSON, and are zeroized after use.

The root helper `dojo-dm-pam` is one process per login attempt. It authenticates, then waits. On the greeter's start request it runs `pam_acct_mgmt`, `pam_setcred`, `pam_open_session`, forks the session as the user, waits for it, then closes the PAM session. The user name comes over the socket, is validated with `getpwnam` (uid in the human range, valid shell), and is never on argv.

## Rules for every workstream

- Work in your own Rift workspace: `rift create --name <task>` from `/home/nathan/Dojo`, `cd ~/.rifts/Dojo/<task>`, `git switch -c <branch>`. No git worktrees. Leave the main checkout alone.
- Run Cargo through `devenv shell --`. Do not set `CARGO_TARGET_DIR`; leave no build output in `/tmp`; check `du -sh` on anything you created.
- Never run real power actions, never lock the live session, never touch `/etc` or install anything on the host. Everything runs against fakes, temporary directories and the nested compositor.
- Build first, verify lightly: `cargo check`, unit tests for pure logic and parsing, a release build only when asked. Say plainly what you did not verify.
- Use the glossary's terms. Comments only for a non-obvious reason. Config parsing and external input validation stay at the boundary; domain logic stays pure and free of GPUI, Niri and host services.
- Commit on your branch in small ordered steps. End each commit message with `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`.
- Report: files changed, what the tests cover, what is unverified, and anything the next workstream must know.
