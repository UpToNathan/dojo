# Lock screen grounding

Findings from two spikes and a read of the existing code, 2026-10-17. Artifacts are under `spikes/`. The spike scripts hardcode `/tmp/dojo-lock-pam-spike` as their root, so copy `spikes/pam` there before running them.

## Host facts

- No screen locker exists today. The system menu deliberately has no Lock command (`.scratch/shell-controls/design.md`).
- The vendored Niri fork implements `ext-session-lock-v1` (`LockState` in `vendor/niri/src/niri.rs`). While locked it allows only `ChangeVt`, `Suspend`, monitor power, `SwitchLayout` and `ToggleKeyboardShortcutsInhibit`. It calls `reset_all_agents()` on lock and sets the logind `LockedHint`.
- A dead locker leaves the session locked (red fallback) and a new locker can take over; a second locker while one is alive is refused.
- Keyboard focus goes to the lock surface on one output only (the one under the pointer, else the active output).
- `stasis` is enabled but `~/.config/stasis/stasis.rune` has no lock or idle action.
- Fingerprint hardware: one Synaptics press sensor with the right index finger enrolled; `fprintd` is D-Bus activated; `pam_fprintd.so` is installed.

## GPUI spike

Verdict: feasible with caveats. Patch `spikes/gpui-session-lock.patch` (core +246/-4 over six files, plus a 287-line example). Niri needed no change.

- Lock reached `locked`, window rendered, keys, text input, Backspace and Enter arrive, wrong password rejected, correct password unlocks.
- Dead keys compose under lock with `us(intl)` (`'`+`e` gives `é`). A real input method (`zwp_text_input_v3`) was not tested.
- Two outputs: one lock object, two lock surfaces, both rendered; keyboard focus went to the first only.
- Idle: no `wl_surface.commit` for about 32 s, then exactly one at the minute tick; 0 context switches and 0 CPU ticks over 30 s idle. GPUI parks its frame loop when nothing is dirty.
- Memory: release with only the radeon Vulkan driver, 89 MB RSS (38 MB PSS); with all drivers, 107 MB RSS (54 MB PSS); debug, 133 MB RSS (78–80 MB PSS). The GPU context stays resident after the first frame.
- Startup to `Locked`: debug 391–439 ms; release with all drivers 433–632 ms; release with the radeon driver pinned 106–144 ms. Nearly all of it is wgpu/Vulkan driver enumeration.
- Not measured: whether GPU runtime power management suspends with the device open (no power meter), real input method, hotplug.

Risks found: committing before the first `ack_configure` makes Niri disconnect the client (fixed in the patch by skipping the initial commit for lock windows); GPUI spun at 99% CPU after a dead connection, so the locker must exit explicitly; a lock object is orphaned if the first lock window fails; only one output gets keyboard focus.

## PAM spike

Verdict: feasible without root. `pam_start_confdir` (libpam 1.7.3) lets a non-root process run private service stacks. Harness `spikes/pam/bin/dojo_pam_race.py`, stand-in suite `run_scenarios.py` (8 of 8 pass), recommended service files in `spikes/pam/recommended/`.

- Password worker as non-root works through the setuid `unix_chkpwd`; confirmed with the real password on 2026-10-17 (`PAM_SUCCESS`).
- Fingerprint worker matched a real finger and won the race at 3.17 s; the password worker was killed in 0.059 s with no survivors.
- `pam_fprintd` sends "Place your right index finger on the fingerprint reader" as `PAM_TEXT_INFO` about 0.09 s after start when `fprintd` is warm and about 0.58 s when it was inactive. With `timeout=10 max-tries=1` and no touch it sends "Verification timed out" and returns `PAM_AUTHINFO_UNAVAIL`. No-match and retry-hint messages were not observed.
- The sensor is held while a worker runs and was claimable again right after the kill.
- `pam_unix` failures take 1.5–3 s inside `pam_authenticate`.
- `pam_exec` children call `setsid()` and escape a process-group kill, so a harness must kill the whole descendant tree. Real stacks have no long-lived children.
- `/run/faillock` is `root:root 755`; a non-root process cannot create a missing tally file but can append to one root has created.

## Design prototypes

The states and the shell-to-lock transition were prototyped as HTML during design and are not kept here. The geometry they use comes from `src/geometry.rs` (left panel 64, top bar 24, right and bottom insets 12) and the 28 px opening radius in `src/frame.rs`. Tray glyphs and the top-bar clock text in those prototypes were placeholders, so the real top bar must be copied exactly for the transition to match.
