import { execFile } from 'node:child_process';
import { access, constants } from 'node:fs/promises';
import { homedir } from 'node:os';
import { join } from 'node:path';

// The agent reaches other Dojo nodes only through `dojo-mesh`, which checks that the peer is a
// member, refuses offline peers at once, marks the call as the agent's, and writes the audit log.
// There is deliberately no tool for pausing peer access or for changing the network (enrolling a
// node, replacing the policy): those are the operator's.

const MAX_OUTPUT = 20000;
const peerName = { type: 'string', pattern: '^[A-Za-z0-9][A-Za-z0-9._-]*$', description: 'A peer from mesh_status, by short or full network name.' };
const none = { type: 'object', properties: {}, additionalProperties: false };

export const meshExecutable = (env = process.env) => env.DOJO_MESH_EXE || join(env.XDG_DATA_HOME || join(homedir(), '.local/share'), 'dojo/bin/dojo-mesh');

const clip = text => text.length <= MAX_OUTPUT ? text
  : `${text.slice(0, MAX_OUTPUT / 2)}\n… ${text.length - MAX_OUTPUT} characters left out …\n${text.slice(-MAX_OUTPUT / 2)}`;

// What an exit status of dojo-mesh asks of the model.
const meaning = {
  1: 'The command reached the peer and failed.',
  2: 'The request was malformed or the mesh is misconfigured.',
  3: 'A person has to act before this can go ahead. Tell the user what the message says. Do not retry it or find another way around it.',
  4: 'The call was refused or could not reach the peer (offline, not a member of the mesh, or peer access is paused). Tell the user; do not retry in a loop.',
};

const text = value => { if (typeof value !== 'string' || !value.length) throw new Error('A text value is required.'); return value; };
const peer = value => { if (typeof value !== 'string' || !/^[A-Za-z0-9][A-Za-z0-9._-]*$/.test(value)) throw new Error('Name a peer from mesh_status.'); return value; };

/** The agent's mesh tools as plain data: how each one maps to a `dojo-mesh` command line. */
export function meshTools() {
  return [
    { name: 'mesh_status', replay: 'safe', parameters: none,
      description: 'List this Dojo node and the other nodes (peers) it can reach, with whether each is online.',
      command: () => ['status', '--json'] },
    { name: 'mesh_find', replay: 'safe',
      parameters: { type: 'object', properties: { capability: { type: 'string', pattern: '^[a-z0-9][a-z0-9-]*$', description: 'For example nix-cache or voice-agent.' } }, required: ['capability'], additionalProperties: false },
      description: 'Ask the peers which of them offer a capability, and how to reach it.',
      command: args => { if (typeof args.capability !== 'string' || !/^[a-z0-9][a-z0-9-]*$/.test(args.capability)) throw new Error('A capability is lowercase letters, digits and dashes.'); return ['find', args.capability, '--json']; } },
    { name: 'mesh_exec', replay: 'unsafe',
      parameters: { type: 'object', properties: {
        peer: peerName,
        argv: { type: 'array', items: { type: 'string' }, minItems: 1, description: 'The program and its arguments, passed as they are with no shell.' },
        script: { type: 'string', description: 'A shell script, instead of argv, run with sh -c.' },
        timeout_seconds: { type: 'integer', minimum: 1, maximum: 600, description: 'Default 120.' },
      }, required: ['peer'], additionalProperties: false },
      description: 'Run a command on a peer as the same user, not as root. Give exactly one of argv or script. Prefer argv. The call is recorded in the audit log. For anything that needs the peer\'s desktop, use mesh_task instead.',
      command: args => {
        if ((args.argv === undefined) === (args.script === undefined)) throw new Error('Give exactly one of argv or script.');
        const argv = args.script !== undefined ? ['sh', '-c', text(args.script)] : args.argv;
        if (!Array.isArray(argv) || !argv.length || argv.some(part => typeof part !== 'string')) throw new Error('argv is a list of strings.');
        const seconds = args.timeout_seconds ?? 120;
        if (!Number.isInteger(seconds) || seconds < 1 || seconds > 600) throw new Error('timeout_seconds is between 1 and 600.');
        return ['exec', '--timeout', String(seconds), peer(args.peer), '--', ...argv];
      },
      timeoutMs: args => ((args.timeout_seconds ?? 120) + 15) * 1000 },
    { name: 'mesh_copy', replay: 'unsafe',
      parameters: { type: 'object', properties: {
        source: { type: 'string', description: 'A local path, or PEER:PATH.' },
        destination: { type: 'string', description: 'A local path, or PEER:PATH. Exactly one of the two is on a peer.' },
      }, required: ['source', 'destination'], additionalProperties: false },
      description: 'Copy one regular file between this node and a peer. An existing destination file is replaced.',
      command: args => ['cp', '--', text(args.source), text(args.destination)] },
    { name: 'mesh_task', replay: 'unsafe',
      parameters: { type: 'object', properties: { peer: peerName, text: { type: 'string', description: 'What the peer\'s own agent should do, as the user would say it.' } }, required: ['peer', 'text'], additionalProperties: false },
      description: 'Hand a task to the agent on a peer, which carries it out there with that machine\'s own tools. Use this for anything that needs the peer\'s desktop or browser, instead of trying to drive it remotely. It starts the task and returns; it does not wait for the result.',
      command: args => ['task', peer(args.peer), '--', text(args.text)] },
  ];
}

/** Runs `dojo-mesh` as the agent and reports its output with what the exit status means. */
export function runMesh(executable, args, { session, signal, timeoutMs = 60000 }) {
  return new Promise(resolve => {
    execFile(executable, args, { env: { ...process.env, DOJO_MESH_ORIGIN: 'agent', DOJO_MESH_SESSION: String(session) }, maxBuffer: 8 * 1024 * 1024, timeout: timeoutMs, signal, windowsHide: true }, (error, stdout, stderr) => {
      const code = error ? (typeof error.code === 'number' ? error.code : null) : 0;
      const body = clip([stdout, stderr].filter(Boolean).join('\n').trim());
      if (code === 0) return resolve({ text: body || 'Done.', isError: false });
      const why = code === null ? (error.killed ? 'The call was stopped because it took too long.' : `dojo-mesh could not run: ${error.message}`) : (meaning[code] ?? `dojo-mesh exited with status ${code}.`);
      resolve({ text: [body, why].filter(Boolean).join('\n'), isError: true });
    });
  });
}

/** The extension, or nothing when `dojo-mesh` is not installed here. */
export async function openMeshExtension(runtime) {
  const { pi, config } = runtime;
  const executable = config.meshExe ?? meshExecutable();
  try { await access(executable, constants.X_OK); } catch { return undefined; }
  return pi.defineExtension({ name: 'dojo-mesh', tools: meshTools().map(tool => pi.defineTool({
    name: tool.name, description: tool.description, parameters: tool.parameters, replay: tool.replay, executionMode: 'sequential',
    execute: async (args, api, context) => {
      let command;
      try { command = tool.command(args ?? {}); } catch (error) { return { output: [{ type: 'text', text: error.message }], isError: true }; }
      const result = await runMesh(executable, command, { session: api.conversationId, signal: context.abortSignal, timeoutMs: tool.timeoutMs?.(args) });
      return { output: [{ type: 'text', text: result.text }], isError: result.isError };
    },
  })) });
}
