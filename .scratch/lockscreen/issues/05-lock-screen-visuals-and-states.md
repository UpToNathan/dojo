# Lock screen visuals and states

Status: resolved
Blocked by: 02

Build the experience in [the spec](../spec.md): the dial with its tick ring, date, hint line, top and bottom chips, and the states resting, fingerprint reading, typing, wrong and password pause. Read the theme from `$XDG_STATE_HOME/dojo/themes/current/palette.json` once at start and use the shell's tokens in `src/design.rs`. Compute the blurred wallpaper when the theme is applied (`dojo-theme`), and have the locker load that image; the locker never blurs live.

Show a dim clock on outputs that are not the active output. Honor reduced motion by replacing movement with colour.

## Acceptance

- Each state renders correctly in the nested Niri, with a screenshot checked for each.
- The lock view never animates continuously. Between events and minute ticks it commits no frames.
- A wrong password shakes the pill and flashes the ring; under reduced motion it only changes colour.
- The wallpaper image is produced at theme apply and used by the locker; a missing image falls back to a flat palette colour.
- Two outputs show the same dots, and only the active output shows the hint and pill.
- Nothing from the session (notifications, window titles, media) appears.

## Greeting and farewell

Add a pure `greeting` module for both the greeting and the farewell: the pools and selection rules in [greetings.md](../greetings.md), with unit tests that pin selection for a fixed seed, never repeat the previous line, and fall back to a plain line when no condition matches. The name rule reads the account's full name once at start.

- A greeting is chosen and shown for each of: morning, afternoon, evening, night, a short lock and a long lock, checked by unit test.
- A farewell is chosen for each of: any, night, evening, and each lock reason, checked by unit test, and never repeats the previous farewell.
- The last line shown persists across lockers and is not repeated.

## Progress

Built: the dial, all states, chips, the pre-blurred and contrast-aware wallpaper, greetings and farewells with per-occasion pools. Contrast on a near-white wallpaper measured 4.5:1 (dark palette) and 7.1:1 (light). Not measured: near-black, busy mid-grey, the user's other wallpapers.

## Comments
