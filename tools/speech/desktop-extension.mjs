import { join } from 'node:path';
import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';

export const DESKTOP_MCP_PROTOCOL_VERSION = '2025-06-18';

// Deliberate allowlist: annotations/idempotence do not make desktop mutations replay-safe.
export const desktopReplay = name => ['list_windows', 'get_screen_size', 'list_apps', 'parse_visual_regions', 'desktop_context', 'desktop_screenshot', 'desktop_observe', 'desktop_verify', 'desktop_decode_model', 'desktop_accessibility'].includes(name) ? 'safe' : 'unsafe';
const cleanupTools = new Set(['desktop_release']);
const metadata = result => {
  if (result.structuredContent) return result.structuredContent;
  for (const block of result.content ?? []) if (block.type === 'text') {
    try { const value = JSON.parse(block.text); if (value && typeof value === 'object') return value; } catch {}
  }
  return {};
};

export class DesktopMcp {
  static async open(root, options = {}) {
    const environment = options.env ?? process.env;
    const child = (options.spawn ?? spawn)(options.command ?? environment.DOJO_SEMANTIC_AGENT_EXE ?? join(root, 'target/debug/dojo-agent'), options.args ?? ['--name', 'dojo-voice'], { cwd: root, env: environment, stdio: ['pipe', 'pipe', 'inherit'] });
    const client = new DesktopMcp(child, options);
    try {
      const initialized = await client.call('initialize', { protocolVersion: DESKTOP_MCP_PROTOCOL_VERSION, capabilities: {}, clientInfo: { name: 'dojo-pi', version: '1' } });
      if (initialized.protocolVersion !== DESKTOP_MCP_PROTOCOL_VERSION) throw new Error('Dojo desktop MCP protocol version mismatch');
      child.stdin.write(JSON.stringify({ jsonrpc: '2.0', method: 'notifications/initialized' }) + '\n');
      client.tools = (await client.call('tools/list', {})).tools;
      if (!Array.isArray(client.tools)) throw new Error('Dojo desktop MCP returned no tool catalog');
      return client;
    } catch (error) { client.close(error); throw error; }
  }
  constructor(child, { timeoutMs = 30000, killGraceMs = 250 } = {}) {
    if (!Number.isFinite(timeoutMs) || timeoutMs <= 0 || !Number.isFinite(killGraceMs) || killGraceMs < 0) throw new Error('Invalid desktop MCP deadlines');
    Object.assign(this, { child, timeoutMs, killGraceMs, pending: new Map(), next: 0, closed: false });
    const fail = error => this.close(error);
    child.on('error', fail);
    child.stdin.on('error', fail);
    child.stdout.on('error', fail);
    child.on('exit', (code, signal) => { this.exited = true; clearTimeout(this.killTimer); fail(new Error(`Dojo desktop MCP exited (${code ?? signal})`)); });
    child.on('close', () => { this.exited = true; clearTimeout(this.killTimer); fail(new Error('Dojo desktop MCP closed')); });
    this.lines = createInterface({ input: child.stdout });
    this.lines.on('close', () => fail(new Error('Dojo desktop MCP stdout closed')));
    this.lines.on('line', line => {
      let response; try { response = JSON.parse(line); } catch { return; }
      const request = this.pending.get(response.id); if (!request) return; // Ignore late/duplicate responses.
      if (response.error) request.reject(new Error(response.error.message ?? 'Desktop MCP error')); else request.resolve(response.result);
    });
  }
  call(method, params, { signal, closeOnAbort = true } = {}) {
    if (this.closed) return Promise.reject(new Error('Dojo desktop MCP is closed'));
    if (signal?.aborted) { if (closeOnAbort) this.close(new Error('Dojo desktop MCP aborted')); return Promise.reject(new Error('Dojo desktop MCP aborted')); }
    const id = ++this.next;
    return new Promise((resolve, reject) => {
      const finish = (fn, value) => {
        if (!this.pending.delete(id)) return;
        clearTimeout(timer); signal?.removeEventListener('abort', abort); fn(value);
      };
      const abort = () => {
        const error = new Error('Dojo MCP request aborted');
        if (closeOnAbort) this.close(error);
        else {
          finish(reject, error);
          try { this.child.stdin.write(JSON.stringify({ jsonrpc: '2.0', method: 'notifications/cancelled', params: { requestId: id, reason: 'Dojo task cancelled' } }) + '\n'); } catch (failure) { this.close(failure); }
        }
      };
      const timer = setTimeout(() => this.close(new Error(`Dojo desktop MCP ${method} timed out`)), this.timeoutMs);
      this.pending.set(id, { resolve: value => finish(resolve, value), reject: error => finish(reject, error) });
      signal?.addEventListener('abort', abort, { once: true });
      try {
        this.child.stdin.write(JSON.stringify({ jsonrpc: '2.0', id, method, params }) + '\n', error => { if (error) this.close(error); });
      } catch (error) { this.close(error); }
    });
  }
  close(reason = new Error('Dojo desktop MCP is closed')) {
    if (this.closed) return;
    this.closed = true;
    for (const request of [...this.pending.values()]) request.reject(reason);
    this.lines.close();
    // A tools/call release on this connection could queue behind blocked Rust IPC.
    // Disconnect instead; Rust owns disconnect cleanup. Escalate even if SIGTERM is ignored.
    if (!this.exited) {
      this.child.kill('SIGTERM');
      this.killTimer = setTimeout(() => { if (!this.exited) this.child.kill('SIGKILL'); }, this.killGraceMs);
      this.killTimer.unref?.();
    }
  }
}

