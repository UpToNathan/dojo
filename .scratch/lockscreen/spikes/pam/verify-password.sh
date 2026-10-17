#!/bin/bash
# Interactive check of the REAL password path as an unprivileged process.  Run it yourself in a terminal:
#     /tmp/dojo-lock-pam-spike/verify-password.sh            # real /run/faillock tally (mirrors production)
#     FAILLOCK_DIR=/tmp/dojo-lock-pam-spike/data/verify/tally /tmp/dojo-lock-pam-spike/verify-password.sh   # private tally
# No sudo, nothing in /etc is touched.  The password is read with echo off on /dev/tty by the harness, handed
# straight to PAM's conversation function, and is never printed or logged.
#
# It runs a private-confdir stack that mirrors /etc/pam.d/system-auth (faillock preauth, pam_unix, faillock
# authfail/authsucc) as uid $(id -u).  A WRONG password adds one failure to the real tally (3 within 15 min
# locks sudo/ly/login for 10 min); a correct one clears it.  Reset: faillock --user "$USER" --reset
set -u
ROOT=/tmp/dojo-lock-pam-spike
CONF=$ROOT/data/verify
mkdir -p "$CONF"
extra=""
if [ -n "${FAILLOCK_DIR:-}" ]; then mkdir -p "$FAILLOCK_DIR"; extra=" dir=$FAILLOCK_DIR"; fi
cat > "$CONF/verify-system-auth" <<EOT
auth       required                    pam_faillock.so      preauth$extra
-auth      [success=2 default=ignore]  pam_systemd_home.so
auth       [success=1 default=bad]     pam_unix.so          try_first_pass nullok
auth       [default=die]               pam_faillock.so      authfail$extra
auth       optional                    pam_permit.so
auth       required                    pam_env.so
auth       required                    pam_faillock.so      authsucc$extra
EOT
fl() { if [ -n "${FAILLOCK_DIR:-}" ]; then faillock --dir "$FAILLOCK_DIR" --user "$USER" "$@"; else faillock --user "$USER" "$@"; fi; }
echo "== pre-flight: uid=$(id -u) user=$USER  (must NOT be 0)"
[ "$(id -u)" -ne 0 ] || { echo "run as your normal user"; exit 2; }
echo "-- tally dir/file:"; ls -ld "${FAILLOCK_DIR:-/run/faillock}" "${FAILLOCK_DIR:-/run/faillock}/$USER" 2>&1
echo "-- current failures:"; fl
echo "-- unix_chkpwd: $(stat -c '%A %U' /usr/bin/unix_chkpwd)"
echo
echo "== enter your password at the prompt (echo off). Ctrl-C aborts without recording anything."
START=$(date '+%Y-%m-%d %H:%M:%S')
python3 -I "$ROOT/bin/dojo_pam_race.py" --confdir "$CONF" --user "$USER" --worker pw=verify-system-auth --reply-tty --window 120
rc=$?
echo
echo "== post-flight failures in tally:"; fl
echo "== journal (auth lines since $START):"
journalctl --since "$START" --no-pager 2>/dev/null | grep -E 'unix_chkpwd|pam_unix\(verify|pam_faillock\(verify' | sed 's/^/   /' || true
echo
case $rc in
  0) echo "PASS (rc=0 PAM_SUCCESS): pam_unix verified your password unprivileged via unix_chkpwd, and the faillock stack ran to authsucc (tally above should be empty)." ;;
  1) echo "FAIL (rc=1): see the pam_authenticate result line above."
     echo "  PAM_AUTH_ERR + no 'locked' message  -> wrong password (or unix_chkpwd rejected: look for 'password check failed' in journal)."
     echo "  PAM_AUTH_ERR + 'account is locked'  -> 3+ recent failures; correct passwords are refused until 'faillock --user $USER --reset' or 10 min pass."
     echo "  PAM_SYSTEM_ERR / PAM_AUTHINFO_UNAVAIL / PAM_PERM_DENIED -> unprivileged helper problem (unix_chkpwd setuid bit, nscd/ shadow access, or faillock file perms); this would make the design need a different mechanism." ;;
  2) echo "TIMED OUT / no prompt answered (rc=2)." ;;
  *) echo "unexpected rc=$rc" ;;
esac
exit $rc
