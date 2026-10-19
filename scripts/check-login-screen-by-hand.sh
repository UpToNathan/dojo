#!/usr/bin/env bash
# Walks a human through the parts of the Dojo login screen that only a real boot can show.
# It runs read-only checks itself and never performs a power action: you do those, and it
# records what you saw. Run it from a terminal on a second TTY (tty2 is the rescue console).
set -euo pipefail

log=${XDG_STATE_HOME:-"$HOME/.local/state"}/dojo/login-checklist.log
mkdir -p -- "$(dirname -- "$log")"

say() { printf '\n\033[1m%s\033[0m\n' "$*"; }
record() { printf '%s %s\n' "$(date -Is)" "$*" | tee -a "$log"; }

ask() {
    local answer
    printf '%s [y/n/s=skip] ' "$1"
    read -r answer
    case $answer in
        y | Y) record "ok: $1" ;;
        s | S) record "skipped: $1" ;;
        *) record "FAILED: $1"; failed=1 ;;
    esac
}

failed=0
pass() { printf '  ok   %s\n' "$1"; }
fail() { printf '  FAIL %s\n' "$1"; failed=1; }
check() { local what=$1; shift; if "$@" >/dev/null 2>&1; then pass "$what"; else fail "$what"; fi; }

say 'Preflight (read-only)'
release=/usr/local/lib/dojo-dm/current
check 'release installed' test -x "$release/dojo-dm"
check 'release is root-owned' test "$(stat -c %U "$(readlink -f "$release")")" = root
check 'dojo-dm-pam loads libpam' "$release/dojo-dm-pam" --check
check 'dojo-greeter user exists' getent passwd dojo-greeter
check 'login PAM files installed' test -f /etc/pam.d/dojo-login -a -f /etc/pam.d/dojo-greeter
check 'dojo-dm.service enabled' systemctl is-enabled dojo-dm.service
check 'tty2 rescue getty enabled' systemctl is-enabled getty@tty2.service
check 'ly no longer enabled' bash -c '! systemctl is-enabled ly@tty2.service'
check 'power key left to Niri' grep -q '^HandlePowerKey=ignore' /etc/systemd/logind.conf.d/90-dojo-power-key.conf
check 'session entry present' test -f /usr/share/wayland-sessions/niri-uwsm.desktop
[[ $failed -eq 0 ]] || { say 'Fix the failures above before rebooting (scripts/install-system-config.sh, scripts/bootstrap-host login).'; exit 1; }

say 'Way back, before anything else'
cat <<'TEXT'
  tty2 is a plain login console. If the login screen does not appear after the reboot, press
  Ctrl+Alt+F2, log in, then either read the logs:
      journalctl -b -u dojo-dm.service
  or roll back to ly:
      sudo systemctl disable dojo-dm.service
      sudo systemctl enable ly@tty2.service getty@tty1.service
      sudo reboot
TEXT
ask 'Have you logged in on tty2 once, so you know the way back works?'

say 'Login'
echo '  Reboot now (the script cannot do it for you), unlock the disk, then answer the rest after you are logged in.'
ask 'The login screen looked like the lock screen (dial, wallpaper, chips)?'
ask 'Your name was prefilled, and typing went straight to the password pill?'
ask 'A wrong password shook the pill and showed an error colour?'
ask 'Clicking or Tab on the user chip let you type another user, and the look changed to theirs?'
ask 'The correct password played a greeting, and the desktop frame opened out of it?'
ask 'The GNOME keyring is unlocked (1Password and others did not prompt)?'
if fprintd-list "$USER" 2>/dev/null | grep -q 'finger'; then
    ask 'On this machine a fingerprint also logged you in (then the keyring stays locked, as documented)?'
fi

say 'Lock and unlock still work'
ask 'Super+Alt+L locks, and the unlock greeting opens back into the desktop?'

say 'Power occasions (do each yourself, one at a time)'
ask 'Menu: Log out played a farewell, then the login screen came back?'
ask 'Menu: Restart played a farewell before the machine rebooted?'
ask 'Menu: Shut down played a farewell before power off? (power it on again)'
ask 'A short press of the power button played the shutdown farewell?'
ask 'A terminal "systemctl reboot" played a farewell too (the bridge)?'
ask 'Closing the lid on battery locked, played the farewell, and suspended?'
ask 'Menu: Hibernate played a farewell and resumed to the lock screen?'
ask 'Holding the power button for several seconds still forces power off on a hung machine (only if you dare)?'

say 'Result'
if [[ $failed -eq 0 ]]; then record 'login screen checklist: all answered ok or skipped'; else record 'login screen checklist: FAILURES, see above'; fi
echo "  Log: $log"
exit "$failed"
