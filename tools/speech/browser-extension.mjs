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
  const mcp = await open();
  runtime.browser = mcp;
  const catalog = Array.isArray(mcp.tools) ? mcp.tools.filter(tool => tool.name !== 'session_new') : mcp.tools;
  if (!Array.isArray(catalog) || catalog.some(tool => typeof tool.name !== 'string' || !tool.inputSchema) || new Set(catalog.map(tool => tool.name)).size !== catalog.length || !['execute', 'execute_cancel', 'session_resume'].every(name => catalog.some(tool => tool.name === name))) {
    mcp.close(); throw new Error('Pinned Dojo Browser Control lifecycle tools are unavailable');
  }
  runtime.BrowserState = pi.defineDoc({ kind: 'dojo.browser', version: 1, scope: 'conversation', history: 'latest', fork: 'initial', initial: () => ({ uncertain: false, toolAttempts: 0 }) });
  let active = false, stopping = false, retired = false, resume, cancellation;
  runtime.beginBrowserTask = async () => { await runtime.finishBrowserInput(); retired = false; };
  const call = (name, args = {}, options) => mcp.call('tools/call', { name, arguments: args }, options);
  const saveUncertainty = async () => {
    if (runtime.conversation) await runtime.conversation.commit(async tx => {
      (await tx.doc(runtime.BrowserState, runtime.id)).uncertain = true;
    }, runtime.context);
  };
  runtime.finishBrowserInput = () => {
    if (cancellation) return cancellation;
    if (!active && !resume) return Promise.resolve();
    stopping = true; retired = true;
    cancellation = (async () => {
      try {
        await resume;
        const result = await call('execute_cancel');
        const receipt = metadata(result);
        if (result.isError || receipt.nativeRetired !== true || (receipt.unknownCommandIds?.length ?? 0) > 0) {
          await saveUncertainty();
          console.error('Dojo browser cleanup: command outcome is uncertain; browser continuation requires operator review');
        }
        return { ...receipt, isError: result.isError ?? receipt.nativeRetired !== true };
      } catch (error) {
        await saveUncertainty();
        mcp.close(error);
        console.error(`Dojo browser cleanup: ${error.message}`);
        return { nativeRetired: false, effect: 'unknown', isError: true, error: error.message };
      } finally { active = false; stopping = false; cancellation = undefined; }
    })();
    return cancellation;
  };
  runtime.closeBrowser = async () => {
    await runtime.finishBrowserInput();
    mcp.close(); runtime.browser = undefined;
  };
  const ensureActive = async (api, context) => {
    if (runtime.closed || runtime.browserStopping || stopping || retired || mcp.closed) throw new Error('Browser task is stopped');
    const state = await api.snapshot(runtime.BrowserState, api.conversationId, context);
    if (state?.uncertain) throw new Error('An earlier browser command has an uncertain outcome. Ask the operator to review it; do not retry or switch to desktop input.');
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
      try {
        if (signal?.aborted) { abort(); throw new Error('Browser task is stopped'); }
        const result = await call(tool.name, args, { signal: requestAbort.signal, closeOnAbort: false });
        const receipt = metadata(result);
        if (receipt.effect === 'unknown' || (receipt.unknownCommandIds?.length ?? 0) > 0) await saveUncertainty();
        // Status only reads the relay, which starts on the first session_resume. A stopped relay is the normal
        // idle state, not a failure; reported as an error it sent the model to desktop input instead.
        if (tool.name === 'status' && result.isError && /relay is not reachable/i.test(JSON.stringify(result.content))) {
          return { output: [{ type: 'text', text: 'The Browser Control relay is not running yet. It starts with the first browser_control_execute; if that reports the extension is not connected, the browser is still starting, so wait a few seconds and try again.' }], isError: false };
        }
        return { output: result.content, isError: result.isError ?? false };
      } catch (error) {
        if (!signal?.aborted && !safe.has(tool.name)) await saveUncertainty();
        throw error;
      } finally { signal?.removeEventListener('abort', abort); }
    },
  })) });
}
