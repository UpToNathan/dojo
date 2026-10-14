import { randomUUID } from 'node:crypto';
import { createAssistantMessageEventStream } from '@earendil-works/pi-ai';

const profiles = new Set(['canonical', 'ui_tars', 'ui_tars2', 'qwen35', 'generic_vlm', 'fara', 'open_cua']);

export function decodedAction(value) {
  const action = value.outcome?.Action;
  if (!action) return null;
  const anchor = point => ({ x: point.delivered_x, y: point.delivered_y });
  const center = { x: (value.image_width - 1) / 2, y: (value.image_height - 1) / 2 };
  const base = { capture_id: value.capture_id, observe_after: true };
  if (action.Move) return { ...base, ...anchor(action.Move.at), input: { kind: 'move' } };
  if (action.Click) {
    if (![1, 2].includes(action.Click.count)) throw new Error('Unsupported click count');
    return { ...base, ...anchor(action.Click.at), input: { kind: action.Click.count === 2 ? 'double_click' : 'click', button: action.Click.button.toLowerCase() } };
  }
  if (action.Drag) {
    if (!action.Drag.path.length) throw new Error('Empty drag path');
    return { ...base, ...anchor(action.Drag.path[0]), input: { kind: 'drag', button: 'left', path: action.Drag.path.slice(1).map(anchor) } };
  }
  if (action.Type) return { ...base, ...(action.Type.at ? anchor(action.Type.at) : center), input: { kind: 'text', text: action.Type.text } };
  if (action.Keypress) return { ...base, ...center, input: action.Keypress.keys.length === 1 ? { kind: 'key', key: action.Keypress.keys[0] } : { kind: 'chord', keys: action.Keypress.keys } };
  if (action.Scroll) {
    if (action.Scroll.unit !== 'Pixels') throw new Error('Wheel-step conversion is unsupported');
    return { ...base, ...(action.Scroll.at ? anchor(action.Scroll.at) : center), input: { kind: 'scroll', horizontal: action.Scroll.dx, vertical: action.Scroll.dy } };
  }
  throw new Error('Unsupported desktop action in model profile');
}

// Transport/auth/usage stay in pi-ai. Rust parses completed text before Pi classifies it.
export function computerModelBridge(models, runtime, profile, foregroundRequestMessages) {
  if (!profiles.has(profile)) throw new Error('Unsupported text computer model profile');
  if (!(foregroundRequestMessages instanceof WeakSet)) throw new Error('Computer model bridge requires host-scoped request markers');
  const wrap = method => (model, context, options) => {
    // Scope is captured before transport or any await. Summary requests sharing
    // the model ID have no host marker and retain the normal provider stream.
    const foreground = model.id === runtime.agent.model.modelId
      && context.messages?.some(message => foregroundRequestMessages.has(message));
    if (!foreground) return models[method](model, context, options);
    const outer = createAssistantMessageEventStream();
    void (async () => {
      let message;
      try {
        message = await models[method](model, context, options).result();
        if (message.stopReason === 'error' || message.stopReason === 'aborted' || message.content.some(block => block.type === 'toolCall')) {
          outer.push({ type: message.stopReason === 'error' || message.stopReason === 'aborted' ? 'error' : 'done', reason: message.stopReason, [message.stopReason === 'error' || message.stopReason === 'aborted' ? 'error' : 'message']: message });
          outer.end(message); return;
        }
        const state = await runtime.harness.snapshot(runtime.DesktopState, runtime.id, runtime.context);
        if (state?.recoveryRequired || !state?.captureId || !runtime.desktop || runtime.desktop.closed) throw new Error('Fresh desktop observation required for text model output');
        const raw = message.content.filter(block => block.type === 'text').map(block => block.text).join('\n');
        const result = await runtime.decodeDesktopModel({ capture_id: state.captureId, profile, raw }, options?.signal);
        if (result.isError) throw new Error('Rust computer model decoder refused output');
        const text = result.content.find(block => block.type === 'text')?.text;
        const decoded = JSON.parse(text);
        const arguments_ = decodedAction(decoded);
        if (arguments_ && decoded.capture_digest) arguments_.model_source = {
          profile, parser_version: decoded.parser_version, raw,
          capture_digest: decoded.capture_digest, call_id: decoded.call_id ?? null,
        };
        if (arguments_) {
          message = { ...message, stopReason: 'toolUse', content: [...message.content.filter(block => block.type !== 'text'), { type: 'toolCall', id: decoded.call_id ?? randomUUID(), name: 'desktop_capture_action', arguments: arguments_ }] };
        } else if (decoded.outcome?.AskHuman) {
          const prompt = decoded.outcome.AskHuman.text;
          if (typeof prompt !== 'string' || !prompt.trim() || prompt.length > 2048) throw new Error('Invalid model clarification request');
          message = { ...message, stopReason: 'toolUse', content: [...message.content.filter(block => block.type !== 'text'), { type: 'toolCall', id: randomUUID(), name: 'human_request', arguments: { prompt } }] };
        } else if (!decoded.outcome?.Done && !decoded.outcome?.Refusal && !decoded.outcome?.AskHuman) throw new Error('Unknown decoded outcome');
        outer.push({ type: 'done', reason: message.stopReason, message }); outer.end(message);
      } catch (error) {
        const failed = { ...(message ?? { role: 'assistant', content: [], provider: model.provider, model: model.id, api: model.api, timestamp: Date.now() }), stopReason: 'error', errorMessage: error.message };
        outer.push({ type: 'error', reason: 'error', error: failed }); outer.end(failed);
      }
    })();
    return outer;
  };
  return new Proxy(models, { get(target, key) {
    if (key === 'stream' || key === 'streamSimple') return wrap(key);
    const value = Reflect.get(target, key, target);
    return typeof value === 'function' ? value.bind(target) : value;
  } });
}
