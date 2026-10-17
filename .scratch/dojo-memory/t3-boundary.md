# Dojo memory T3 boundary findings

Inspected Dojo `tools/speech/voice.mjs`, `t3-client.mjs`, and `voice.test.mjs`. Inspected the actual installed T3 bundle in `~/.local/opt/t3code/resources/app.asar`, extracted read-only to `/tmp/dojo-t3-installed.mjs`. No threads/projects/config were mutated; no credentials or private logs were printed.

## Workspace isolation

`projects.mutate` accepts a project mutation directly, not wrapped in another field. Create payload:

```js
{ type: 'project.create', commandId, projectId, title: 'Dojo assistant',
  workspaceRoot: '/absolute/dojo/owned/workspace',
  createWorkspaceRootIfMissing: true }
```

Returns full `Project` with `id`, `workspaceRoot`, and other project metadata. Existing `orchestration:operate` scope authorizes it. Relevant extracted source lines 27528-27570, 48140, 294672, 129093-129114. Prefer stable persisted commandId/projectId, as T3 command receipts make replay safe. Do not retry project.create with a different commandId against an already created id. Project ids are branded trimmed strings, UUIDs work. `defaultModelSelection` is accepted by the external create schema but is ignored in the create implementation; select models explicitly on thread launch.

Alternative `projects.createNew({name:'Dojo assistant'})` creates a fresh Git repository under T3 baseDir/projects, README/icon, initial commit, and random project id. Returns `{projectId,workspaceRoot,commitError?}`. It is not idempotent across retries. This is less useful than a known Dojo state workspace. Relevant lines 27738, 274631-274716.

Current voice `projects.ensureScratch` uses a shared project identity, but T3 root-launch special handling actually claims a separate per-thread Scratch subfolder, then converts strategy to existing_worktree. Source 274608-274629 and 278649-278660. Current AGENTS writes therefore normally reach a private Scratch subfolder, not Scratch root. Nonetheless dedicated Dojo project is the explicit and verifiable isolation choice.

For a normal dedicated project, `workspaceStrategy:{type:'root'}` uses project.workspaceRoot and projection.thread.worktreePath is NULL. Voice must keep known workspaceRoot or compute `worktreePath ?? workspaceRoot`; current code only writes assistant files when worktreePath is truthy. Source 278418, 278511.

Never write AGENTS.md, .mcp.json, opencode.json, skills, or memory outside validated Dojo-owned workspace directories. Avoid reusing old conversation project/workspace paths during migration. Leave old chats intact. User global T3/Codex configuration need not change.

## Fresh thread launch and dispatch

Launch accepts:

```js
{ commandId, threadId, projectId, title, modelSelection,
  runtimeMode:'full-access', interactionMode:'default',
  workspaceStrategy:{type:'root'} }
```

Optional initialMessage is `{messageId?,text,context?,attachments:[]}` and starts a run immediately. To finish preparing workspace files before agent start, launch without initialMessage, subscribe, then dispatch. Returns `{threadId,projection,resumed}`. Explicitly use a new threadId per conversational turn and leave reuseExistingThread absent. Receipt replay with the same commandId is supported. Source 31525-31548, 278625 onward.

Dispatch uses existing voice format:

```js
{type:'message.dispatch',commandId,threadId,messageId,text,
 attachments:[],deliveryIntent:'auto',createdBy:'user',creationSource:'web',
 dispatchMode:{type:'start_immediately'}}
```

A dispatch may also supply modelSelection. Do not reuse previous providerThreadId or provider session identifiers when requiring fresh model context.

## Memory evidence

`orchestration.subscribeThread({threadId,afterSequence?,requestCompletionMarker:true})` is scoped to a single thread. It yields snapshots with projection.messages, projection.runs, projection.turnItems, and events with sequence/event. Persist received domain payloads without the UI preview truncation. Resume after saved sequence; deduplicate and only advance sequence after durable append.

