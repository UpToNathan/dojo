import { createInterface } from 'node:readline/promises';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { randomUUID } from 'node:crypto';
import { DojoCredentials, subscriptionProvider } from './providers.mjs';
import { dataDir } from './voice-state.mjs';
import { acquireServerLock } from './voice.mjs';

if (!process.stdin.isTTY) throw new Error('Run Dojo subscription login in an interactive terminal.');
const provider = process.argv[2] || 'openai-codex';
const owner = await acquireServerLock();
if (!owner) throw new Error('Stop dojo-voice.service before changing Dojo subscription credentials.');
const stateDir = join(dataDir, 'pi');
await mkdir(stateDir, { recursive: true, mode: 0o700 });
const consoleInput = createInterface({ input: process.stdin, output: process.stdout });
const signal = new AbortController();
process.once('SIGINT', () => signal.abort());
const deviceFile = join(stateDir, 'device-id');
let deviceId;
try { deviceId = (await readFile(deviceFile, 'utf8')).trim(); }
catch (error) { if (error.code !== 'ENOENT') throw error; deviceId = randomUUID(); await writeFile(deviceFile, deviceId, { mode: 0o600 }); }
try {
  const credential = await subscriptionProvider(provider).auth.oauth.login({
    signal: signal.signal,
    notify: event => {
      if (event.type === 'auth_url') console.log(`${event.instructions || 'Open this login link:'}\n${event.url}`);
      else if (event.type === 'device_code') console.log(`Open ${event.verificationUri} and enter ${event.userCode}`);
      else if (event.message) console.log(event.message);
    },
    prompt: question => consoleInput.question(`${question.message}\n`, { signal: question.signal || signal.signal }),
  }, { getDeviceId: () => deviceId, agentName: 'Dojo' });
  await new DojoCredentials(stateDir).modify(provider, async () => credential);
  console.log(`Dojo ${provider} subscription login saved privately.`);
} finally { consoleInput.close(); owner.stdin.end(); }
