import { spawn } from 'node:child_process';
import { createHash, randomUUID } from 'node:crypto';
import { helperBinary } from './helper-binary.mjs';

const text = value => ({ type: 'text', text: JSON.stringify(value) });
const metadata = result => {
  if (result?.structuredContent && typeof result.structuredContent === 'object') return result.structuredContent;
  for (const part of result?.content ?? []) if (part.type === 'text') {
    try { const value = JSON.parse(part.text); if (value && typeof value === 'object') return value; } catch {}
  }
  return {};
};
const MAX_BYTES = 65536;
/** Rust owns every phase transition. This transport creates no desktop effects. */
export function computerStepPolicy(runtime, input) {
  const bytes = JSON.stringify(input);
  if (Buffer.byteLength(bytes) > MAX_BYTES) return Promise.reject(new Error('Step state exceeds64KiB'));
  const binary = runtime.config.computerPolicyBinary ?? helperBinary(runtime.root, 'dojo-computer-verification');
  return new Promise((resolve, reject) => {
    const child = spawn(binary, [], { stdio: ['pipe', 'pipe', 'pipe'] });
    let size = 0, settled = false; const output = [];
    const finish = (error, value) => {
      if (settled) return; settled = true; clearTimeout(timer);
      if (error) { child.kill('SIGKILL'); reject(error); } else resolve(value);
    };
    const timer = setTimeout(() => finish(new Error('Rust step policy timed out')), 5000);
    child.on('error', error => finish(error)); child.stdin.on('error', error => finish(error)); child.stderr.resume();
    child.stdout.on('data', chunk => { size += chunk.length; if (size > MAX_BYTES) finish(new Error('Rust step output exceeds64KiB')); else output.push(chunk); });
    child.on('close', code => {
      if (code !== 0) finish(new Error('Rust step policy refused transition'));
      else { try { finish(null, JSON.parse(Buffer.concat(output).toString())); } catch { finish(new Error('Malformed Rust step policy result')); } }
    });
    child.stdin.end(bytes);
  });
}
async function desktopCall(runtime, name, args, api, context, state, now) {
  const controller = new AbortController();
  const abort = () => controller.abort();
  context.abortSignal?.addEventListener('abort', abort, { once: true });
  if (context.abortSignal?.aborted) controller.abort();
  const remaining = state.started_ms + state.limits.timeout_ms - now();
  if (remaining <= 0) controller.abort();
  const timer = setTimeout(abort, Math.max(1, remaining));
  try { return await runtime.executeDesktopTool(name, args, api, context, controller.signal); }
  finally { clearTimeout(timer); context.abortSignal?.removeEventListener('abort', abort); }
}
const object = properties => ({ type: 'object', properties, additionalProperties: false, required: Object.keys(properties) });
const runId = { type: 'string', minLength: 1, maxLength: 256 };

// Local references in a catalog schema are rooted at that schema. Once nested
// inside a step tool, their pointers must follow it without changing constraints.
export function nestDesktopSchema(schema, property) {
  if (!['action', 'observation'].includes(property)) throw new Error('Invalid desktop schema location');
  const resolve = ref => {
    if (ref !== '#' && !ref.startsWith('#/')) throw new Error('Desktop schemas require local pointer references');
    let value = schema;
    for (const token of ref === '#' ? [] : decodeURIComponent(ref.slice(2)).split('/')) {
      const key = token.replace(/~1/g, '/').replace(/~0/g, '~');
      if (!value || typeof value !== 'object' || !Object.hasOwn(value, key)) throw new Error('Desktop schema reference is unavailable');
      value = value[key];
    }
    return `#/properties/${property}${ref.slice(1)}`;
  };
  const visit = value => {
    if (Array.isArray(value)) return value.map(visit);
    if (!value || typeof value !== 'object') return value;
    return Object.fromEntries(Object.entries(value).map(([key, item]) => {
      if (key === '$ref') {
        if (typeof item !== 'string') throw new Error('Invalid desktop schema reference');
        return [key, resolve(item)];
      }
      if (['$id', '$anchor', '$dynamicRef', '$dynamicAnchor', '$recursiveRef'].includes(key)) throw new Error('Unsupported desktop schema reference scope');
      // These keywords contain literal data, not schemas.
      return [key, ['const', 'enum', 'default', 'examples'].includes(key) ? structuredClone(item) : visit(item)];
    }));
  };
  return visit(schema);
}

