import test from 'node:test';
import assert from 'node:assert/strict';
import { handle } from './memory-mcp.mjs';

test('memory MCP only exposes read-only Dojo retrieval', async () => {
  assert.deepEqual((await handle({ id: 1, method: 'tools/list' })).tools.map(tool => tool.name), ['dojo_memory_zoom', 'dojo_memory_date']);
  const commands = [];
  const result = await handle({ id: 2, method: 'tools/call', params: { name: 'dojo_memory_zoom', arguments: { id: 4, count: 2 } } }, '/private/dojo.sock', async (socket, value) => { commands.push([socket, value]); return { children: [{ id: 4, count: 1 }, { id: 5, count: 1 }] }; });
  assert.deepEqual(commands, [['/private/dojo.sock', { action: 'memory-zoom', id: 4, count: 2 }]]);
  assert.deepEqual(JSON.parse(result.content[0].text), { children: [{ id: 4, count: 1 }, { id: 5, count: 1 }] });
});
test('invalid or write-like requests never reach the memory owner', async () => {
  let calls = 0;
  for (const [name, args] of [['dojo_memory_write', {}], ['dojo_memory_zoom', { id: -1 }], ['dojo_memory_zoom', { id: 0, count: 3 }], ['dojo_memory_zoom', { id: 0, path: '/other/history' }], ['dojo_memory_search', { query: 'fact', limit: 9 }]]) {
    const result = await handle({ id: 1, method: 'tools/call', params: { name, arguments: args } }, '/private/dojo.sock', async () => { calls++; });
    assert.equal(result.isError, true);
  }
  assert.equal(calls, 0);
});
