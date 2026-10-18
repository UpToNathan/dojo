import { readFile, mkdir, writeFile, chmod } from 'node:fs/promises';
import { join } from 'node:path';
import { randomUUID } from 'node:crypto';
import { openDesktopExtension } from './desktop-extension.mjs';
import { openBrowserExtension } from './browser-extension.mjs';
import { openCuaExtension } from './cua-extension.mjs';
import { installComputerSteps } from './computer-steps.mjs';
import { computerModelBridge } from './computer-model-bridge.mjs';
import { exportDesktopTrace } from './desktop-trace.mjs';
import { installHumanRequests, respondHuman, cancelHumanRequests } from './human-requests.mjs';
import { projectDesktopImages } from './image-retention.mjs';

const textContent = content => typeof content === 'string' ? content : (content ?? []).filter(x => x.type === 'text').map(x => x.text).join('\n');
const archiveResult = content => Array.isArray(content) && content.some(block => block.type !== 'text') ? JSON.stringify(content) : textContent(content);
const resultText = value => typeof value === 'string' ? value : JSON.stringify(value);
const prefix512 = text => { let result = ''; for (const char of text) { if (Buffer.byteLength(result + char) > 512) break; result += char; } return result; };

export function summaryTask(text, context = {}) {
  const ruler = '-'.repeat(512);
  if (context.kind === 'merge') {
    const id = context.id ?? context.ref[1] * 2 ** context.ref[0];
    const n = context.count ? context.count / 2 : 2 ** (context.ref[0] - 1);
    return `Compaction: merge lines ${id}+${n} and ${id + n}+${n}, adjacent, into one line of at most\n512 bytes (about 70 words), the length of this ruler:\n${ruler}\n<chat> may hold their messages, ${id} to ${id + n * 2 - 1}, in more detail: take details\nof them from there too.\n<input>\n${text}\n</input>`;
  }
  return `Compaction: compress message ${context.message?.i ?? context.ref?.[1] ?? context.id ?? 0} into one line of at most 512 bytes\n(about 70 words), the length of this ruler:\n${ruler}\n<input>\n${text}\n</input>`;
}

