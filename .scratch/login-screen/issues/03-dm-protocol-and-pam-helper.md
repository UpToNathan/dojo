# Greeter protocol and the root PAM helper

Status: ready-for-agent
Blocked by: 01

Define the greeter socket protocol (`Hello`, `SelectUser`, `Authenticate`, PAM prompts, info and errors, pause, success, failure, `Power`) and `dojo-dm-pam`, the root helper that speaks the lock worker's wire format, runs `dojo-login` and `dojo-login-fingerprint`, and on success continues as session leader. Reuse `lock::worker::{libpam,wire}`; add `pam_acct_mgmt`, `pam_setcred`, `pam_open_session`, `pam_close_session` and `pam_getenvlist`. The user comes from the protocol, validated against `getpwnam`, never from argv.

## Acceptance

- The scenarios of the lock PAM suite pass against a fake stack with `dojo-dm-pam` as root-less test builds.
- Password and fingerprint helpers race; the winner is the session leader and the loser is killed with its tree.
- Peer credentials must be `dojo-greeter`; anything else is dropped.

## Comments
