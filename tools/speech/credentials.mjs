import { spawn } from 'node:child_process';
import { chmod, mkdir, readFile, rename, writeFile } from 'node:fs/promises';
import { randomUUID } from 'node:crypto';
import { join } from 'node:path';
import { homedir } from 'node:os';

const timeoutMs = 1500;
const unlockTimeoutMs = 120_000;
const unlockRetryMs = 30_000;
const renewalMessage = 'The shared subscription login needs renewal in its owning app, or a separate Dojo login. Dojo will not rotate another app\'s credentials.';

export function runCommand(command, args, { input, timeout = timeoutMs, env } = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { stdio: ['pipe', 'pipe', 'pipe'], ...(env ? { env } : {}) });
    let stdout = '', stderr = '', settled = false;
    const timer = setTimeout(() => {
      child.kill('SIGKILL');
      finish(Object.assign(new Error(`${command} timed out.`), { code: 'ETIMEDOUT' }));
    }, timeout);
    const finish = (error, result) => {
      if (settled) return;
      settled = true; clearTimeout(timer);
      error ? reject(error) : resolve(result);
    };
    child.stdout.setEncoding('utf8'); child.stderr.setEncoding('utf8');
    child.stdin.on('error', () => {});
    child.stdout.on('data', data => { stdout = (stdout + data).slice(-64_000); });
    child.stderr.on('data', data => { stderr = (stderr + data).slice(-8_000); });
    child.once('error', error => finish(error));
    child.once('close', code => code === 0 ? finish(null, stdout.trim())
      : finish(Object.assign(new Error(stderr.trim() || `${command} exited (${code}).`), { code: code ?? 1, stdout, stderr })));
    if (input !== undefined) child.stdin.end(input); else child.stdin.end();
  });
}

async function readJson(path, fallback) {
  try { return JSON.parse(await readFile(path, 'utf8')); }
  catch (error) { if (error.code === 'ENOENT') return fallback; throw error; }
}

// The real secret-tool exits 1 with no output when an item does not exist.
function isMissing(error) { return /no such item|no such secret|no matching items|not found/i.test(error.message) || (error.code === 1 && !(error.stderr ?? '').trim()); }

