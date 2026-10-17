#!/bin/bash
# usage: sleep_ok.sh SECONDS MARKERDIR NAME   -- stand-in "slow fingerprint match"
touch "$2/$3.started"
sleep "$1"
touch "$2/$3.completed"
exit 0
