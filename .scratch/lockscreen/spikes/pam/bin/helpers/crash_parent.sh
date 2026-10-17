#!/bin/bash
# kills the PAM worker (our parent) with SIGKILL: simulates a hard worker crash.
sleep 0.3
kill -9 "$PPID"
sleep 5