// Some models emit object or array arguments as JSON strings ("{\"kind\":\"invoke\"}"). The tool catalog is strict,
// so those calls fail validation and the model falls back to unverified low-level input. Repair only top-level
// properties whose schema permits an object or array but not a string, and only when the text parses to that type.
const schemaTypes = (schema, root, seen = new Set()) => {
  if (!schema || typeof schema !== 'object' || seen.has(schema)) return new Set();
  seen.add(schema);
  if (typeof schema.$ref === 'string' && schema.$ref.startsWith('#/')) {
    const target = schema.$ref.slice(2).split('/').reduce((node, key) => node?.[key], root);
    return schemaTypes(target, root, seen);
  }
  const types = new Set(Array.isArray(schema.type) ? schema.type : schema.type ? [schema.type] : []);
  if (schema.properties) types.add('object');
  if (schema.items) types.add('array');
  for (const branch of [...(schema.oneOf ?? []), ...(schema.anyOf ?? []), ...(schema.allOf ?? [])]) for (const type of schemaTypes(branch, root, seen)) types.add(type);
  return types;
};
export function repairStructuredArguments(schema, args) {
  if (!args || typeof args !== 'object' || Array.isArray(args)) return args;
  let repaired;
  for (const [key, value] of Object.entries(args)) {
    if (typeof value !== 'string') continue;
    const types = schemaTypes(schema?.properties?.[key], schema);
    if (types.has('string') || types.size === 0) continue;
    const text = value.trim();
    let parsed;
    try { parsed = JSON.parse(text); } catch { continue; }
    const kind = Array.isArray(parsed) ? 'array' : parsed !== null && typeof parsed === 'object' ? 'object' : undefined;
    if (kind && types.has(kind)) (repaired ??= { ...args })[key] = parsed;
  }
  return repaired ?? args;
}

