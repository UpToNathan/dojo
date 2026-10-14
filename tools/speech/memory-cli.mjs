import { join, isAbsolute } from 'node:path';
import { handle, memoryRequest } from './memory-mcp.mjs';

const args = process.argv.slice(2);
let socket = process.env.XDG_RUNTIME_DIR ? join(process.env.XDG_RUNTIME_DIR, 'dojo-voice.sock') : null;
if (args[0] === '--socket') { args.shift(); socket = args.shift(); }
try {
  if (!socket || !isAbsolute(socket)) throw new Error('An absolute Dojo voice socket is required.');
  const action = args.shift();
  let name, parameters;
  if (action === 'status' && !args.length) {
    console.log(JSON.stringify(await memoryRequest(socket, { action: 'memory-status' })));
  } else {
    if (action === 'zoom' && args.length >= 1 && args.length <= 3) { name = 'dojo_memory_zoom'; parameters = { id: Number(args[0]), count: Number(args[1] ?? 1), offset: Number(args[2] ?? 0) }; }
    else if (action === 'date' && args.length === 1) { name = 'dojo_memory_date'; parameters = { id: Number(args[0]) }; }
    else throw new Error('Use zoom ID [COUNT [OFFSET]], date ID, or status.');
    const result = await handle({ id: 1, method: 'tools/call', params: { name, arguments: parameters } }, socket);
    if (result.isError) throw new Error(result.content[0].text);
    console.log(result.content[0].text);
  }
} catch (error) { console.error(error.message); process.exitCode = 1; }
