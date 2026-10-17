# Greetings and farewells

A greeting is one short line shown briefly on the lock screen after a successful unlock and before the lock transition opens the desktop frame. It replaces the hint line under the dial. The ring completes, the greeting fades in, holds, fades out, and the frame opens.

## Rules

- **Name.** The first word of the account's full name (GECOS), else the login name with its first letter capitalised. Never print the raw login if a full name exists.
- **Timing.** Fade in 150 ms, hold 650 ms, fade out 150 ms, then the transition. About one second in total. Any key press or touch skips straight to the transition and never cancels the unlock.
- **Cost.** It plays inside the unlock transition, so it adds no frames afterwards and nothing runs while locked. A config switch turns it off, in which case the frame opens immediately.
- **Reduced motion.** The text cross-fades with no movement.
- **Selection.** A pure function of the unlock time, how long the session was locked, and the last greeting shown. It prefers the most specific matching entry, never repeats the previous line, and is deterministic for a given seed so tests can pin it. The last line shown is stored in the locker's state file.
- **Tone.** Short, warm, a little dojo. No exclamation marks, no emoji, no jokes that age badly.

## Pool

Templates use `{name}`. Each entry has a condition. Any condition-free entry is always eligible.

| Condition | Lines |
| --- | --- |
| Any | Welcome back, {name} · Onwards, {name} · Back to it, {name} · Step onto the mat, {name} · The dojo is yours, {name} · Hajime, {name} · Bow in, {name} · Ready when you are, {name} · Carry on, {name} |
| Morning (05:00–12:00) | Good morning, {name} · Early start, {name} · A fresh mat, {name} |
| Afternoon (12:00–17:00) | Good afternoon, {name} · Afternoon session, {name} |
| Evening (17:00–22:00) | Good evening, {name} · Evening practice, {name} |
| Night (22:00–05:00) | Still going, {name}? · Late session, {name} · Easy does it, {name} |
| Locked under 2 minutes | That was quick, {name} · Right where you left it, {name} · Back already, {name} |
| Locked 8 hours or more | A new day, {name} · Long time, {name} · Rested, {name}? |

Specificity order, most specific first: lock duration, then time of day, then Any. A time-of-day or duration match is chosen about two times in three, so the plain lines still appear often.

## Farewells

A farewell is the lock-side mirror. It appears while the desktop frame closes in, fades out, and leaves the resting hint line. The session is already locked from the first frame, so a farewell never delays locking, and typing during it skips straight to the password pill.

- **Timing.** It fades in as the dial forms, holds about 600 ms, and fades out, all inside the lock transition and the first moments after it. After that the lock screen is static as usual.
- **Name, tone, cost and reduced motion** follow the greeting rules above. A config switch turns it off, and the greeting and farewell switches are independent.
- **Selection.** Same pure function and the same never-repeat rule, with its own last-line state.
- **Reason (optional).** When the trigger passes a reason (`manual`, `idle`, `suspend`), the reason pool is eligible. An unknown reason uses the plain pool.

| Condition | Lines |
| --- | --- |
| Any | Securing the perimeter · Goodbye, {name} · Locking the dojo · The mat is rolled up · Until next time, {name} · Door's closed, {name} · Perimeter secure |
| Night (22:00–05:00) | Rest well, {name} · Sleep well, {name} · Lights out, {name} |
| Evening (17:00–22:00) | Good evening, {name} · Enjoy the evening, {name} |
| Reason: idle | Stepping away, {name}? · The dojo keeps watch |
| Reason: suspend | Zzzz · See you on waking |

## Other occasions

The same pure selection serves every occasion. Opening occasions (unlock, login) use the greeting pool. Closing occasions use these lines, and lock keeps the farewell pool above. A line may be Any, or limited by time of day as above.

| Occasion | Lines |
| --- | --- |
| Login | the greeting pool, plus after a fresh boot: Good to see you, {name} · Everything is ready, {name} |
| Logout | Until next time, {name} · See you soon, {name} · Bow out, {name} · Logging out |
| Suspend | Zzzz · See you on waking |
| Hibernate | Zzzz · Deep sleep, {name} · See you on waking |
| Shutdown | Powering down · Closing the dojo · Lights out, {name} · Goodnight, {name} (night only) |
| Restart | Back in a moment, {name} · Be right back · Taking a breath |