export async function openDesktopExtension(runtime) {
  const { pi, config } = runtime;
  runtime.DesktopState = pi.defineDoc({ kind: 'dojo.desktop', version: 1, scope: 'conversation', history: 'latest', fork: 'initial', initial: () => ({ recoveryRequired: true, captureId: null, catalog: null, toolAttempts: 0, externalUnknown: false, semanticOperation: null }) });
  const open = config.desktopClientFactory ?? (() => DesktopMcp.open(runtime.root, { timeoutMs: config.desktopTimeoutMs ?? 30000 }));
  runtime.desktop = await open();
  const expandCatalog = async mcp => {
    const tools = mcp.tools;
    if (!tools.some(tool => tool.name === 'desktop_driver_tools')) return tools;
    const result = await mcp.call('tools/call', { name: 'desktop_driver_tools', arguments: {} });
    if (result.isError) throw new Error(`Dojo Driver catalog unavailable: ${(result.content ?? []).map(block => block.text).filter(Boolean).join(' ').slice(0, 400)}`);
    const value = metadata(result);
    const driverTools = Array.isArray(value) ? value : value.tools;
    if (!Array.isArray(driverTools)) throw new Error('Invalid Dojo Driver catalog');
    const names = new Set(tools.map(tool => tool.name));
    return [...tools.filter(tool => !['desktop_driver_tools', 'desktop_driver_call'].includes(tool.name)), ...driverTools.map(tool => {
      const name = `driver_${tool.name}`;
      if (names.has(name) || typeof tool.name !== 'string' || !tool.inputSchema) throw new Error('Invalid or duplicate Driver tool');
      names.add(name);
      return { ...tool, name, driverName: tool.name };
    })];
  };
  let catalog;
  try { catalog = await expandCatalog(runtime.desktop); }
  catch (error) { runtime.desktop.close(error); runtime.desktop = undefined; throw error; }
  // Pin registration/schema for this runtime. Reconnection may not silently reinterpret it.
  const fingerprint = tools => JSON.stringify(tools.map(({ name, inputSchema }) => ({ name, inputSchema })).sort((a, b) => a.name.localeCompare(b.name)));
  const expected = fingerprint(catalog);
  const maxAttempts = config.desktopMaxToolAttempts ?? 1000;
  if (!Number.isSafeInteger(maxAttempts) || maxAttempts < 1 || maxAttempts > 100000) throw new Error('Invalid desktop tool budget');
  runtime.releaseDesktop = (reason = 'Dojo desktop session interrupted') => { runtime.desktop?.close(new Error(reason)); runtime.desktop = undefined; };
  runtime.invalidateDesktop = () => runtime.conversation.commit(async tx => {
    const state = await tx.doc(runtime.DesktopState, runtime.id); state.recoveryRequired = true; state.captureId = null; state.catalog = expected;
  }, runtime.context);
  runtime.finishDesktopInput = async () => {
    const mcp = runtime.desktop;
    if (!mcp || mcp.closed) return;
    try {
      await runtime.invalidateDesktop();
      const result = await mcp.call('tools/call', { name: 'desktop_action', arguments: { action: 'reset' } });
      if (result.isError) throw new Error('Desktop input reset refused');
    } catch (error) {
      console.error(`Dojo cursor cleanup: ${error.message}`);
      mcp.close(error);
      if (runtime.desktop === mcp) runtime.desktop = undefined;
    }
  };
  const client = async signal => {
    if (runtime.closed || runtime.desktopStopping || signal?.aborted) throw new Error('Dojo desktop session cancelled');
    if (!runtime.desktop || runtime.desktop.closed) {
      await runtime.invalidateDesktop();
      const replacement = await open();
      if (runtime.closed || runtime.desktopStopping || signal?.aborted || fingerprint(await expandCatalog(replacement).catch(error => { replacement.close(error); throw error; })) !== expected) {
        replacement.close(); throw new Error('Dojo desktop session cancelled or tool catalog changed; reopen runtime');
      }
      runtime.desktop = replacement;
    }
    return runtime.desktop;
  };
  const names = new Set(catalog.map(tool => tool.name));
  const driverNames = new Map(catalog.filter(tool => tool.driverName).map(tool => [tool.name, tool.driverName]));
  runtime.executeDesktopTool = async (name, args, api, context, abortSignal = context.abortSignal) => {
    if (!names.has(name)) throw new Error('Unknown desktop tool');
      const signal = abortSignal;
      const abort = () => runtime.releaseDesktop('Dojo desktop tool aborted');
      signal?.addEventListener('abort', abort, { once: true });
      try {
        const mcp = await client(signal);
        // Persist reservations before transport. Failed/uncertain attempts consume budget.
        await api.commit(async tx => {
          const state = await tx.doc(runtime.DesktopState, api.conversationId);
          if (state.catalog !== expected) {
            state.recoveryRequired = true; state.captureId = null; state.catalog = expected;
          }
          if (runtime.BrowserState && desktopReplay(name) === 'unsafe' && !cleanupTools.has(name) && !(name === 'desktop_action' && args.action === 'reset')) {
            const browser = await tx.doc(runtime.BrowserState, api.conversationId);
            if (browser.uncertain) throw new Error('Browser outcome is unresolved; desktop input cannot bypass browser recovery');
          }
          if (state.externalUnknown && desktopReplay(name) === 'unsafe' && !cleanupTools.has(name) && name !== 'desktop_feedback') throw new Error('External desktop effect is unresolved; mutations are blocked pending operator recovery');
          const attempts = state.toolAttempts ?? 0;
          if (!Number.isSafeInteger(attempts) || attempts < 0) throw new Error('Invalid desktop tool budget state');
          if (attempts >= maxAttempts) {
            if (!cleanupTools.has(name)) throw new Error('Desktop tool budget exhausted');
          } else state.toolAttempts = attempts + 1;
        }, context);
        if (name === 'desktop_capture_action') {
          // Commit consumption BEFORE sending effects. A crash/unknown receipt cannot restore the ID.
          await api.commit(async tx => {
            const state = await tx.doc(runtime.DesktopState, api.conversationId);
            if (state.recoveryRequired || !state.captureId || args.capture_id !== state.captureId) throw new Error('Desktop recovery required: call desktop_observe for a new capture before desktop_capture_action');
            state.recoveryRequired = true; state.captureId = null;
          }, context);
        } else if (desktopReplay(name) === 'unsafe') {
          await api.commit(async tx => {
            const state = await tx.doc(runtime.DesktopState, api.conversationId); state.recoveryRequired = true; state.captureId = null;
          }, context);
        }
        if (name === 'desktop_accessibility_action') await api.commit(async tx => {
          const state = await tx.doc(runtime.DesktopState, api.conversationId);
          state.externalUnknown = true; state.semanticOperation = 'in_flight';
        }, context);
        const result = await mcp.call('tools/call', { name: driverNames.has(name) ? 'desktop_driver_call' : name, arguments: driverNames.has(name) ? { name: driverNames.get(name), arguments: args } : args }, { signal });
        if (name === 'desktop_accessibility_action' && !result.isError) {
          const receipt = metadata(result);
          await api.commit(async tx => {
            const state = await tx.doc(runtime.DesktopState, api.conversationId);
            if (['acknowledged', 'rejected', 'refused'].includes(receipt.effect)
                || (receipt.effect === 'unknown' && ['acknowledged', 'rejected'].includes(receipt.late_terminal))) {
              state.externalUnknown = false; state.semanticOperation = null;
            } else if (receipt.effect === 'unknown' && typeof receipt.operation_id === 'string') {
              state.semanticOperation = receipt.operation_id;
            }
          }, context);
        }
        if (name === 'desktop_accessibility' && !result.isError) {
          const receipts = metadata(result).semantic_receipts;
          if (Array.isArray(receipts)) await api.commit(async tx => {
            const state = await tx.doc(runtime.DesktopState, api.conversationId);
            if (receipts.some(receipt => receipt.operation_id === state.semanticOperation
                && ['acknowledged', 'rejected'].includes(receipt.late_terminal))) {
              state.externalUnknown = false; state.semanticOperation = null;
            }
          }, context);
        }
        if ((name === 'desktop_observe' || name === 'desktop_capture_action') && !result.isError) {
          const value = metadata(result);
          const captureId = name === 'desktop_observe' ? value.capture_id : value.post_observation?.capture_id;
          if (typeof captureId === 'string' && captureId.length > 0) await api.commit(async tx => {
            const state = await tx.doc(runtime.DesktopState, api.conversationId); state.captureId = captureId; state.recoveryRequired = false;
          }, context);
        }
        if (!['desktop_observe', 'desktop_capture_action'].includes(name) && result.content?.some(block => block.type === 'image')) {
          // A page/compatibility screenshot is not the PNG bound to captureId.
          await api.commit(async tx => {
            const state = await tx.doc(runtime.DesktopState, api.conversationId);
            state.recoveryRequired = true; state.captureId = null;
          }, context);
        }
        // handled != verified success; paused may consume admission. Every action attempt
        // keeps recoveryRequired, regardless of receipt status. Rust owns effect classification.
        // Images pass through unchanged; no canonical transcript/memory retention changes.
        return { content: result.content, isError: result.isError ?? false };
      } finally { signal?.removeEventListener('abort', abort); }
  };
  runtime.decodeDesktopModel = (args, signal) => runtime.executeDesktopTool('desktop_decode_model', args, {
    conversationId: runtime.id,
    commit: (fn, context) => runtime.conversation.commit(fn, context),
  }, runtime.context, signal);
  return pi.defineExtension({ name: 'dojo-desktop', tools: catalog.map(tool => pi.defineTool({
    name: tool.name, description: tool.description ?? tool.name, parameters: tool.inputSchema,
    replay: desktopReplay(tool.name), executionMode: 'sequential',
    prepareArguments: args => repairStructuredArguments(tool.inputSchema, args),
    execute: async (args, api, context) => {
      const result = await runtime.executeDesktopTool(tool.name, args, api, context);
      return { output: result.content, isError: result.isError };
    },
  })) });
}
