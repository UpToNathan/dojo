#!/usr/bin/env node
// A stand-in for the Dojo voice service's account socket, used to check the model picker panel in an
// isolated session. It keeps state in memory, never logs a key, and records only action names.
import net from 'node:net';
import { appendFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';

const runtime = process.env.XDG_RUNTIME_DIR;
const logPath = process.env.FAKE_VOICE_LOG;
const loginSeconds = Number(process.env.FAKE_LOGIN_SECONDS ?? 6);
const socketPath = join(runtime, 'dojo-voice.sock');
rmSync(socketPath, { force: true });

const accountsAll = [
  { id: 'opencode-go', label: 'OpenCode Go' }, { id: 'opencode', label: 'OpenCode Zen' },
  { id: 'openai-codex', label: 'ChatGPT' }, { id: 'openai', label: 'OpenAI' },
];
const signedIn = new Set();
const models = {
  'opencode-go': ['DeepSeek V4.1 Flash', 'Qwen3.8 Flash', 'GLM-5.3-Flash', 'MiniMax-M3', 'Kimi K2.7 Code', 'Qwen3.7 Plus', 'Gemini 4 Argon Contributor', 'Qwen3.8 Max', 'Kimi K3'].map((name, index) => ({
    model: name.toLowerCase().replaceAll(' ', '-'), name, cost: { input: 0.15 + index * 0.2, output: 0.6 + index * 0.5 }, context: 1000000 })),
  'openai-codex': [{ model: 'gpt-6-luna', name: 'GPT-6 Luna', cost: { input: 1, output: 4 }, context: 400000 }],
};
let selection = null, login = { status: 'idle', active: false }, nextLogin = 0, completeTimer;

function catalog() {
  const rows = [];
  for (const id of signedIn) for (const row of models[id] ?? []) {
    rows.push({ account: id, accountLabel: accountsAll.find(a => a.id === id).label, api: 'openai-responses', maxTokens: 8000, reasoning: false, ...row });
  }
  if (!selection || !signedIn.has(selection.account)) selection = rows[0] ? { account: rows[0].account, model: rows[0].model, name: rows[0].name, available: true } : null;
  return {
    accounts: accountsAll.map(({ id, label }) => signedIn.has(id)
      ? { id, label, state: 'available', source: 'dojo', backend: 'keyring' } : { id, label, state: 'unavailable', reason: 'Not signed in.' }),
    models: rows, selection,
  };
}

function handle(command) {
  appendFileSync(logPath, `${command.action}${command.account ? ` ${command.account}` : ''}${command.method ? ` ${command.method}` : ''}\n`);
  switch (command.action) {
    case 'models': return catalog();
    case 'select-model': selection = { account: command.account, model: command.model, name: command.model, available: true }; return catalog();
    case 'sign-out': signedIn.delete(command.account); return catalog();
    case 'login-status': return login;
    case 'begin-chatgpt-login': {
      const loginId = `L${++nextLogin}`;
      login = { status: 'starting', active: true, loginId };
      setTimeout(() => { if (login.loginId === loginId && login.active) login = command.method === 'browser'
        ? { status: 'auth_url', active: true, loginId, url: 'https://auth.openai.com/oauth/authorize?fake=1' }
        : { status: 'device_code', active: true, loginId, userCode: 'WXYZ-1234', verificationUri: 'https://auth.openai.com/device' }; }, 700);
      if (command.method !== 'browser') {
        clearTimeout(completeTimer);
        completeTimer = setTimeout(() => { if (login.loginId === loginId && login.active) { signedIn.add('openai-codex'); login = { status: 'complete', active: false, loginId, account: 'openai-codex' }; } }, loginSeconds * 1000);
      }
      return { loginId };
    }
    case 'cancel-login': clearTimeout(completeTimer); login = { status: 'failure', active: false, loginId: command.loginId, message: 'Login cancelled.' }; return { cancelled: true };
    case 'login-input': return { accepted: true };
    case 'submit-opencode-key':
      if (command.key === 'bad') return { error: 'OpenCode rejected this key.' };
      signedIn.add(command.account); return { saved: true, account: command.account };
    default: return { error: `Unknown action ${command.action}` };
  }
}

net.createServer(socket => {
  let buffer = '';
  socket.setEncoding('utf8');
  socket.on('data', data => {
    buffer += data;
    if (!buffer.includes('\n')) return;
    let response;
    try { response = handle(JSON.parse(buffer)); } catch (error) { response = { error: error.message }; }
    socket.end(`${JSON.stringify(response)}\n`);
  });
  socket.on('error', () => {});
}).listen(socketPath, () => console.log('fake voice listening'));
