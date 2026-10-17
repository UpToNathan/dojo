#!/bin/bash
# Interactive: the REAL race.  Fingerprint worker (pam_fprintd) vs password worker (pam_unix+faillock) as your
# normal user, using the recommended stacks in a private confdir.  Touch the sensor OR type your password.
# Locks nothing, installs nothing.  A wrong password adds one failure to the real faillock tally
# (reset: faillock --user $USER --reset).  Set FAILLOCK_DIR=... to use a private tally instead.
set -u
ROOT=/tmp/dojo-lock-pam-spike; CONF=$ROOT/data/race-real; mkdir -p "$CONF"
cp "$ROOT/recommended/dojo-lock-fingerprint" "$CONF/"
if [ -n "${FAILLOCK_DIR:-}" ]; then mkdir -p "$FAILLOCK_DIR"; sed "s#pam_faillock.so \(preauth\|authfail\|authsucc\)#pam_faillock.so \1 dir=$FAILLOCK_DIR#" "$ROOT/recommended/dojo-lock-password" > "$CONF/dojo-lock-password"
else cp "$ROOT/recommended/dojo-lock-password" "$CONF/"; fi
exec python3 -I "$ROOT/bin/dojo_pam_race.py" --confdir "$CONF" --user "$USER" --worker fp=dojo-lock-fingerprint --worker pw=dojo-lock-password --reply-tty --window 60
