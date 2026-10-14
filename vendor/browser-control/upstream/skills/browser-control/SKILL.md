---
name: browser-control
description: Inspect or operate an existing browser tab in Dojo through the browser_control MCP server. Use for browser navigation, authenticated web workflows, forms, and human steps such as passkeys, 2FA, CAPTCHAs, or payment confirmation.
---

# Browser Control in Dojo

Use the installed `browser_control` MCP server. Its commands run through Dojo's
native browser admission and the Browser Control extension in the user's Helium
profile. The calling agent chooses the task and verifies its outcome.

Use the MCP tools for browser work. Do not launch another relay, use a raw CDP
connection, call an upstream npm installation, change the wrapper's Dojo mode, or
retry a refused desktop action through browser tools. CLI `doctor` and `status`
are available for setup diagnosis. Direct CLI `execute` does not carry this MCP
client's task ownership and must not replace its browser tools.

## Start a task

1. Call `session_resume` to obtain this client's session and a new native lease
   generation. Keep the returned session ID for subsequent tool calls. Use the
   current implicit session when no explicit session ID is needed.
2. Select the intended page. For an existing user tab, the user must attach it
   with the Browser Control toolbar button. Call `session_adopt` with the exact
   attached target selector when the task should keep using that tab. A selector
   chooses a tab and does not navigate it.
3. Call `execute` to inspect the page before choosing an action:

   ```js
   return { url: page.url(), title: await page.title(), outline: await snapshot() }
   ```

Completion means the session has an active native generation and the inspected
page is the intended target. A tool result acknowledging an action is not proof
that the requested outcome happened.

`session_new` can create another owned session when needed. An MCP client cannot
resume, adopt, execute, or cancel another client's session by guessing its ID.
The MCP process retains its owner identity. Closing it retires its sessions;
use a fresh generation when a new process begins.

## Inspect, act, and verify

Use the fresh snapshot's references or Playwright roles and labels. Keep actions
small enough to inspect their results before proceeding.

```js
await ref("e12").click()
await page.getByRole("heading", { name: "Settings" }).waitFor()
return { url: page.url(), outline: await snapshot() }
```

Use `page.goto` for requested navigation and ordinary Playwright locators for
page controls. Derive selectors from the real page. If a native fill fails,
inspect the failure before using the explicit `fillInput` helper. Protected
extension UI requires the user's action.

For an outward action such as sending a message, submitting a form, booking, or
payment, act once and read the specific receipt or confirmation. A click return,
HTTP status, redirect, or cleared field alone does not prove delivery. If the
result is refused, cancelled, partial, timed out, or unknown, inspect after a
new authorized generation and report what is known. Do not repeat the mutation
to resolve uncertainty.

Browser Control observations identify a browser tab. Do not describe them as an
exact native-window match or as a screenshot-authorized desktop input receipt.

## Yield to the human

When a passkey, login, 2FA, CAPTCHA, protected extension, payment confirmation, or
other human step is needed, stop browser execution and call `execute_cancel` for
this session before handing control to the user. Dojo calls this transition
`YieldHuman`. State the concrete step the user must complete and wait for their
response. Dojo's voice runtime performs the retirement as part of its human
request lifecycle; other MCP clients must call `execute_cancel` themselves.

Cancellation retires the current generation, rejects queued work, and disconnects
its Playwright transport. It keeps the tab open. Already admitted Chrome work
may finish, so report any unresolved outcome and never replay it automatically.
An extension progress pill or in-page handoff acknowledgement alone does not
retire the native lease.

After the user reports completion, call `session_resume` to obtain a fresh
generation. Reinspect the same adopted tab and verify the actual destination or
receipt. Human activity on a browser window can retire a generation even when
the agent did not request a handoff. Treat that refusal as a handover and follow
the same resume and inspection sequence.

## Finish or interrupt

Call `execute_cancel` when the task completes, is cancelled, is replaced by a new
task, or must yield to a human. Do not retain an active generation between tasks.
Keep the adopted tab open for the user. A later task starts with `session_resume`
and a new observation. Cancellation is safe to repeat for a retired generation.

Do not reset or delete sessions to escape a refusal or clear an unknown effect.
Those upstream operations are unavailable through the Dojo MCP tool set.

## Setup and recovery

The unpacked extension has a stable installed path:

```text
~/.local/share/dojo/browser-control/extension
```

Helium's maintained launcher loads this directory. A browser process already
running before package activation needs a normal browser restart to read new
launch flags. Do not stop the user's browser to complete an agent task. If setup
requires manual approval, open the browser's extension page and have the user
approve this installed extension.

`browser-control doctor` and `browser-control status --json` report relay and
extension state without starting a relay. Missing native admission, a missing
broker, extension disconnection, or a stale build is a setup failure. Preserve
its diagnostic and stop browser actions until the configured path works.
Ordinary operations never replace a running relay. A coordinated
`browser-control relay restart` drains work and preserves tabs; never force-kill
another agent's relay or bypass the native transport to repair it.

Never put credentials in execute source or logs. Inspect the minimum page data
needed for the requested task and return concrete evidence of its outcome.
