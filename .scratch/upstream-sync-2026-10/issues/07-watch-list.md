# 07 — Watch list: changes to act on when they release

Type: task
Status: needs-triage
Blocked by: none
Effort: small, recurring

## Brief

- **wluma 5.x** (`478ae36` is the same fix as `wluma-no-reply.patch`): drop the patch and the "do not
  enable until patched" note when nixpkgs moves past 4.11.1; `[als.none]`, `[[output.backlight]]` and
  `[[keyboard]]` stay valid.
- **nucleo-matcher**: stay on =0.3.1. Git master fixes case-sensitive prefix matching (`dc30540`) and
  Latin normalization (`2200109`), which `AtomKind::Prefix`/`Exact` in `src/application_catalogue.rs`
  rely on. Add a test for a capitalised prefix query now, adopt on release.
- **yazi**: installed 26.9.1; master removes the `search` action, moves `search_*` options to
  `[rg]`/`[fd]`, changes `ya.time()` to return `Time`. Re-run the theme and chooser tests at the next
  release.
- **pi-durable** (tools/speech pins 1.3.0): the next version migrates `pi.sqlite` to schema v2 with no
  rollback and removes `TaskRuntime.sleep()`. Before bumping: back up `pi.sqlite`, grep for `sleep(`,
  add a test for the crash-loop park.
- **browser-control**: branches `fix/execute-hardening`, `fix/relay-drain-deadline`,
  `fix/tab-capture-hint` touch files `dojo.patch` patches; expect rework when they merge.
- **t3code**: installed bundle version unknown; confirm with `t3 --version` before the next update.
  `dba8297da` rejects worktree delegation from a checkout outside the project (Rift workspaces).

## Acceptance

Revisit at the next `dojo-update`; each item closed when acted on.
