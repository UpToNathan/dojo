# 1Password on Linux: startup, resume, and tray recovery

## Findings

- **Use the desktop app's own startup preference / desktop-session autostart for normal launch.** 1Password's Linux support page documents supported installation methods (including its packages) but does not prescribe a custom systemd service or `Restart=always` workaround. Keep the application in the logged-in graphical session: a system service is the wrong context for a desktop app that needs the user's graphical session and tray.
- **A locked app is not an exited app.** 1Password's unlock documentation describes locking/unlocking as application security state. If the process remains alive, an unlock prompt or unavailable vault is not evidence that the app needs restarting.
- **A missing tray icon does not prove the process exited.** The tray is a desktop-shell/status-area presentation surface. Check the process and app window/Quick Access independently. If the process is alive but the icon is absent, investigate the desktop environment's tray/status-notifier support or 1Password's tray integration; restarting the process may not address the shell-side cause. 1Password's Quick Access documentation confirms the Linux app uses a system-tray icon, but does not document a universal tray-recovery procedure.
- **Suspend/resume does not normally mean the app should be restarted.** systemd-logind publishes `PrepareForSleep(true)` before sleep and `PrepareForSleep(false)` after resume. A process that needs to respond can subscribe to the documented D-Bus signal; systemd does not infer that it should restart ordinary services on resume.
- **Lock/unlock is a separate session event.** logind's session `Lock()`/`Unlock()` signals are requests for the session manager to lock or unlock the screen; the session manager is expected to listen and act. They are not equivalent to app process exit, and systemd does not automatically restart user services on either event. An app/service can explicitly subscribe to the D-Bus/session protocol if it needs those events.

## Is `Restart=always` appropriate?

Generally, **no—not as the standard way to manage the 1Password desktop app**. `Restart=always` restarts a service whenever its main process exits, including a clean exit (except explicit stop and configured exceptions). That can mask or fight intentional quits, duplicate a desktop-session launch, or repeatedly relaunch an app that is failing. It also cannot restore a tray icon if the application process is still running. If there is a demonstrated crash/restart need, a user-level service with a narrowly chosen restart policy can be considered, but that is an operator workaround, not a 1Password-recommended Linux setup.

User services **can remain running across suspend** and can receive resume/lock events only if they subscribe to the relevant D-Bus/session signals (or are explicitly wired to suitable targets/hooks by the desktop/system). A service unit's `Restart=` policy reacts to process termination, not to suspend, resume, lock, or unlock signals. User-manager lifetime is also distinct from a graphical session's lifetime; desktop environments differ in how they integrate `graphical-session.target`.

### Crash recovery in Dojo

Dojo's autostart drop-in (`home-manager.nix`) sets `Restart=on-failure` with a 3 s delay and at most five starts per ten minutes. On 2026-10-15 niri closed 1Password's Wayland connection and Electron aborted with SIGTRAP (`code=dumped`), which is a failure exit that this policy restarts. A clean Quit exits 0 and a session stop is not a failure, so neither is restarted. The wrapper waits for the StatusNotifierWatcher before launching, so the new instance registers with Dojo's tray.

## Practical triage

1. After resume, determine whether the 1Password process is present. If it is, try opening the app/Quick Access and distinguish a locked vault from a dead process.
2. If the process is absent, launch 1Password normally from the app menu. If this recurs, inspect the app's own logs/journal and update to a current supported version rather than adding an unconditional restart loop first.
3. If the process is present but the tray item is missing, diagnose the desktop environment's tray/status notifier separately; process supervision alone cannot guarantee icon presentation.
4. Use 1Password's documented unlock and auto-lock settings for unlock behavior. Do not use an autostart/restart mechanism as a way to bypass the app's lock state.

## CLI integration on Niri