export class PiRuntime {
  static async open({ stateDir, root, memory, config = {}, onEvent = () => {} }) {
    const [pi, chord, envModule, coding, ai] = await Promise.all([
      import('@earendil-works/pi-durable'), import('@earendil-works/chord/context'),
      import('@earendil-works/pi-durable/env/node'), import('@earendil-works/pi-durable/tools'), import('@earendil-works/pi-ai'),
    ]);
    const runtime = new PiRuntime();
    Object.assign(runtime, { stateDir, root, memory, config, onEvent, pi, context: chord.BACKGROUND_CONTEXT, preparing: false, queue: Promise.resolve(), archived: Promise.resolve(), archiveSerial: Promise.resolve(), summaryJobs: new Set(), closed: false });
    runtime.summarize = runtime.summarize.bind(runtime);
    const foregroundRequestMessages = new WeakSet();
    runtime.State = pi.defineDoc({ kind: 'dojo.runtime', version: 1, scope: 'conversation', history: 'latest', fork: 'initial', initial: () => ({ mode: 'foreground', view: '<chat>\n</chat>', admissions: {}, archiveCursor: 0 }) });
    const instructions = config.instructions ?? (await readFile(join(root, 'tools/speech/memory-prompt.md'), 'utf8') + '\n\n' + await readFile(join(root, 'tools/speech/assistant.md'), 'utf8'));
    const workspace = join(stateDir, 'workspace');
    await mkdir(workspace, { recursive: true, mode: 0o700 });
    const registry = pi.createRegistry(); registry.install(coding.CodingTools);
    await installHumanRequests(runtime, registry, ai);
    const memoryTools = [
      pi.defineTool({ name: 'zoom', description: 'Open line id+n into its two children; n=1 gives the original message. n must be a power of two and id a multiple of n. Long messages come in pages.', parameters: ai.Type.Object({ id: ai.Type.Integer({ minimum: 0 }), n: ai.Type.Integer({ minimum: 1 }), offset: ai.Type.Optional(ai.Type.Integer({ minimum: 0 })) }), replay: 'safe', execute: async args => ({ output: (await runtime.zoom(args)).content }) }),
      pi.defineTool({ name: 'date', description: 'The date and time of message id.', parameters: ai.Type.Object({ id: ai.Type.Integer({ minimum: 0 }) }), replay: 'safe', execute: async args => ({ output: [{ type: 'text', text: resultText(await memory.date(args.id)) }] }) }),
    ];
    // The browser and desktop helpers start independently, so their startup overlaps.
    const browserOpening = config.browserTools !== false && (config.desktopTools !== false || config.browserClientFactory)
      ? openBrowserExtension(runtime) : undefined;
    browserOpening?.catch(() => {});
    try {
      if (config.desktopTools !== false) {
        registry.install(await openDesktopExtension(runtime));
        await installComputerSteps(runtime, registry, ai);
      }
      if (browserOpening) registry.install(await browserOpening);
      if (config.cuaTools || config.cuaClientFactory) registry.install(await openCuaExtension(runtime));
    } catch (error) {
      await browserOpening?.catch(() => {});
      await runtime.closeBrowser?.();
      runtime.releaseCua?.('Dojo runtime initialization failed');
      runtime.releaseDesktop?.('Dojo runtime initialization failed');
      throw error;
    }
    registry.install(pi.defineExtension({ name: 'dojo-memory', tools: memoryTools, sections: [pi.section('dojo', () => instructions, { tag: false })], hooks: [
      pi.hook(pi.GenerationTask, { beforeRequest: async (request, api, context) => {
        const state = await api.snapshot(runtime.State, api.conversationId, context);
        const messages = [...request.messages];
        const firstUser = messages.findIndex(message => message.role === 'user');
        if (firstUser >= 0) {
          const view = { role: 'user', content: state?.view ?? '<chat>\n</chat>', timestamp: 0 };
          if (api.conversationId === runtime.id && state?.mode === 'foreground') foregroundRequestMessages.add(view);
          messages.splice(firstUser, 0, view);
        }
        return { messages: projectDesktopImages(messages, config.desktopRequestImages ?? 8) };
      } }),
      pi.hook(pi.ToolTask, { beforeTool: async (_call, api, context) => (await api.snapshot(runtime.State, api.conversationId, context))?.mode === 'summary' ? { block: 'Compactions must call no tools.' } : undefined }),
      pi.hook(pi.CompactionTask, { beforeCompact: () => ({ decline: true }) }),
    ] }));
    await mkdir(stateDir, { recursive: true });
    const prepared = config.models ? undefined
      : await (await import('./accounts.mjs')).prepareAgentModels({ stateDir, config, cacheId: `dojo:${stateDir}`, credentials: config.credentials });
    let models = config.models ?? prepared.models;
    runtime.accounts = prepared?.accounts ?? []; runtime.modelRows = prepared?.rows ?? [];
    runtime.credentialStore = prepared?.credentials;
    runtime.selection = prepared ? prepared.selection : { account: config.provider ?? 'openai-codex', model: config.model ?? 'gpt-6.1-sol' };
    runtime.selectionAvailable = prepared?.selectionAvailable ?? true;
    runtime.selectionReason = prepared?.selectionReason;
    runtime.summaryModelId = prepared?.summaryModel;
    runtime.models = models;
    if (config.desktopModelProfile) models = computerModelBridge(models, runtime, config.desktopModelProfile, foregroundRequestMessages);
    const database = join(stateDir, 'pi.sqlite');
    if (!config.storage) {
      await writeFile(database, '', { mode: 0o600, flag: 'ax' }).catch(error => { if (error.code !== 'EEXIST') throw error; });
      await chmod(database, 0o600);
    }
    const storage = config.storage ?? await (await import('@earendil-works/pi-durable/storage/sqlite/node')).openNodeSqliteStorage(database);
    const firstAvailable = prepared?.rows?.[0];
    const runtimeSelection = runtime.selectionAvailable ? runtime.selection : firstAvailable
      ? { account: firstAvailable.account, model: firstAvailable.model }
      : { account: config.provider ?? 'openai-codex', model: config.model ?? 'gpt-6.1-sol' };
    const agentProvider = runtimeSelection.account;
    runtime.agent = { model: { provider: agentProvider, modelId: runtimeSelection.model }, thinkingLevel: config.thinkingLevel ?? (agentProvider === 'anthropic' ? 'xhigh' : 'low'), cwd: workspace };
    runtime.harness = await pi.Harness.open(storage, { models, registry, env: () => new envModule.NodeExecutionEnv({ cwd: workspace }), settings: { compaction: { enabled: false, backgroundTokens: 0 }, retry: { maxRetries: 0 }, steeringMode: 'all' } }, runtime.context);
    runtime.conversation = await runtime.harness.root(runtime.context, { agent: runtime.agent });
    runtime.id = runtime.conversation.id;
    await runtime.conversation.configure(runtime.agent, runtime.context);
    // A reopened MCP process has a new session; historical capture IDs cannot authorize effects.
    if (runtime.invalidateDesktop) await runtime.invalidateDesktop();
    await runtime.mirror();
    const human = await runtime.harness.snapshot(runtime.HumanState, runtime.id, runtime.context);
    const pendingHuman = Object.values(human?.requests ?? {}).filter(request => request.status === 'pending' && request.expiresAt > Date.now());
    if (pendingHuman.length) runtime.onEvent({ type: 'human_request', request: pendingHuman.at(-1) });
    runtime.watch = await runtime.conversation.watch(runtime.context);
    runtime.live = runtime.watch.value.docs['pi.live'];
    runtime.watch.start(async value => {
      const wasRunning = !!runtime.live?.run;
      runtime.live = value.docs['pi.live'];
      const cleanup = wasRunning && !runtime.live?.run ? Promise.all([runtime.finishDesktopInput?.(), runtime.finishBrowserInput?.()]) : undefined;
      runtime.archived = runtime.archived.then(async () => { await cleanup; await runtime.mirror(); });
      await runtime.archived;
      const run = runtime.live?.run;
      runtime.onEvent({ type: 'run', status: run ? 'running' : 'idle', runId: run?.taskId, conversationId: runtime.id });
    });
    await runtime.recoverAdmissions();
    return runtime;
  }
  async zoom({ id, n, offset = 0 }) {
    const page = await this.memory.zoom(id, n, offset);
    const content = [{ type: 'text', text: resultText(page) }];
    if (n === 1 && offset === 0 && this.memory.readRecord) {
      const record = await this.memory.readRecord(id);
      const match = new RegExp(`^pi:${this.id}:([0-9]+):result$`).exec(record?.key ?? '');
      if (match) {
        const entryId = Number(match[1]);
        if (Number.isSafeInteger(entryId)) {
          const entry = await this.conversation.commit(tx => tx.entry(this.pi.ToolResultEntry, entryId), this.context);
          if (entry?.conversationId === this.id && entry.kind === this.pi.ToolResultEntry.kind)
            content.push(...(entry.model?.[0]?.content ?? []).filter(block => block.type === 'image'));
        }
      }
    }
    return { content };
  }
  respondHuman(id, response) { return respondHuman(this, id, response); }
  exportDesktopTrace(options) { return exportDesktopTrace(this, options); }
  busy() { return this.preparing || !!this.live?.run; }
  async state() { return await this.harness.snapshot(this.State, this.id, this.context) ?? this.State.definition.initial(); }
  mirror() {
    const mirror = this.archiveSerial.then(() => this.drainArchive());
    this.archiveSerial = mirror.catch(() => {});
    return mirror;
  }
  async drainArchive() {
    const state = await this.state();
    let cursor, appended = false;
    do {
      const page = await this.conversation.entries({ order: 'ascending', minEntryId: state.archiveCursor + 1 }, 200, cursor, this.context);
      for (const entry of page.items) {
        const date = new Date(entry.model?.[0]?.timestamp ?? 0).toISOString();
        const threadId = String(this.id);
        if (entry.kind === 'pi.assistant') {
          appended = true;
          const message = entry.model?.[0];
          const text = textContent(message?.content);
          if (text) { await this.memory.append({ key: `pi:${this.id}:${entry.id}:text`, kind: 'unii', text, date, threadId }); this.onEvent({ type: 'response', text }); }
          // A failed model call ends the run with an empty reply; say so instead of leaving the user waiting.
          else if (message?.stopReason === 'error') this.onEvent({ type: 'run', status: 'error', error: message.errorMessage || 'The model request failed.', conversationId: this.id });
          for (const call of message?.content ?? []) if (call.type === 'toolCall') await this.memory.append({ key: `pi:${this.id}:${entry.id}:call:${call.id}`, kind: 'tool', text: `${call.name} ${JSON.stringify(call.arguments)}`, date, threadId });
        } else if (entry.kind === 'pi.tool-result') { appended = true; await this.memory.append({ key: `pi:${this.id}:${entry.id}:result`, kind: 'echo', text: archiveResult(entry.model?.[0]?.content), date, threadId }); }
        await this.conversation.commit(async tx => { (await tx.doc(this.State, this.id)).archiveCursor = entry.id; }, this.context);
      }
      cursor = page.next;
    } while (cursor !== undefined);
    if (appended) this.scheduleCompaction();
  }
  scheduleCompaction() {
    if (!this.memory.compact || this.closed) return;
    // compact() hands every caller the run already in flight, so a second handler would report
    // that run's failure again. The first one reports it.
    if (this.memory.compacting) return;
    this.compaction = this.memory.compact(this.summarize, { shouldStop: () => this.closed }).then(result => {
      if (result?.errors?.length) this.onEvent({ type: 'memory', status: 'error', error: result.errors.map(error => error.message ?? String(error)).join('; ') });
    }).catch(error => this.onEvent({ type: 'memory', status: 'error', error: error.message }));
  }
  async reloadAccountCatalog() {
    if (!this.credentialStore || !this.models?.setProvider) return;
    const { prepareAgentModels } = await import('./accounts.mjs');
    const prepared = await prepareAgentModels({ stateDir: this.stateDir, config: { ...this.config, modelSelection: this.selection },
      cacheId: `dojo:${this.stateDir}`, credentials: this.credentialStore });
    for (const id of prepared.models.dojoAccounts) {
      const provider = prepared.models.getProvider(id);
      if (provider) this.models.setProvider(provider);
    }
    this.accounts = prepared.accounts;
    this.modelRows = prepared.rows;
    if (!this.selection && prepared.selection) this.selection = prepared.selection;
    this.selectionAvailable = prepared.selectionAvailable;
    this.selectionReason = prepared.selectionReason;
    this.summaryModelId = prepared.summaryModel;
    if (!this.busy() && this.selectionAvailable) await this.applyModelSelection();
  }
  async setModelSelection(selection, { available, reason, summaryModelId } = {}) {
    this.selection = { account: selection.account, model: selection.model, ...(selection.name ? { name: selection.name } : {}) };
    this.selectionAvailable = !!available;
    this.selectionReason = reason;
    this.summaryModelId = summaryModelId;
    if (!this.busy() && this.selectionAvailable) await this.applyModelSelection();
  }
  async applyModelSelection() {
    if (!this.selectionAvailable || !this.selection) throw new Error(this.selectionReason || 'Sign in to use Dojo.');
    if (this.agent?.model?.provider === this.selection.account && this.agent.model.modelId === this.selection.model) return;
    this.agent = { ...this.agent, model: { provider: this.selection.account, modelId: this.selection.model } };
    await this.conversation.configure({ model: this.agent.model }, this.context);
  }
  async submit(text, { requestId = randomUUID() } = {}) {
    if (typeof text !== 'string' || !text.trim()) throw new Error('Dojo input must be nonempty text');
    if (this.closed) throw new Error('Dojo runtime is closed');
    const receipt = await this.conversation.commit(async tx => {
      const state = await tx.doc(this.State, this.id);
      if (state.admissions[requestId]) return { requestId, conversationId: this.id, duplicate: true };
      state.admissions[requestId] = { text, date: new Date().toISOString(), status: 'pending', steer: this.busy() };
      return { requestId, conversationId: this.id, duplicate: false };
    }, this.context);
    if (!receipt.duplicate) {
      this.preparing = true;
      this.queue = this.queue.then(() => this.dispatch(requestId)).catch(error => { this.onEvent({ type: 'run', status: 'error', error: error.message, conversationId: this.id }); });
    }
    return receipt;
  }
  async dispatch(requestId) {
    const admission = (await this.state()).admissions[requestId];
    if (this.closed || admission.status !== 'pending') return;
    const existing = await this.conversation.commit(tx => tx.submissionByRequest(this.id, requestId), this.context);
    if (existing) {
      await this.conversation.commit(async tx => { const item = (await tx.doc(this.State, this.id)).admissions[requestId]; item.status = 'submitted'; item.submissionId = existing.id; }, this.context);
      this.preparing = false;
      return;
    }
    try {
      if ((!this.selectionAvailable || !this.selection) && await this.credentialStore?.unlock?.()) await this.reloadAccountCatalog();
      if (!this.selectionAvailable || !this.selection) {
        const error = new Error(this.selectionReason || 'Sign in to use Dojo.');
        await this.conversation.commit(async tx => {
          const item = (await tx.doc(this.State, this.id)).admissions[requestId];
          if (item?.status === 'pending') { item.status = 'failed'; item.error = error.message; }
        }, this.context);
        throw error;
      }
      await this.mirror();
      const live = await this.harness.snapshot(this.pi.LiveDoc, this.id, this.context);
      if (!admission.steer || !live?.run) {
        const view = admission.view ?? await this.memory.readyView(this.summarize);
        if (this.closed || (await this.state()).admissions[requestId].status !== 'pending') return;
        const rendered = typeof view === 'string' ? view : view.rendered;
        if (typeof rendered !== 'string') throw new Error('Memory readiness did not return a rendered view');
        await this.conversation.commit(async tx => { const state = await tx.doc(this.State, this.id); state.view = rendered; state.admissions[requestId].view = rendered; }, this.context);
        await this.beginBrowserTask?.();
        await this.conversation.reset(undefined, this.context);
      }
      if (this.closed) return;
      if (!live?.run) await this.applyModelSelection();
      await this.memory.append({ key: `user:${requestId}`, kind: 'user', text: admission.text, date: admission.date, threadId: String(this.id) });
      this.scheduleCompaction();
      const submission = await this.conversation.submit({ type: 'input', content: admission.text, requestId, whenBusy: 'steer' }, this.context);
      await this.conversation.commit(async tx => { const item = (await tx.doc(this.State, this.id)).admissions[requestId]; item.status = 'submitted'; item.submissionId = submission.id; }, this.context);
      this.onEvent({ type: 'run', status: 'running', runId: submission.id, conversationId: this.id });
    } finally { this.preparing = false; }
  }
  async recoverAdmissions() {
    const state = await this.state();
    for (const [id, item] of Object.entries(state.admissions)) if (item.status === 'pending') {
      this.preparing = true; this.queue = this.queue.then(() => this.dispatch(id)).catch(error => this.onEvent({ type: 'run', status: 'error', error: error.message, conversationId: this.id }));
    }
  }
  async summarize(text, context = {}) {
    if (this.closed) throw new Error('Dojo runtime is closed');
    let rejectCancellation;
    const cancelled = new Promise((_resolve, reject) => { rejectCancellation = reject; });
    const job = { conversation: undefined, cancel: reason => { job.cancelled = true; rejectCancellation(new Error(reason)); return job.conversation?.abort(this.context); } };
    this.summaryJobs.add(job);
    const timer = setTimeout(() => { Promise.resolve(job.cancel('Pi summary timed out')).catch(() => {}); }, this.config.summaryTimeoutMs ?? 60000);
    try { return await Promise.race([this.runSummary(text, context, job), cancelled]); }
    finally { clearTimeout(timer); this.summaryJobs.delete(job); }
  }
  async runSummary(text, context, job) {
    const conversation = await this.harness.createConversation({ ownership: { kind: 'ownerless' }, agent: { ...this.agent, model: { provider: this.agent.model.provider, modelId: this.config.summaryModel ?? this.summaryModelId ?? this.agent.model.modelId }, thinkingLevel: this.agent.model.provider === 'anthropic' ? 'xhigh' : 'low' }, init: async (tx, id) => { const state = await tx.doc(this.State, id); state.mode = 'summary'; state.view = context.view ?? '<chat>\n</chat>'; } }, this.context);
    job.conversation = conversation;
    if (this.closed || job.cancelled) { await conversation.abort(this.context); throw new Error('Dojo summary cancelled'); }
    let prompt = summaryTask(text, context), shortest;
    for (let attempt = 0; attempt < 5; attempt++) {
      if (this.closed || job.cancelled) throw new Error('Dojo summary cancelled');
      const submission = await conversation.submit({ type: 'input', content: prompt, requestId: `summary:${attempt}` }, this.context);
      const settled = await submission.wait(this.context);
      if (settled.status !== 'done') throw new Error(`Pi summary failed: ${settled.reason ?? settled.status}${settled.detail ? ` (${settled.detail})` : ''}`);
      const entry = await conversation.commit(tx => tx.entry(this.pi.AssistantEntry, settled.answer), this.context);
      const candidate = textContent(entry?.model?.[0]?.content).trim();
      if (!candidate) throw new Error('Pi summary returned no text');
      if (shortest === undefined || Buffer.byteLength(candidate) < Buffer.byteLength(shortest)) shortest = candidate;
      const bytes = Buffer.byteLength(candidate);
      if (bytes <= 512) break;
      prompt = `Too long: your line is ${bytes} bytes, over the 512-byte limit. Write\nthe whole line again for the same <input>, cutting just enough of the\nleast valuable items to fit before this cut:\n${prefix512(candidate)}| ← LIMIT`;
    }
    return shortest;
  }
  resume() { this.harness.resume(); }
  async stop() {
    this.browserStopping = true;
    const browserCleanup = this.finishBrowserInput?.();
    this.releaseCua?.('Dojo runtime stopped');
    this.desktopStopping = true;
    this.releaseDesktop?.('Dojo runtime stopped'); // Release before waiting for Pi/storage.
    try {
      await browserCleanup;
      if (this.invalidateDesktop) await this.invalidateDesktop();
      await cancelHumanRequests(this);
      await this.cancelComputerSteps?.();
      await this.conversation.commit(async tx => {
        const state = await tx.doc(this.State, this.id);
        for (const item of Object.values(state.admissions)) if (item.status === 'pending') item.status = 'stopped';
      }, this.context);
      await this.conversation.abort(this.context);
    } finally { this.desktopStopping = false; this.browserStopping = false; }
  }
  abort() { return this.stop(); }
  async close() {
    if (this.closed) return;
    this.closed = true;
    await this.closeBrowser?.();
    this.releaseCua?.('Dojo runtime closed');
    this.releaseDesktop?.('Dojo runtime closed');
    if (this.invalidateDesktop) await this.invalidateDesktop();
    await this.watch.stop();
    await Promise.allSettled([...this.summaryJobs].map(job => job.cancel('Dojo runtime is closed')));
    await this.queue;
    await this.compaction;
    await this.mirror();
    await this.harness.close(this.context);

  }
}
