# Notify when a software update run finishes

Status: ready-for-agent
Type: task
Blocked by: 01

`scripts/dojo-update` calls `dojo-notify` from the end of `main()`: finished, or finished with failed or blocked steps, naming them. A missing `dojo-notify` is ignored.
