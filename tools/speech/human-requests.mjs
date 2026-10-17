import { randomUUID } from 'node:crypto';

export async function installHumanRequests(runtime, registry, ai) {
  const { pi } = runtime;
  runtime.HumanState = pi.defineDoc({ kind: 'dojo.human-requests', version: 1, scope: 'conversation', history: 'latest', fork: 'initial', initial: () => ({ requests: {} }) });
  registry.install(pi.defineExtension({ name: 'dojo-human-requests', tools: [
    pi.defineTool({ name: 'human_request', description: 'Ask the user for clarification. Returns a durable pending request ID. This never grants permission; any resumed desktop input needs a fresh observation. Do not request passwords or credentials.',
      parameters: ai.Type.Object({ prompt: ai.Type.String({ minLength: 1, maxLength: 2048 }) }), replay: 'unsafe', executionMode: 'sequential',
      execute: async ({ prompt }, api, context) => {
        await runtime.invalidateDesktop?.();
        let request;
        await api.commit(async tx => {
          const state = await tx.doc(runtime.HumanState, api.conversationId);
          const now = Date.now();
          for (const item of Object.values(state.requests)) if (item.status === 'pending' && item.expiresAt <= now) item.status = 'expired';
          request = Object.values(state.requests).find(item => item.status === 'pending' && item.prompt === prompt);
          if (!request) {
            while (Object.keys(state.requests).length >= 64) {
              const oldest = Object.values(state.requests).filter(item => item.status !== 'pending').sort((a, b) => a.createdAt - b.createdAt)[0];
              if (!oldest) throw new Error('Human request limit reached');
              delete state.requests[oldest.id];
            }
            request = { id: randomUUID(), prompt, status: 'pending', createdAt: now, expiresAt: now + 900000, response: null };
            state.requests[request.id] = request;
          }
        }, context);
        runtime.onEvent({ type: 'human_request', request });
        return { output: [{ type: 'text', text: JSON.stringify(request) }] };
      },
    }),
    pi.defineTool({ name: 'human_requests', description: 'Read durable clarification requests and user responses. Completed clarification does not authorize desktop effects.', parameters: ai.Type.Object({}), replay: 'safe',
      execute: async (_args, api, context) => ({ output: [{ type: 'text', text: JSON.stringify((await api.snapshot(runtime.HumanState, api.conversationId, context))?.requests ?? {}) }] }),
    }),
  ] }));
}

// Called by the local user-input host, never registered as a model tool.
export async function respondHuman(runtime, id, response) {
  if (typeof response !== 'string' || !response.trim() || response.length > 4096) throw new Error('Invalid human clarification response');
  await runtime.conversation.commit(async tx => {
    const state = await tx.doc(runtime.HumanState, runtime.id);
    const request = state.requests[id];
    if (!request || request.status !== 'pending' || request.expiresAt <= Date.now()) throw new Error('Human request is unavailable or expired');
    request.status = 'completed'; request.response = response; request.completedAt = Date.now();
  }, runtime.context);
  await runtime.invalidateDesktop?.();
}

export async function cancelHumanRequests(runtime) {
  await runtime.conversation.commit(async tx => {
    const state = await tx.doc(runtime.HumanState, runtime.id);
    for (const request of Object.values(state.requests)) if (request.status === 'pending') request.status = 'cancelled';
  }, runtime.context);
}
