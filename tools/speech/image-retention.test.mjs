import test from 'node:test';
import assert from 'node:assert/strict';
import { projectDesktopImages } from './image-retention.mjs';

test('request retention handles zero/one/all without changing canonical messages or signatures', () => {
  const messages = [
    { role: 'assistant', content: [{ type: 'thinking', thinking: 'signed', thinkingSignature: 'signature' }, { type: 'toolCall', id: 'a' }] },
    { role: 'toolResult', toolName: 'desktop_observe', toolCallId: 'a', content: [{ type: 'text', text: 'old metadata' }, { type: 'image', data: 'old' }] },
    { role: 'toolResult', toolName: 'desktop_observe', toolCallId: 'b', content: [{ type: 'image', data: 'new' }] },
    { role: 'user', content: [{ type: 'image', data: 'user' }] },
  ];
  const original = structuredClone(messages);
  assert.equal(projectDesktopImages(messages, 0)[2].content[0].type, 'text');
  const one = projectDesktopImages(messages, 1);
  assert.equal(one[1].content[1].type, 'text');
  assert.equal(one[2].content[0].data, 'new');
  assert.equal(one[1].toolCallId, 'a');
  assert.equal(one[0].content[0].thinkingSignature, 'signature');
  assert.equal(one[3].content[0].data, 'user');
  assert.deepEqual(projectDesktopImages(messages, 8), original);
  assert.deepEqual(messages, original);
  assert.throws(() => projectDesktopImages(messages, -1), /image limit/);
});

test('step observation and action images share the desktop request limit', () => {
  const messages = ['computer_steps_observe','computer_steps_apply'].map((toolName,i)=>({role:'toolResult',toolName,content:[{type:'image',data:String(i)}]}));
  const projected=projectDesktopImages(messages,1);
  assert.equal(projected[0].content[0].type,'text');assert.equal(projected[1].content[0].data,'1');
  assert.equal(messages[0].content[0].type,'image');
});
