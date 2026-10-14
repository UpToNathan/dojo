import {mkdir} from 'node:fs/promises';
import {readFileSync} from 'node:fs';
import {isDeepStrictEqual} from 'node:util';
import {join} from 'node:path';
import {DesktopMcp} from './desktop-extension.mjs';

const pinnedCatalog = JSON.parse(readFileSync(new URL('../../vendor/cua/metadata/pi-resource-catalog.json', import.meta.url)));
const resourceSchemas = new Map(pinnedCatalog.tools.map(tool => [tool.name, tool.inputSchema]));
// Forward the pinned resource contract. Pi remains the task and model owner.
export const cuaResourceTool = name => resourceSchemas.has(name);

export async function openCuaExtension(runtime) {
  const {pi, config} = runtime;
  const settings = config.cuaTools && typeof config.cuaTools === 'object' ? config.cuaTools : {};
  const args = ['--embedded', 'mcp'];
  for (const [key, flag] of [['sandbox', '--sandbox'], ['permissions', '--permissions']]) {
    if (settings[key] !== undefined) {
      if (typeof settings[key] !== 'string' || !settings[key].trim() || settings[key].includes('\0')) throw new Error(`Invalid CUA ${key}`);
      args.push(flag, settings[key]);
    }
  }
  const home = join(runtime.stateDir, 'cua');
  await mkdir(home, {recursive: true, mode: 0o700});
  const env = {...process.env, CUA_HOME: home, DO_NOT_TRACK: '1'};
  for (const key of ['OPENAI_API_KEY', 'ANTHROPIC_API_KEY', 'GOOGLE_API_KEY', 'GEMINI_API_KEY']) delete env[key];
  const open = config.cuaClientFactory ?? (() => DesktopMcp.open(runtime.root, {
    command: config.cuaBinary ?? join(runtime.root, 'vendor/cua/upstream/libs/cua/target/debug/cua'),
    args, env, timeoutMs: config.cuaTimeoutMs ?? 30000,
  }));
  runtime.cua = await open();
  let generation = 0;
  runtime.releaseCua = (reason = 'Dojo CUA session interrupted') => {
    generation++;
    runtime.cua?.close(new Error(reason));
    runtime.cua = undefined;
  };
  const catalog = runtime.cua.tools;
  if (!Array.isArray(catalog) || catalog.some(tool => !/^[A-Za-z0-9_-]{1,60}$/.test(tool.name) || !tool.inputSchema)
      || new Set(catalog.map(tool => tool.name)).size !== catalog.length) {
    runtime.releaseCua('Invalid CUA tool catalog');
    throw new Error('Invalid CUA tool catalog');
  }
  if (!config.cuaClientFactory && pinnedCatalog.tools.some(expected => {
    const actual = catalog.find(tool => tool.name === expected.name);
    return !actual || !isDeepStrictEqual(actual.inputSchema, expected.inputSchema);
  })) {
    runtime.releaseCua('CUA resource contract changed');
    throw new Error('CUA resource contract changed; review the pinned catalog before reconnecting');
  }
  const selected = catalog.filter(tool => cuaResourceTool(tool.name));
  const schemaMatches = tools => selected.every(expected => {
    const actual = tools?.find(tool => tool.name === expected.name);
    return actual && isDeepStrictEqual(actual.inputSchema, expected.inputSchema);
  });
  const clientFor = async signal => {
    if (runtime.closed || runtime.desktopStopping || signal?.aborted) throw new Error('CUA connection retired');
    if (!runtime.cua || runtime.cua.closed) {
      const started = generation;
      const replacement = await open();
      if (replacement.closed || generation !== started || runtime.closed || runtime.desktopStopping || signal?.aborted
          || !schemaMatches(replacement.tools)) {
        replacement.close();
        throw new Error('CUA connection retired or resource contract changed');
      }
      runtime.cua = replacement;
    }
    return runtime.cua;
  };
  return pi.defineExtension({name: 'dojo-cua', tools: selected.map(tool => pi.defineTool({
    name: `cua_${tool.name}`, description: tool.description ?? tool.name,
    parameters: tool.inputSchema, replay: 'unsafe', executionMode: 'sequential',
    execute: async (args, _api, context) => {
      const client = await clientFor(context.abortSignal);
      const result = await client.call('tools/call', {name: tool.name, arguments: args}, {signal: context.abortSignal});
      return {content: result.content, isError: result.isError ?? false};
    },
  }))});
}
