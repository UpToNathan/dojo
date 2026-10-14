# Native browser transport contract

The pinned relay launches `dojo-browser-control-broker --stdio`. Stdin/stdout are JSON lines; logs go to stderr. One broker owns loopback `127.0.0.1:19988`. Node retains relay HTTP port 19989 and owns no extension WebSocket. The extension connects to `ws://127.0.0.1:19988/extension`; require Origin `chrome-extension://eibhgjafffkigblngnhafgbcipofaeon`. No compression negotiation. Existing upstream extension JSON message/response shapes are unchanged inside the `message` field below, except for lease/generation metadata carried in the command by the fork.

Requests:

```json
{"op":"bind","id":0,"lease":"pi:task-id","generation":1}
{"op":"send","id":1,"lease":"pi:task-id","generation":1,"command":{"id":7,"method":"debugger.sendCommand","params":{}}}
{"op":"retire","id":2,"lease":"pi:task-id","generation":1}
```

Responses/events:

```json
{"op":"result","id":0,"status":"bound"}
{"op":"result","id":1,"status":"admitted"}
{"op":"result","id":2,"status":"retired"}
{"op":"result","id":1,"status":"refused","reason":"human ownership"}
{"op":"result","id":1,"status":"unknown","reason":"partial write"}
{"op":"extension","message":{"id":7,"result":{}}}
{"op":"connected"}
{"op":"disconnected"}
```

`id` correlates stdio requests. `command.id` is the existing upstream extension RPC request ID, unique across the relay. The native actor queues at most one in-flight browser command and bounded pending commands. Broker automatically sends a native settlement when it receives that exact extension response before forwarding it to Node. A Chrome error conservatively retires that command as unknown; no replay. Node rejects refused/unknown sends and feeds extension messages into existing ExtensionRpc event/response handlers. Connected/disconnected events replace the former Node socket lifecycle. All extension RPCs require the owning lease/generation, including helper commands. Sessionless browser operations refuse. Extension ping/pong WebSocket control has a tightly typed native control route; JSON application ping goes through ordinary command admission.

Bind the exact session generation before operational resume/adoption and recheck it before the execute worker starts user JavaScript. Bind is idempotent for a live generation and refuses a retired generation or current human browser ownership. A send requires this prior binding. This lets native takeover retire a script that has not yet produced its first browser command. Bind is not a transferable ownership permit; every send still checks native ownership in the write actor.

`retired` means Niri installed the exact-generation tombstone, rejected its pending commands and will write no later application frame for it. It does not mean Chrome canceled admitted work. Retirement bypasses the execution semaphore. The upstream extension must also install its own tombstone and acknowledge it; a narrow retire notification is emitted through the native typed retirement route, not through arbitrary exempt application bytes. Native retirement proceeds even with a missing/disconnected extension. Pi records any already-admitted unanswered command as unknown. Old generations never reopen; new generation binding must still pass native human ownership checks. No durable broker task journal.

Native ingress is a same-UID Unix datagram socket at `$DOJO_BROWSER_CONTROL_SOCKET` or `$XDG_RUNTIME_DIR/dojo-browser-admission.sock`. Niri enables it only with operator-pinned immutable `$DOJO_BROWSER_CONTROL_BROKER_EXE`. It authenticates SO_PASSCRED sender uid and `/proc/<pid>/exe` against that path. The broker transfers the sole application write descriptor via SCM_RIGHTS after HTTP upgrade; its sealed duplicate only uses recv. Niri sends unmasked bounded text frames directly with nonblocking send. Partial writes retire the connection as unknown. Native human input never waits for this connection.

Bounds are 1 MiB for the complete stdio JSON line including its newline, Unix request packet, and outbound application payload; 32 pending commands; one browser command in flight. Broker and Niri request 2 MiB send and receive socket buffers and verify at least 2 MiB effective kernel capacity. This includes the native TCP writer. No sysctl changes occur; inadequate kernel buffer limits refuse startup/registration. Node checks the complete UTF-8 line size before writing. Browser-client scope includes configured Helium/Chromium app IDs; ambiguous/no browser native identity refuses mutation. Human activity on any browser client retires all browser leases in Niri's input actor. No exact tab-to-window claim.

The operator explicitly hands control back with `niri msg yield-human-input`. It clears native ownership without moving the cursor or changing keyboard focus. Release held keys and pointer grabs first; Niri refuses yield while input is held. If invoked from a terminal while Enter is still pressed, use `sleep 0.2; niri msg yield-human-input` to allow its release. Relay and Pi never call this automatically. Yield does not reopen a retired browser session generation; Pi starts a fresh generation after its existing outcome resolution. Incoming extension JSON replies allow 32 MiB and bounded fragmentation for screenshots; native outbound frames remain 1 MiB and unfragmented.