/** One tool call observes or executes ONE current action. Pi remains the run scheduler. */
export async function installComputerSteps(runtime, registry, _ai, { now = () => Date.now(), policy = computerStepPolicy } = {}) {
  if (typeof runtime.executeDesktopTool !== 'function') throw new Error('Computer steps require the guarded desktop tool executor');
  const observeSchema = runtime.desktop?.tools?.find(tool => tool.name === 'desktop_observe')?.inputSchema;
  const actionSchema = runtime.desktop?.tools?.find(tool => tool.name === 'desktop_capture_action')?.inputSchema;
  if (!observeSchema || !actionSchema) throw new Error('Computer steps require current observation/action schemas');
  runtime.ComputerSteps = runtime.pi.defineDoc({ kind: 'dojo.computer-steps', version: 1, scope: 'conversation', history: 'latest', fork: 'initial', initial: () => ({ run: null }) });
  const persist = async (api, context, event, expectedRun) => {
    let next;
    await api.commit(async tx => {
      const doc = await tx.doc(runtime.ComputerSteps, api.conversationId);
      if (!doc.run || doc.run.run_id !== expectedRun) throw new Error('Computer step run is unavailable');
      next = await policy(runtime, { command: 'steps_transition', state: doc.run, event, now_ms: now() });
      doc.run = next;
    }, context);
    return next;
  };
  const snapshot = async (api, context, expectedRun) => {
    const doc = await api.snapshot(runtime.ComputerSteps, api.conversationId, context);
    if (!doc?.run || doc.run.run_id !== expectedRun) throw new Error('Computer step run is unavailable');
    return doc.run;
  };
  runtime.cancelComputerSteps = async () => {
    await runtime.conversation.commit(async tx => {
      const doc = await tx.doc(runtime.ComputerSteps, runtime.id);
      if (doc.run) doc.run = await policy(runtime, { command: 'steps_transition', state: doc.run, event: { kind: 'cancel' }, now_ms: now() });
    }, runtime.context);
    runtime.releaseDesktop?.('Computer step run cancelled');
    await runtime.invalidateDesktop?.();
  };
  registry.install(runtime.pi.defineExtension({ name: 'dojo-computer-steps', tools: [
    runtime.pi.defineTool({ name: 'computer_steps_begin', description: 'Begin a bounded sequence. Each action must follow a new computer_steps_observe. The run records delivery, not verified task success. Unknown effects block another run pending operator recovery.', replay: 'unsafe', executionMode: 'sequential',
      parameters: { type: 'object', additionalProperties: false, properties: { max_steps: { type: 'integer', minimum: 1, maximum: 64, default: 16 }, timeout_ms: { type: 'integer', minimum: 1, maximum: 300000, default: 60000 } } },
      execute: async ({ max_steps = 16, timeout_ms = 60000 }, api, context) => {
        let state;
        await api.commit(async tx => {
          const doc = await tx.doc(runtime.ComputerSteps, api.conversationId);
          if (doc.run?.phase.kind === 'in_flight' || doc.run?.phase.ambiguous_step) throw new Error('An effect is unresolved; operator recovery is required');
          if (doc.run && !['finished', 'stopped'].includes(doc.run.phase.kind)) throw new Error('A computer step run is already active');
          state = await policy(runtime, { command: 'steps_new', run_id: randomUUID(), limits: { max_steps, timeout_ms }, now_ms: now() });
          doc.run = state;
        }, context);
        await runtime.invalidateDesktop?.();
        return { output: [text({ step_state: state, task_verified: false })] };
      },
    }),
    runtime.pi.defineTool({ name: 'computer_steps_observe', description: 'Acquire a fresh server observation for the next bounded step. Select the target using the desktop observation schema. Existing post-action captures cannot substitute for this call.', replay: 'unsafe', executionMode: 'sequential',
      parameters: object({ run_id: runId, observation: nestDesktopSchema(observeSchema, 'observation') }),
      execute: async ({ run_id, observation }, api, context) => {
        // Invalidates a persisted ready capture after restart, and freezes an interrupted effect as unknown.
        let state = await persist(api, context, { kind: 'restart' }, run_id);
        if (state.phase.kind !== 'observe') { runtime.releaseDesktop?.('Computer step run stopped'); await runtime.invalidateDesktop?.(); return { output: [text({ step_state: state, task_verified: false })], isError: true }; }
        try {
          const result = await desktopCall(runtime, 'desktop_observe', observation, api, context, state, now);
          const value = metadata(result);
          if (result.isError || typeof value.capture_id !== 'string' || !value.capture_id) {
            state = await persist(api, context, { kind: 'cancel' }, run_id);
            return { output: [...(result.content ?? []), text({ step_state: state, task_verified: false })], isError: true };
          }
          state = await persist(api, context, { kind: 'observed', capture_id: value.capture_id, observation_id: value.capture_id }, run_id);
          if (state.phase.kind !== 'ready') { runtime.releaseDesktop?.('Computer observation expired'); await runtime.invalidateDesktop?.(); }
          return { output: [...result.content, text({ step_state: state, task_verified: false })], isError: state.phase.kind !== 'ready' };
        } catch (error) {
          await persist(api, context, { kind: 'cancel' }, run_id).catch(() => {});
          throw error;
        }
      },
    }),
    runtime.pi.defineTool({ name: 'computer_steps_apply', description: 'Apply exactly one action planned from this run’s latest fresh observation. The capture is retired before transport. Refusal, human pause, stale state, partial or unknown effects stop the run; no pending effect is replayed.', replay: 'unsafe', executionMode: 'sequential',
      parameters: object({ run_id: runId, action: nestDesktopSchema(actionSchema, 'action') }),
      execute: async ({ run_id, action }, api, context) => {
        const encoded = JSON.stringify(action);
        if (Buffer.byteLength(encoded) > 32768) throw new Error('Step action exceeds32KiB');
        // Dispatch the exact cloned argument whose digest is persisted, avoiding mutation after intent commit.
        const args = JSON.parse(encoded);
        const intent_id = randomUUID();
        let state = await persist(api, context, { kind: 'begin_effect', capture_id: args.capture_id, intent_id,
          action_digest: createHash('sha256').update(encoded).digest('hex') }, run_id);
        if (state.phase.kind !== 'in_flight') { runtime.releaseDesktop?.('Computer step run stopped'); await runtime.invalidateDesktop?.(); return { output: [text({ step_state: state, task_verified: false })], isError: true }; }
        let result;
        try {
          result = await desktopCall(runtime, 'desktop_capture_action', args, api, context, state, now);
          const value = metadata(result);
          const outcome = !result.isError && ['handled', 'confirmed', 'refused', 'paused', 'stale', 'partial', 'unknown'].includes(value.effect) ? value.effect : 'unknown';
          state = await persist(api, context, { kind: 'receipt', intent_id, outcome }, run_id);
        } catch (error) {
          // Failure to persist this receipt leaves the durable in-flight intent, which restart also classifies unknown.
          await persist(api, context, { kind: 'receipt', intent_id, outcome: 'unknown' }, run_id).catch(() => {});
          runtime.releaseDesktop?.('Computer step effect is unknown');
          await runtime.invalidateDesktop?.().catch(() => {});
          throw error;
        }
        if (state.phase.kind === 'stopped') { runtime.releaseDesktop?.('Computer step run stopped'); await runtime.invalidateDesktop?.(); }
        return { output: [...(result.content ?? []), text({ step_state: state, task_verified: false })], isError: result.isError || state.phase.kind === 'stopped' };
      },
    }),
    runtime.pi.defineTool({ name: 'computer_steps_status', description: 'Inspect the completed prefix and any ambiguous effect. Restart recovery retires ready captures and blocks pending effects; this tool sends no desktop input.', replay: 'safe', executionMode: 'sequential', parameters: object({ run_id: runId }),
      execute: async ({ run_id }, api, context) => {
        await snapshot(api, context, run_id);
        const state = await persist(api, context, { kind: 'restart' }, run_id);
        if (state.phase.kind === 'stopped') { runtime.releaseDesktop?.('Computer step recovery required'); await runtime.invalidateDesktop?.(); }
        return { output: [text({ step_state: state, task_verified: false })] };
      },
    }),
    runtime.pi.defineTool({ name: 'computer_steps_cancel', description: 'Stop the bounded run and release desktop ownership. Any pending effect remains ambiguous; cancellation grants no retry permission.', replay: 'safe', executionMode: 'sequential', parameters: object({ run_id: runId }),
      execute: async ({ run_id }, api, context) => {
        const state = await persist(api, context, { kind: 'cancel' }, run_id);
        runtime.releaseDesktop?.('Computer step run cancelled'); await runtime.invalidateDesktop?.();
        return { output: [text({ step_state: state, task_verified: false })] };
      },
    }),
  ] }));
}