On the author's machine (Arch Linux with Niri), `op account list` returned `connection reset`
even though the 1Password app was running, its CLI socket existed, and restarting
the app did not help. PolKit was installed and active, but there was no user
authentication-agent process. The 1Password Linux CLI integration requires both
PolKit and an agent; the daemon alone is not enough. After enabling the agent,
the reset persisted. The Nix-store `op` binary was `root:root` mode `0555`, and
the `onepassword-cli` group did not exist. 1Password's Linux CLI setup requires
an `op` executable owned by that group with setgid enabled, which cannot be set
on a binary inside the read-only Nix store.

Enable Home Manager's `services.polkit-gnome` module so the agent runs in the
graphical session:

```nix
services.polkit-gnome.enable = true;
```

The Dojo configuration uses the existing `graphical-session.target`. Home
Manager no longer installs `_1password-cli`, so this native executable won't be
shadowed by the Nix profile. Install the already-verified Nix CLI binary at a
mutable system path with the permissions 1Password requires:

```sh
sudo groupadd --force onepassword-cli
sudo install --owner=root --group=onepassword-cli --mode=2755 \
  "$(readlink -f "$HOME/.nix-profile/bin/op")" /usr/local/bin/op
```

Then activate Home Manager with `devenv machines deploy "$(cat /etc/hostname)"`, open a new
terminal, and verify `command -v op` reports `/usr/local/bin/op` before retrying
`op account list`. The PolKit agent itself is user-managed; the `sudo` commands
are only for creating the CLI group and installing its setgid executable. The
manual `/usr/local/bin/op` copy does not receive future Nix updates; upgrade it
from a verified CLI release while preserving `root:onepassword-cli` ownership
and mode `2755`, or switch to Arch's `1password-cli` AUR package. Do not add the
Nix CLI back to `home.packages`, or it will shadow the setgid executable.

## Primary sources

- 1Password Support, [Install 1Password for Linux](https://support.1password.com/install-linux/) — supported Linux installation methods.
- 1Password Support, [Manage your unlock and auto-lock settings](https://support.1password.com/unlock-auto-lock/) — app lock/unlock and auto-lock settings.
- 1Password Support, [Use system authentication to unlock 1Password on Linux](https://support.1password.com/system-authentication-linux/) — Linux unlock behavior and cases requiring account-password unlock.
- 1Password Support, [Get to know Quick Access](https://support.1password.com/quick-access/) — Linux system-tray icon and Quick Access.
- 1Password Developer, [Use the desktop app to sign in to CLI](https://www.1password.dev/cli/app-integration) — Linux PolKit requirement and connection-reset troubleshooting.
- 1Password Developer, [Get started with CLI](https://www.1password.dev/cli/get-started) — manual Linux CLI installation's `onepassword-cli` group and setgid requirements.
- Home Manager, [`services.polkit-gnome`](https://github.com/nix-community/home-manager/blob/master/modules/services/polkit-gnome.nix) — user service tied to `graphical-session.target`.
- systemd upstream, [systemd.service(5)](https://www.freedesktop.org/software/systemd/man/latest/systemd.service.html) — semantics of `Restart=always` and restart exceptions.
- systemd upstream, [Inhibitor Locks](https://systemd.io/INHIBITOR_LOCKS/) — `PrepareForSleep` protocol and example handling before/after sleep.
- systemd upstream, [org.freedesktop.login1(5)](https://www.freedesktop.org/software/systemd/man/latest/org.freedesktop.login1.html) — logind manager sleep signals and session Lock/Unlock signals; session-manager responsibility.
- systemd upstream, [systemd-suspend.service(8)](https://www.freedesktop.org/software/systemd/man/latest/systemd-suspend.service.html) — sleep hooks are system-level, and applications should use the documented inhibitor/event mechanisms rather than treating hooks as general app restart triggers.
- systemd upstream, [systemd.special(7)](https://www.freedesktop.org/software/systemd/man/latest/systemd.special.html) — special user/session targets, including graphical-session integration.

## Scope note

The first-party 1Password Linux pages above do not prescribe a systemd-managed restart-on-resume recipe, and the cited official pages do not promise that a tray icon will always be restored by relaunching the app. Treat tray disappearance and process exit as distinct observations. No system configuration was changed as part of this research.
