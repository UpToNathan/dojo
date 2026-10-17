# Keep the lock screen from draining battery

Status: needs-info
Blocked by: 02, 03, 05

Implement and verify the power rules in [the spec](../spec.md): the lock screen settles into a static dim frame, the display turns off after a short timeout and wakes on input, the fingerprint sensor is armed only while the display is on, and the locker starts on demand with its Vulkan driver pinned. This follows the project constraint that the lock screen is near-zero cost while locked.

Needs the owner's answers to the open decisions in the spec before it is finished: the display-off timeout (recommend 30 s) and whether a touch on the sensor with the display off should wake only or also unlock.

## Acceptance

- Measured on the real machine, not assumed: frames and wakeups per second for a static locked screen are zero apart from the minute tick, and GPU busy stays at idle.
- Battery `power_now` is recorded for unlocked idle, locked and lit, locked and static, and locked with the display off, with the method written down so it can be repeated.
- Whether GPU runtime power management suspends while the locker's device is open is answered. This was unmeasured in the spike.
- Waking the display and the first keypress both work, and the first keypress is not lost.
- The locker does not keep the sensor claimed while the display is off.

## Progress

Built and measured: zero frames when static apart from the minute tick, zero wakeups after pinning `blocking` to 1.6.2, display off after 30 s, sensor disarmed with the display off, startup about 155 ms with the Vulkan driver pinned. Not measured: battery `power_now`, GPU runtime power management, the waking key on a real display.

## Comments
