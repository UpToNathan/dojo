import { randomUUID } from 'node:crypto';
import { DesktopMcp } from './desktop-extension.mjs';

const safe = new Set(['status', 'session_current', 'session_list', 'skill']);
const metadata = result => result.structuredContent ?? (result.content ?? []).reduce((value, block) => {
  if (block.type !== 'text') return value;
  try { return JSON.parse(block.text); } catch { return value; }
}, {});

export async function openBrowserExtension(runtime) {
  const { pi, config } = runtime;
  const owner = `dojo-pi:${randomUUID()}`;
  const maxAttempts = config.browserMaxToolAttempts ?? 1000;
  if (!Number.isSafeInteger(maxAttempts) || maxAttempts < 1 || maxAttempts > 100000) throw new Error('Invalid browser tool budget');
  const open = config.browserClientFactory ?? (() => DesktopMcp.open(runtime.root, {
    command: process.env.DOJO_BROWSER_CONTROL_MCP_EXE ?? 'browser-control-mcp', args: [], timeoutMs: config.browserTimeoutMs ?? 180000, killGraceMs: 4000,
    env: { ...process.env, DOJO_BROWSER_CONTROL_MODE: '1', DOJO_BROWSER_OWNER: owner },
  }));
  const lifecycle = ['execute', 'execute_cancel', 'session_resume'];
  const toolCatalog = client => {
    const tools = Array.isArray(client.tools) ? client.tools.filter(tool => tool.name !== 'session_new') : client.tools;
    if (!Array.isArray(tools) || tools.some(tool => typeof tool.name !== 'string' || !tool.inputSchema) || new Set(tools.map(tool => tool.name)).size !== tools.length || !lifecycle.every(name => tools.some(tool => tool.name === name))) {
      client.close(); throw new Error('Pinned Dojo Browser Control lifecycle tools are unavailable');
    }
    return tools;
  };
  let mcp = await open();
  runtime.browser = mcp;
  const catalog = toolCatalog(mcp);
  runtime.BrowserState = pi.defineDoc({ kind: 'dojo.browser', version: 1, scope: 'conversation', history: 'latest', fork: 'initial', initial: () => ({ uncertain: false, toolAttempts: 0 }) });
  // retirementPending: the last cleanup could not confirm the native lease was released, but no command was
  // running, so nothing about the browser is unknown. It is retried, never persisted. inflight counts commands
  // sent whose answer has not come back; only those can leave an outcome unknown.
  let active = false, stopping = false, retired = false, retirementPending = false, inflight = 0, resume, cancellation;
  runtime.beginBrowserTask = async () => { await runtime.finishBrowserInput(); retired = false; };
  const call = (name, args = {}, options) => mcp.call('tools/call', { name, arguments: args }, options);
  const saveUncertainty = async reason => {
    console.error(`Dojo browser: outcome uncertain, operator review required (${reason})`);
    if (runtime.conversation) await runtime.conversation.commit(async tx => {
      const state = await tx.doc(runtime.BrowserState, runtime.id);
      state.uncertain = true; state.reason = reason; state.at = new Date().toISOString();
    }, runtime.context);
  };
  // A closed client never recovers by itself; the next browser call starts a fresh one.
  const reconnect = async () => {
    if (!mcp.closed || runtime.closed) return;
    mcp = await open();
    runtime.browser = mcp;
    toolCatalog(mcp);
    // The old process released its lease when it shut down; a fresh one holds nothing to retire.
    active = false; resume = undefined; retirementPending = false;
  };
  // The operator's own word that they looked at the browser: the only way the flag goes away.
  // The agent has no tool for this.
  runtime.acknowledgeBrowser = async () => {
    if (!runtime.conversation) throw new Error('No conversation is active.');
    let wasUncertain = false, reason;
    await runtime.conversation.commit(async tx => {
      const state = await tx.doc(runtime.BrowserState, runtime.id);
      wasUncertain = state.uncertain === true;
      reason = state.reason;
      state.uncertain = false; delete state.reason; delete state.at;
    }, runtime.context);
    return wasUncertain ? { cleared: true, reason: reason ?? 'recorded before reasons were kept' } : { cleared: false };
  };
  runtime.browserStatus = async () => {
    const state = runtime.conversation ? await runtime.harness.snapshot(runtime.BrowserState, runtime.id, runtime.context) : undefined;
    return { uncertain: state?.uncertain === true, reason: state?.reason, at: state?.at, cleanupPending: retirementPending, connected: !mcp.closed };
  };
  runtime.finishBrowserInput = () => {
    if (cancellation) return cancellation;
    if (mcp.closed || (!active && !resume && !retirementPending)) return Promise.resolve();
    stopping = true; retired = true;
    cancellation = (async () => {
      try {
        await resume;
        const result = await call('execute_cancel');
        const receipt = metadata(result);
        const unknown = receipt.unknownCommandIds ?? [];
        const confirmed = receipt.nativeRetired === true;
        retirementPending = !confirmed;
        if (unknown.length > 0 || receipt.effect === 'unknown') {
          await saveUncertainty(`browser command${unknown.length === 1 ? '' : 's'} ${unknown.join(', ') || 'of unknown id'} had an unknown effect when the task ended`);
        } else if (!confirmed && inflight > 0) {
          await saveUncertainty('a browser command was running when the native lease could not be confirmed released');
        } else if (!confirmed) {
          console.error('Dojo browser cleanup: native release not confirmed; no command was running, so it will be retried');
        }
        return { ...receipt, isError: result.isError ?? !confirmed };
      } catch (error) {
        // The cleanup call itself failed. That is only an unknown outcome if a command could have been running.
        const running = inflight > 0;
        if (running) await saveUncertainty(`cleanup failed while a browser command was running: ${error.message}`);
        else retirementPending = true;
        mcp.close(error);
        console.error(`Dojo browser cleanup: ${error.message}`);
        return { nativeRetired: false, ...(running ? { effect: 'unknown' } : {}), isError: true, error: error.message };
      } finally { active = false; stopping = false; cancellation = undefined; }
    })();
    return cancellation;
  };
  runtime.closeBrowser = async () => {
    await runtime.finishBrowserInput();
    mcp.close(); runtime.browser = undefined;
  };
  const ensureActive = async (api, context) => {
    if (runtime.closed || runtime.browserStopping || stopping || retired) throw new Error('Browser task is stopped');
    const state = await api.snapshot(runtime.BrowserState, api.conversationId, context);
    if (state?.uncertain) throw new Error(`An earlier browser command has an uncertain outcome${state.reason ? ` (${state.reason})` : ''}. Tell the operator to check the browser, then run \`dojo-voice acknowledge-browser\` to clear it; do not retry or switch to desktop input.`);
    await reconnect();
    if (mcp.closed) throw new Error('Browser task is stopped');
    if (!active) {
      resume ??= (async () => {
        const result = await call('session_resume');
        if (result.isError) throw new Error(`Browser session resume refused: ${JSON.stringify(result.content)}`);
        active = true;
      })();
      try { await resume; } finally { resume = undefined; }
    }
    if (stopping || runtime.browserStopping || runtime.closed) throw new Error('Browser task is stopped');
  };
  return pi.defineExtension({ name: 'dojo-browser-control', tools: catalog.map(tool => pi.defineTool({
    name: `browser_control_${tool.name}`, description: tool.description ?? tool.name, parameters: tool.inputSchema,
    replay: safe.has(tool.name) ? 'safe' : 'unsafe', executionMode: 'sequential',
    execute: async (args, api, context) => {
      const signal = context.abortSignal;
      if (signal?.aborted) throw new Error('Browser task is stopped');
      if (tool.name === 'execute_cancel') {
        const receipt = await runtime.finishBrowserInput() ?? { nativeRetired: true, unknownCommandIds: [], active: false };
        return { output: [{ type: 'text', text: JSON.stringify(receipt) }], isError: receipt.isError ?? false };
      }
      // A task gets one generation. Models cannot resume a retired task or select another client's session.
      if (args.session !== undefined || tool.name === 'session_new') throw new Error('Pi owns one browser session per agent; use its current session');
      if (!safe.has(tool.name)) await ensureActive(api, context);
      if (tool.name === 'session_resume') return { output: [{ type: 'text', text: 'Current task browser generation is active.' }] };
      await api.commit(async tx => {
        const state = await tx.doc(runtime.BrowserState, api.conversationId);
        if (!Number.isSafeInteger(state.toolAttempts) || state.toolAttempts < 0) throw new Error('Invalid browser tool budget state');
        if (state.toolAttempts >= maxAttempts) throw new Error('Browser tool budget exhausted');
        state.toolAttempts++;
      }, context);
      const requestAbort = new AbortController();
      const abort = () => { void runtime.finishBrowserInput().then(() => requestAbort.abort(), error => { mcp.close(error); requestAbort.abort(); }); };
      signal?.addEventListener('abort', abort, { once: true });
      const counted = !safe.has(tool.name);
      if (counted) inflight++;
      try {
        if (signal?.aborted) { abort(); throw new Error('Browser task is stopped'); }
        const result = await call(tool.name, args, { signal: requestAbort.signal, closeOnAbort: false });
        const receipt = metadata(result);
        if (receipt.effect === 'unknown' || (receipt.unknownCommandIds?.length ?? 0) > 0) await saveUncertainty(`${tool.name} reported an unknown effect`);
        // Status only reads the relay, which starts on the first session_resume. A stopped relay is the normal
        // idle state, not a failure; reported as an error it sent the model to desktop input instead.
        if (tool.name === 'status' && result.isError && /relay is not reachable/i.test(JSON.stringify(result.content))) {
          return { output: [{ type: 'text', text: 'The Browser Control relay is not running yet. It starts with the first browser_control_execute; if that reports the extension is not connected, the browser is still starting, so wait a few seconds and try again.' }], isError: false };
        }
        return { output: result.content, isError: result.isError ?? false };
      } catch (error) {
        if (!signal?.aborted && !safe.has(tool.name)) await saveUncertainty(`${tool.name} failed without an answer: ${error.message}`);
        throw error;
      } finally { if (counted) inflight--; signal?.removeEventListener('abort', abort); }
    },
  })) });
}