export class DojoCredentials {
  constructor(stateDir, { command = runCommand, cacheMs = 1500, timeout = timeoutMs } = {}) {
    this.file = join(stateDir, 'credentials.json');
    this.stateDir = stateDir;
    this.command = command;
    this.cacheMs = cacheMs;
    this.timeout = timeout;
    this.chain = Promise.resolve();
    this.cache = new Map();
    this.probePromise = undefined;
    this.backend = 'unknown';
    this.unlockTimeout = unlockTimeoutMs;
    this.unlockAttemptAt = 0;
  }
  async defaultCollection() {
    const alias = await this.command('gdbus', ['call', '--session', '--dest', 'org.freedesktop.secrets', '--object-path', '/org/freedesktop/secrets', '--method', 'org.freedesktop.Secret.Service.ReadAlias', 'default'], { timeout: this.timeout });
    const objectPath = /objectpath '([^']+)'/.exec(alias)?.[1];
    if (!objectPath) throw new Error('Secret Service returned no default collection.');
    return objectPath;
  }
  async isLocked(objectPath) {
    const locked = await this.command('gdbus', ['call', '--session', '--dest', 'org.freedesktop.secrets', '--object-path', objectPath, '--method', 'org.freedesktop.DBus.Properties.Get', 'org.freedesktop.Secret.Collection', 'Locked'], { timeout: this.timeout });
    return /true/.test(locked);
  }
  // The Secret Service only raises its unlock dialog when something asks for an item in a locked
  // collection; reading the Locked property never does. Discovery therefore reports a locked keyring
  // without prompting, and a user's command calls this to raise the dialog once. A dismissed dialog is
  // remembered briefly so it does not reappear on every retry. Resolves true when the keyring is open.
  unlock() {
    const work = this.chain.then(async () => {
      let objectPath;
      try { objectPath = await this.defaultCollection(); } catch { return false; }
      if (!await this.isLocked(objectPath)) return true;
      if (Date.now() - this.unlockAttemptAt < unlockRetryMs) return false;
      this.unlockAttemptAt = Date.now();
      try { await this.command('secret-tool', ['lookup', 'service', 'dojo', 'account', 'unlock'], { timeout: this.unlockTimeout }); }
      catch { /* A missing item exits 1; a dismissed dialog leaves the collection locked, which is rechecked next. */ }
      this.cache.clear();
      return !await this.isLocked(objectPath);
    });
    this.chain = work.catch(() => {});
    return work;
  }
  async probe() {
    if (this.probePromise) return this.probePromise;
    const probePromise = (async () => {
      try {
        const objectPath = await this.defaultCollection();
        if (await this.isLocked(objectPath)) throw new Error('The Dojo keyring is locked. Unlock it in the system dialog, then retry.');
        this.backend = 'keyring';
        return { backend: this.backend, objectPath };
      } catch (error) {
        if (/locked/i.test(error.message) || error.code === 'ETIMEDOUT') throw error;
        if (!/ServiceUnknown|NameHasNoOwner|org\.freedesktop\.DBus\.Error|cannot autolaunch|not provided/i.test(error.message)) throw error;
        this.backend = 'file';
        return { backend: this.backend };
      }
    })();
    this.probePromise = probePromise;
    try { return await probePromise; }
    finally { if (this.probePromise === probePromise) this.probePromise = undefined; }
  }
  backendName() { return this.probe().then(value => value.backend); }
  async fileValues() { return readJson(this.file, {}); }
  async writeFileValues(values) {
    await mkdir(this.stateDir, { recursive: true, mode: 0o700 });
    const temporary = `${this.file}.${randomUUID()}.tmp`;
    await writeFile(temporary, JSON.stringify(values), { mode: 0o600, flag: 'wx' });
    await rename(temporary, this.file);
    await chmod(this.file, 0o600);
  }
  async keyringRead(account) {
    const backend = await this.probe();
    if (backend.backend !== 'keyring') throw new Error('Secret Service became unavailable before a credential lookup.');
    try {
      const raw = await this.command('secret-tool', ['lookup', 'service', 'dojo', 'account', account], { timeout: this.timeout });
      if (!raw) return undefined;
      return JSON.parse(raw);
    } catch (error) {
      if (isMissing(error)) return undefined;
      throw error;
    }
  }
  async migrate(account, oldValue) {
    if (oldValue === undefined || oldValue.borrowedSubscription) return oldValue;
    const serialized = JSON.stringify(oldValue);
    await this.command('secret-tool', ['store', '--label=Dojo account', 'service', 'dojo', 'account', account], { input: serialized, timeout: this.timeout });
    const verified = await this.keyringRead(account);
    if (JSON.stringify(verified) !== serialized) throw new Error('Dojo keyring migration could not verify the stored credential. The source file was kept.');
    const values = await this.fileValues();
    if (JSON.stringify(values[account]) === serialized) {
      delete values[account];
      await this.writeFileValues(values);
    }
    return verified;
  }
  read(provider) {
    const work = this.chain.then(() => this.readCurrent(provider));
    this.chain = work.catch(() => {});
    return work;
  }
  async readCurrent(provider) {
    const cached = this.cache.get(provider);
    if (cached && cached.until > Date.now()) return cached.value;
    const backend = await this.probe();
    let value;
    if (backend.backend === 'keyring') {
      value = await this.keyringRead(provider);
      if (value === undefined) value = await this.migrate(provider, (await this.fileValues())[provider]);
    } else value = (await this.fileValues())[provider];
    if (value === undefined) {
      try {
        const candidate = await this.importCandidate(provider);
        if (candidate?.borrowedSubscription) value = candidate;
      } catch { /* Discovery reports unavailable imports without treating them as stored credentials. */ }
    }
    if (value?.borrowedSubscription) {
      value = await this.subscriptionSnapshot(provider);
      if (value.expires <= Date.now() + 60_000) throw new Error(renewalMessage);
    }
    this.cache.set(provider, { value, until: Date.now() + this.cacheMs });
    return value;
  }
  list() {
    const work = this.chain.then(async () => {
      const backend = await this.probe();
      const values = new Map();
      if (backend.backend === 'file') {
        for (const [providerId, value] of Object.entries(await this.fileValues())) values.set(providerId, value);
      } else {
        // Only Dojo's four supported IDs are queried; no secret is returned in metadata.
        const legacy = await this.fileValues();
        for (const providerId of ['opencode-go', 'opencode', 'openai-codex', 'openai']) {
          let value = await this.keyringRead(providerId);
          if (value === undefined) value = await this.migrate(providerId, legacy[providerId]);
          if (value !== undefined) values.set(providerId, value);
        }
      }
      return [...values].map(([providerId, value]) => ({ providerId, type: value.type }));
    });
    this.chain = work.catch(() => {});
    return work;
  }
  modify(provider, change, { signal } = {}) {
    const work = this.chain.then(async () => {
      signal?.throwIfAborted();
      const backend = await this.probe();
      const current = await this.readCurrent(provider);
      const value = await change(current);
      signal?.throwIfAborted();
      if (value !== undefined) {
        if (backend.backend === 'keyring') {
          const serialized = JSON.stringify(value);
          await this.command('secret-tool', ['store', '--label=Dojo account', 'service', 'dojo', 'account', provider], { input: serialized, timeout: this.timeout });
          if (JSON.stringify(await this.keyringRead(provider)) !== serialized) throw new Error('Dojo keyring could not verify the saved credential.');
          const files = await this.fileValues();
          if (files[provider] !== undefined) { delete files[provider]; await this.writeFileValues(files); }
        } else {
          const values = await this.fileValues(); values[provider] = value; await this.writeFileValues(values);
        }
      }
      this.cache.delete(provider);
      return value;
    });
    this.chain = work.catch(() => {});
    return work;
  }
  delete(provider) {
    const work = this.chain.then(async () => {
      const backend = await this.probe();
      if (backend.backend === 'keyring') {
        await this.command('secret-tool', ['clear', 'service', 'dojo', 'account', provider], { timeout: this.timeout }).catch(error => { if (!isMissing(error)) throw error; });
        const values = await this.fileValues();
        if (values[provider] !== undefined) { delete values[provider]; await this.writeFileValues(values); }
      } else {
        const values = await this.fileValues(); delete values[provider]; await this.writeFileValues(values);
      }
      this.cache.delete(provider);
    });
    this.chain = work.catch(() => {});
    return work;
  }
  async importAccount(provider, options) {
    if (await this.read(provider)) return;
    await this.modify(provider, async () => await this.importCandidate(provider, options));
  }
  importSubscription(provider) { return this.importAccount(provider); }
  async importCandidate(provider, { env = process.env } = {}) {
    const OPENCODE_KEY_PROVIDERS = new Set(['opencode', 'opencode-go']);
    if (OPENCODE_KEY_PROVIDERS.has(provider)) {
      const explicit = env.DOJO_OPENCODE_API_KEY;
      if (typeof explicit === 'string' && explicit.trim()) return { type: 'api_key', key: explicit.trim() };
      const entry = (await readJson(join(homedir(), '.pi/agent/auth.json'), {}))[provider];
      if (entry?.type === 'api_key' && typeof entry.key === 'string' && entry.key) return { type: 'api_key', key: entry.key };
      throw new Error('Add an OpenCode API key.');
    }
    if (provider === 'openai-codex') {
      const auth = await readJson(join(homedir(), '.codex/auth.json'), null);
      const tokens = auth?.tokens;
      if (!tokens?.access_token) throw new Error('Sign in to Codex before enabling Dojo subscription execution.');
      const claims = JSON.parse(Buffer.from(tokens.access_token.split('.')[1], 'base64url'));
      if (!Number.isFinite(claims.exp)) throw new Error('Codex subscription credential has no expiry.');
      return { type: 'oauth', access: tokens.access_token, refresh: '', borrowedSubscription: true,
        expires: claims.exp * 1000, accountId: tokens.account_id };
    }
    const snapshot = await this.subscriptionSnapshot(provider);
    if (snapshot.expires <= Date.now() + 60_000) throw new Error(renewalMessage);
    return snapshot;
  }
  async subscriptionSnapshot(provider) {
    if (provider === 'openai-codex') {
      const auth = await readJson(join(homedir(), '.codex/auth.json'), null);
      const tokens = auth?.tokens;
      if (!tokens?.access_token) throw new Error('Sign in to Codex before enabling Dojo subscription execution.');
      const claims = JSON.parse(Buffer.from(tokens.access_token.split('.')[1], 'base64url'));
      if (!Number.isFinite(claims.exp)) throw new Error('Codex subscription credential has no expiry.');
      return { type: 'oauth', access: tokens.access_token, refresh: '', borrowedSubscription: true,
        expires: claims.exp * 1000, accountId: tokens.account_id };
    }
    const auth = await readJson(join(homedir(), '.pi/agent/auth.json'), {});
    if (auth[provider]?.type !== 'oauth') throw new Error(`Dojo needs a ${provider} subscription login; API-key fallback is disabled.`);
    if (typeof auth[provider].access !== 'string' || !auth[provider].access || !Number.isFinite(auth[provider].expires)) throw new Error('Invalid subscription access credential.');
    return { ...auth[provider], refresh: '', borrowedSubscription: true };
  }
}