Events include `message.updated`, `run.updated`, `turn-item.updated`. Turn items include assistant_message{text,messageId,streaming}, reasoning{text,streaming}, command_execution{input,output?,outputOmitted?,exitCode?}, dynamic_tool{toolName,input:unknown,output?:unknown,outputOmitted?}, file_change, file_search, web_search, subagent, and additional variants. Preserve arbitrary JSON payloads, not just final assistant messages. `turn-item.updated` is an upsert of full current state; many incremental events will repeat earlier text. Source 29863-30131.

If outputOmitted is true, `orchestration.getTurnItem({threadId,itemId,revision?:updatedAt})` returns `{item: turnItem|null}` with full size-bounded persisted input/output. Source 31728-31737. There is no exposed raw-provider-event subscription in the listed V2 RPC methods. The domain event carries optional rawEventId, but not the corresponding original JSON-RPC payload. Therefore call the stored stream “T3 orchestration events,” not unbounded raw provider traffic. T3 itself applies provider/output bounds.

## Summary model isolation

`modelSelection` is `{instanceId,model,options?:[{id,value:string|boolean}]}`. Routing uses configured instanceId. Driver, credentials, and home are resolved by T3 existing provider registry. Source 25657-25682, 16046.

Read `server.getConfig` only to inspect existing providers/models and default selection. `providers` is an array with instanceId, driver, enabled, installed, status, models[{slug,name,capabilities}], and supportedRuntimeModes. Model option descriptors are under capability/config contracts. Thread launch modelSelection makes summary model selection independent of the server default. Store foreground and summary selections in Dojo local config; never call server.updateSettings, server.updateProvider, auth changes, or global config writes.

Run each summary on a separate fresh thread with the local summary modelSelection. For concurrent summary and foreground runs, use separate Dojo-owned working directories or serialize writes to shared AGENTS/memory files. Easiest robust preparation is immutable per-thread workspace files with launch using existing_worktree path, but a dedicated project root with stable AGENTS and memory supplied in prompt avoids shared instruction mutation. `existing_worktree` path accepts a plain absolute folder in the inspected launch implementation; it need not be a Git worktree. This is the same strategy T3 uses for Scratch subfolders.

## Existing tests

Tests cover queue bounds, stream replay, snapshot fallback, coalesced sequence writes, final assistant deduplication, local socket singleton behavior, and delayed chunk acknowledgement. They do not currently cover project creation, fresh per-turn launches, ownership path validation, omitted tool-output reads, summary model selection, or memory evidence persistence.

## Additional exact schemas and live read-only check

Turn item statuses are idle, pending, running, waiting, completed, failed, cancelled, interrupted. Assistant turn items use type=assistant_message and have messageId/text/streaming, not a role field. projection.messages instead uses role. Generic tool item type is dynamic_tool, not tool_call. Base fields are id/threadId/runId/nodeId/providerThreadId/providerTurnId/nativeItemRef/parentItemId/ordinal/status/title/startedAt/completedAt/updatedAt, with optional tool metadata.

Project mutation response is the raw project `{id,title,workspaceRoot,defaultModelSelection,scripts,createdAt,updatedAt,deletedAt,...}`. There is no standalone project.get WebSocket RPC among the installed methods. `orchestration.subscribeShell` snapshot contains projects; prefer replaying the persisted create commandId receipt to avoid globally enumerating projects. Creation implementation ignores defaultModelSelection despite schema accepting it; per-thread explicit selection remains correct.

Live read-only `server.getConfig` using an already-valid existing Dojo session confirmed `{instanceId:"codex",model:"gpt-6-luna"}` default and a ready/enabled/installed Codex provider. The Luna advertised reasoningEffort choices are low/medium/high/xhigh/max, default medium. It does not advertise none; existing voice force-injects none, so run behavior requires a separate bounded live experiment authorized by the main agent.
