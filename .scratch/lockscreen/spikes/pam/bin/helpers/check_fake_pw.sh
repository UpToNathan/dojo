#!/bin/bash
# stand-in password check. NEVER used with a real password; compares stdin with a fake constant.
IFS= read -r -d '' pw
[ "$pw" = "fake-spike-secret" ]
