# Zen termination: wluma system-bus reply accumulation

Diagnosed 2026-10-09. Theme palette application and Home Manager theme integrations
were suspended during investigation; the implementation remains in the repository.

## Evidence

- Zen exited with SIGTERM/143 in normal mode, troubleshooting mode, and fresh
  headless profiles. Sites appeared intermittent; disabling VA-API initially made
  the headless test pass but did not stabilize the user's browser.
- `strace -k` identified GLib's GDBus connection-closed callback raising SIGTERM.
  System-bus logs reported clients disconnected for insufficient reply resources.
- Privileged broker accounting showed UID 1000 had only 17,047 bytes remaining
  from its 536,870,912-byte budget. Registered-peer outgoing queues did not explain
  the usage; broker peer statistics skip unregistered peers.
- Unix socket inspection found wluma's system-bus FD 7 stalled, with nonempty
  queues in both directions. Stopping only `wluma.service` restored large system-bus
  replies. Zen then passed fresh-profile tests of about:blank, Discord, X, and
  ChatGPT with VA-API explicitly enabled.
- The first logged quota exhaustion was 21:10; the theme integration activation
  was 21:48. The apparent correlation with the theme rollout was not sufficient
  to attribute the termination to theme CSS or Qt configuration.

Full broker dumps/trace data are local diagnostic artifacts, not committed here.
Broker RSS may retain allocated heap after resources become usable; RSS alone is
not a reliable recovery check.

## Source-level defect and fix

The installed **wluma 4.11.1** brightness fallback sends `SetBrightness` requests
through a blocking D-Bus connection without setting the no-reply flag or draining
replies. The current upstream implementation duplicates the message, appends the
brightness, sets `message.set_no_reply(true)`, then sends it.

`system-config/patches/wluma-no-reply.patch` backports that small change, applied
through `services.wluma.package` in Home Manager. Do not fix this by raising broker
quotas, restarting the system bus underneath applications, disabling browser
sandboxing, or permanently disabling working video acceleration.

The incorrect temporary VA-API policy workaround was removed from the source.
Until the patched package is installed and checked, keep the old wluma service
stopped. Other compositor/browser services need not be restarted to recover bus
resource availability.

## Primary references

- [wluma 4.11.1 backlight implementation](https://github.com/max-baz/wluma/blob/4.11.1/src/brightness/backlight.rs)
- [upstream no-reply implementation](https://github.com/max-baz/wluma/blob/main/src/brightness/backlight.rs)
- [broker maintainer's resource-leak guide](https://dvdhrm.github.io/2021/04/14/locating-dbus-resource-leaks/)
- [broker v37 accounting implementation](https://github.com/bus1/dbus-broker/blob/v37/src/bus/driver.c)

## Launcher duplication (separate defect)

The launcher cleared its in-flight flag before removing the window on successful
launch. Queued input could then invoke the app a second time. The launch gate now
remains latched after success and resets only on failure; held Enter repeats are
ignored. Unit tests exercise both successful duplicate rejection and explicit
retry after failure. This is separate from the browser's system-bus termination.

The old GIO-based launcher also allowed launched processes to inherit Dojo's
process group/log pipes. Group-based shell restarts could therefore signal apps.
Dojo now parses desktop-entry Exec argv natively (never as a shell string), uses
UWSM services when UWSM owns the active session, and otherwise launches into a
separate systemd scope with its own process group and journald output. Explicit
D-Bus activation is not followed by a duplicate Exec fallback on timeout. Restart
the shell through its `--quit` control command, never by killing a shared group.
The live launch fixture checks argv, cwd, separate group/cgroup and survival after
the launch helper exits; no browser is opened by that test.
