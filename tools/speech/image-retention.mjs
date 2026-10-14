// CUA image-retention behavior adapted to an immutable Pi request projection.
// Canonical entries, call/result pairing, signatures and non-image blocks remain intact.
export function projectDesktopImages(messages, retained = 8) {
  if (!Number.isSafeInteger(retained) || retained < 0 || retained > 128) throw new Error('Invalid desktop request image limit');
  let seen = 0;
  const projected = [...messages];
  for (let i = projected.length - 1; i >= 0; i--) {
    const message = projected[i];
    if (message.role !== 'toolResult' || !Array.isArray(message.content)) continue;
    if (!message.toolName?.startsWith('desktop_') && !message.toolName?.startsWith('computer_steps_') && message.toolName !== 'zoom') continue;
    const content = [...message.content];
    for (let j = content.length - 1; j >= 0; j--) {
      if (content[j].type !== 'image') continue;
      if (seen++ >= retained) content[j] = { type: 'text', text: '[Earlier desktop image omitted from this request]' };
    }
    projected[i] = { ...message, content };
  }
  return projected;
}
