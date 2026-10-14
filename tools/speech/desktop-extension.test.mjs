import test from 'node:test';
import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { PassThrough } from 'node:stream';
import { DesktopMcp, desktopReplay, DESKTOP_MCP_PROTOCOL_VERSION } from './desktop-extension.mjs';

test('desktop connection pins MCP protocol and refuses a negotiated downgrade', async t => {
  const make = protocolVersion => {
    const child = new EventEmitter(); child.stdin = new PassThrough(); child.stdout = new PassThrough();
    child.kills = []; child.kill = signal => { child.kills.push(signal); return true; };
    const requests = [];
    child.stdin.on('data', chunk => {
      for (const line of chunk.toString().trim().split('\n')) {
        const request = JSON.parse(line); requests.push(request);
        if (request.id !== undefined) child.stdout.write(JSON.stringify({ jsonrpc: '2.0', id: request.id, result: request.method === 'initialize' ? { protocolVersion } : { tools: [] } }) + '\n');
      }
    });
    return { child, requests };
  };
  const accepted = make(DESKTOP_MCP_PROTOCOL_VERSION);
  const client = await DesktopMcp.open('/fixture', { spawn: () => accepted.child, timeoutMs: 100, killGraceMs: 1 });
  t.after(() => client.close());
  assert.equal(accepted.requests[0].params.protocolVersion, '2025-06-18');
  const downgraded = make('2024-11-05');
  await assert.rejects(DesktopMcp.open('/fixture', { spawn: () => downgraded.child, timeoutMs: 100, killGraceMs: 1 }), /protocol version mismatch/);
  assert.equal(downgraded.requests.some(request => request.method === 'tools/list'), false);
  assert.deepEqual(downgraded.child.kills, ['SIGTERM']);
});

function transport(t, options = {}) {
  const child = new EventEmitter(); child.stdin = new PassThrough(); child.stdout = new PassThrough();
  child.kills = []; child.kill = signal => { child.kills.push(signal); return true; };
  const client = new DesktopMcp(child, { timeoutMs: 100, killGraceMs: 5, ...options });
  t.after(() => { client.close(); child.emit('exit', 0); });
  return { child, client };
}

test('explicit replay allowlist ignores mutations and unknown future tools', () => {
  for (const name of ['list_windows', 'get_screen_size', 'list_apps', 'parse_visual_regions', 'desktop_context', 'desktop_screenshot', 'desktop_observe', 'desktop_verify', 'desktop_decode_model', 'desktop_accessibility']) assert.equal(desktopReplay(name), 'safe');
  for (const name of ['desktop_action', 'desktop_capture_action', 'desktop_launch', 'desktop_release', 'desktop_feedback', 'future_read']) assert.equal(desktopReplay(name), 'unsafe');
});

test('MCP timeout rejects every pending promise, ignores late replies and escalates termination', async t => {
  const { client, child } = transport(t, { timeoutMs: 10 });
  const first = assert.rejects(client.call('tools/call', {}), /timed out/);
  const second = assert.rejects(client.call('tools/list', {}), /timed out/);
  await Promise.all([first, second]);
  assert.equal(client.pending.size, 0); assert.equal(client.closed, true);
  child.stdout.write('{"id":1,"result":{}}\n');
  await new Promise(resolve => setTimeout(resolve, 15));
  assert.deepEqual(child.kills, ['SIGTERM', 'SIGKILL']);
  await assert.rejects(client.call('tools/list', {}), /closed/);
});

for (const event of ['exit', 'close', 'error', 'stdout', 'stdin']) test(`MCP ${event} settles outstanding requests`, async t => {
  const { client, child } = transport(t);
  const rejection = assert.rejects(client.call('tools/call', {}));
  if (event === 'stdout') child.stdout.end();
  else if (event === 'stdin') child.stdin.emit('error', new Error('broken pipe'));
  else child.emit(event, event === 'error' ? new Error('spawn failure') : 1);
  await rejection; assert.equal(client.pending.size, 0); assert.equal(client.closed, true);
});

test('MCP abort/explicit close reject promptly rather than queueing release', async t => {
  const { client, child } = transport(t);
  const controller = new AbortController();
  const rejection = assert.rejects(client.call('tools/call', {}, { signal: controller.signal }), /aborted/);
  controller.abort(); await rejection;
  assert.deepEqual(child.kills, ['SIGTERM']);
  const next = transport(t);
  const closed = assert.rejects(next.client.call('tools/call', {}), /closed/);
  next.client.close(); await closed;
});

test('MCP response resolves once and removes timers/pending entries', async t => {
  const { client, child } = transport(t);
  const result = client.call('tools/call', {});
  child.stdout.write('{"id":1,"result":{"content":[]}}\n');
  assert.deepEqual(await result, { content: [] }); assert.equal(client.pending.size, 0);
  child.stdout.write('{"id":1,"error":{"message":"late"}}\n');
  assert.equal(client.closed, false);
});

test('browser request abort cancels one RPC after retirement without closing other requests', async t => {
  const { client, child } = transport(t);
  const controller = new AbortController();
  const first = assert.rejects(client.call('tools/call', {}, { signal: controller.signal, closeOnAbort: false }), /request aborted/);
  const next = client.call('tools/call', {});
  controller.abort(); await first;
  assert.equal(client.closed, false);
  assert.deepEqual(child.kills, []);
  assert.equal(client.pending.size, 1);
  child.stdout.write('{"id":2,"result":{"content":[]}}\n');
  assert.deepEqual(await next, { content: [] });
  child.stdout.write('{"id":1,"result":{"content":[]}}\n');
  assert.equal(client.closed, false);
});
