# Upstream sync, October 2026

Follow-up work from the 2026-10-18 `dojo-update` run and the review of the upstream projects Dojo
vendors or learns from. The review compared each recorded revision with upstream's head
(2026-10-17/18) and kept only changes that matter to how Dojo uses the project.

## Already done

- Refreshed devenv, nixpkgs, home-manager, bend, opencode and pi pins (`5d457e9ba9`).
- GPUI: five upstream picks on `vendor/zed` (`ac5f05b835`): uniform_list layout, source-over alpha,
  wgpu 30 teardown fix, focus cleanup, touch fling. Recorded in `provenance/README.md`.
- Niri: 20 upstream picks on `vendor/niri` (`959750a899`), including grab cancel on lock and the
  touchpad toggle IPC. The virtual-keyboard rework (`7da6ae44`, Smithay `fc9e8c42`) is held back.
- cua: bump to `afed1fcb` prepared on branch `cua-bump` (Rift workspace); see issue 04 and the
  ledger notes there once merged.

## Reviewed and needing nothing

Omarchy, Aether, omarchy-themes and aether.nvim palettes, theme layout and CLI contract are
unchanged for what Dojo uses. browser-control is at its recorded revision. stasis, vigiland,
clipvault, utsushot, rift, walker have no new commits. Yazi's breaking changes (search action,
`ya.time`, `[tasks].bizarre_retry`) touch nothing Dojo configures.

## Open work

See `issues/`. Order of value: 02 rollout, 01 Niri virtual input, 04 cua driver follow-ups,
03 AT-SPI wake, then the smaller ones.
