import test from 'node:test';
import assert from 'node:assert/strict';
import { chmod, mkdir, mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { randomUUID } from 'node:crypto';
import { DojoCredentials, runCommand } from './credentials.mjs';

const noSecretService = () => Object.assign(new Error('org.freedesktop.DBus.Error.ServiceUnknown'), { code: 1 });
test('no Secret Service falls back to private file storage', async t => {
  const state = await mkdtemp(join(tmpdir(), 'dojo-credentials-')); t.after(() => rm(state, { recursive: true, force: true }));
  const store = new DojoCredentials(state, { command: async () => { throw noSecretService(); } });
  await store.modify('opencode-go', async () => ({ type: 'api_key', key: 'secret-value' }));
  assert.equal(store.backend, 'file');
  assert.equal((await store.read('opencode-go')).key, 'secret-value');
  assert.equal((await stat(store.file)).mode & 0o777, 0o600);
});

test('a missing Secret Service client is an error rather than a silent plaintext downgrade', async t => {
  const state = await mkdtemp(join(tmpdir(), 'dojo-credentials-no-client-')); t.after(() => rm(state, { recursive: true, force: true }));
  const store = new DojoCredentials(state, { command: async () => { throw Object.assign(new Error('spawn gdbus ENOENT'), { code: 'ENOENT' }); } });
  await assert.rejects(store.read('openai'), /gdbus ENOENT/);
  assert.equal(store.backend, 'unknown');
});

test('keyring keeps secrets on stdin, migrates only after verified round trip, and clears its cache on write', async t => {
  const state = await mkdtemp(join(tmpdir(), 'dojo-credentials-keyring-')); t.after(() => rm(state, { recursive: true, force: true }));
  const items = new Map(), calls = [];
  const command = async (exe, args, options = {}) => {
    calls.push({ exe, args, input: options.input });
    if (exe === 'gdbus') return args.some(arg => arg.endsWith('Secret.Service.ReadAlias')) ? "(objectpath '/org/freedesktop/secrets/collection/Default',)" : '(<false>,)';
    if (args[0] === 'lookup') return items.get(args.at(-1)) ?? (() => { throw Object.assign(new Error('no such item'), { code: 1 }); })();
    if (args[0] === 'store') { items.set(args.at(-1), options.input); return ''; }
    if (args[0] === 'clear') { items.delete(args.at(-1)); return ''; }
    throw new Error('unexpected command');
  };
  const secret = { type: 'api_key', key: 'never-in-argv' };
  const legacy = { 'opencode-go': secret };
  await import('node:fs/promises').then(fs => fs.writeFile(join(state, 'credentials.json'), JSON.stringify(legacy), { mode: 0o600 }));
  const store = new DojoCredentials(state, { command, cacheMs: 0 });
  assert.deepEqual(await store.read('opencode-go'), secret);
  assert.deepEqual(JSON.parse(items.get('opencode-go')), secret);
  assert.deepEqual(JSON.parse(await readFile(store.file, 'utf8')), {});
  await store.modify('opencode-go', async () => ({ type: 'api_key', key: 'replacement' }));
  assert.deepEqual(await store.read('opencode-go'), { type: 'api_key', key: 'replacement' });
  assert(calls.every(call => !call.args.includes('never-in-argv') && !call.args.includes('replacement')));
  const stored = calls.find(call => call.args[0] === 'store');
  assert.equal(stored.input, JSON.stringify(secret));
  assert.equal(await store.backendName(), 'keyring');
  await store.delete('opencode-go');
  assert.equal(await store.read('opencode-go'), undefined);
});

test('real child-process boundary keeps a key off argv with a fake secret-tool executable', async t => {
  const state = await mkdtemp(join(tmpdir(), 'dojo-credentials-executable-')); t.after(() => rm(state, { recursive: true, force: true }));
  const bin = join(state, 'bin'), items = join(state, 'items'), argvLog = join(state, 'argv.log');
  await mkdir(bin); await mkdir(items);
  const gdbus = join(bin, 'gdbus'), secretTool = join(bin, 'secret-tool');
  await writeFile(gdbus, "#!/bin/sh\ncase \"$*\" in *ReadAlias*) printf \"(objectpath '/fake/collection',)\\n\" ;; *) printf '(\\<false>,)\\n' ;; esac\n", { mode: 0o755 });
  await writeFile(secretTool, `#!/bin/sh\nset -eu\nop=$1; shift\naccount=''\nwhile [ $# -gt 0 ]; do if [ \"$1\" = account ]; then account=$2; shift 2; else shift; fi; done\nprintf '%s %s\\n' \"$op\" \"$account\" >> \"$FAKE_ARGV_LOG\"\ncase \"$op\" in store) cat > \"$FAKE_KEYRING_DIR/$account\" ;; lookup) if [ -f \"$FAKE_KEYRING_DIR/$account\" ]; then cat \"$FAKE_KEYRING_DIR/$account\"; else echo 'no such item' >&2; exit 1; fi ;; clear) rm -f \"$FAKE_KEYRING_DIR/$account\" ;; esac\n`, { mode: 0o755 });
  await chmod(gdbus, 0o755); await chmod(secretTool, 0o755);
  const env = { ...process.env, PATH: `${bin}:${process.env.PATH}`, FAKE_KEYRING_DIR: items, FAKE_ARGV_LOG: argvLog };
  const command = (exe, args, options) => runCommand(exe, args, { ...options, env });
  const store = new DojoCredentials(state, { command, cacheMs: 0 });
  const secret = 'not-an-argument';
  await store.modify('opencode-go', async () => ({ type: 'api_key', key: secret }));
  assert.equal((await store.read('opencode-go')).key, secret);
  assert.doesNotMatch(await readFile(argvLog, 'utf8'), new RegExp(secret));
  await store.delete('opencode-go');
});

test('borrowed subscription snapshots remain read-only and are not copied into the keyring', async t => {
  const state = await mkdtemp(join(tmpdir(), 'dojo-credentials-borrowed-')); t.after(() => rm(state, { recursive: true, force: true }));
  const borrowed = { type: 'oauth', access: 'snapshot', refresh: '', expires: Date.now() + 3600_000, borrowedSubscription: true };
  await import('node:fs/promises').then(fs => fs.writeFile(join(state, 'credentials.json'), JSON.stringify({ 'openai-codex': borrowed }), { mode: 0o600 }));
  const calls = [];
  const store = new DojoCredentials(state, { command: async (_exe, args) => {
    calls.push(args);
    if (args.some(arg => arg.endsWith('Secret.Service.ReadAlias'))) return "(objectpath '/collection',)";
    if (args.at(-1) === 'Locked') return '(<false>,)';
    if (args[0] === 'lookup') throw Object.assign(new Error('no such item'), { code: 1 });
    throw new Error('unexpected write');
  } });
  store.subscriptionSnapshot = async () => borrowed;
  assert.deepEqual(await store.read('openai-codex'), borrowed);
  assert.equal(calls.some(args => args[0] === 'store'), false);
  assert.deepEqual(JSON.parse(await readFile(store.file, 'utf8')), { 'openai-codex': borrowed });
});

test('every keyring lookup rechecks Locked so a later lock cannot trigger an unlock prompt', async t => {
  const state = await mkdtemp(join(tmpdir(), 'dojo-credentials-lock-transition-')); t.after(() => rm(state, { recursive: true, force: true }));
  let locked = false, lookups = 0;
  const store = new DojoCredentials(state, { cacheMs: 0, command: async (_exe, args) => {
    if (args.some(arg => arg.endsWith('Secret.Service.ReadAlias'))) return "(objectpath '/collection',)";
    if (args.at(-1) === 'Locked') return locked ? '(<true>,)' : '(<false>,)';
    if (args[0] === 'lookup') { lookups++; throw Object.assign(new Error('no such item'), { code: 1 }); }
    throw new Error('unexpected command');
  } });
  assert.equal(await store.read('openai'), undefined);
  const previousLookups = lookups;
  locked = true;
  await assert.rejects(store.read('openai'), /keyring is locked/i);
  assert.equal(lookups, previousLookups);
});

test('unlock raises the system dialog, then the credential can be read', async t => {
  const state = await mkdtemp(join(tmpdir(), 'dojo-credentials-unlock-')); t.after(() => rm(state, { recursive: true, force: true }));
  let locked = true; const calls = [];
  const store = new DojoCredentials(state, { cacheMs: 0, command: async (_exe, args, options = {}) => {
    calls.push({ args, timeout: options.timeout });
    if (args.some(arg => arg.endsWith('Secret.Service.ReadAlias'))) return "(objectpath '/collection',)";
    if (args.at(-1) === 'Locked') return locked ? '(<true>,)' : '(<false>,)';
    if (args[0] === 'lookup' && args.at(-1) === 'unlock') { locked = false; throw Object.assign(new Error('no such item'), { code: 1 }); }
    if (args[0] === 'lookup') return JSON.stringify({ type: 'api_key', key: 'unlocked' });
    throw new Error('unexpected command');
  } });
  await assert.rejects(store.read('openai'), /keyring is locked/i);
  assert.equal(calls.some(call => call.args.at(-1) === 'unlock'), false, 'discovery never raises the dialog');
  assert.equal(await store.unlock(), true);
  assert(calls.find(call => call.args.at(-1) === 'unlock').timeout > 60_000, 'the dialog needs time for a password');
  assert.deepEqual(await store.read('openai'), { type: 'api_key', key: 'unlocked' });
  assert.equal(store.backend, 'keyring');
});

test('a dismissed unlock dialog is not raised again straight away', async t => {
  const state = await mkdtemp(join(tmpdir(), 'dojo-credentials-dismissed-')); t.after(() => rm(state, { recursive: true, force: true }));
  let dialogs = 0;
  const store = new DojoCredentials(state, { command: async (_exe, args) => {
    if (args.some(arg => arg.endsWith('Secret.Service.ReadAlias'))) return "(objectpath '/collection',)";
    if (args.at(-1) === 'Locked') return '(<true>,)';
    if (args[0] === 'lookup') { dialogs++; throw new Error('dismissed'); }
    throw new Error('unexpected command');
  } });
  assert.equal(await store.unlock(), false);
  assert.equal(await store.unlock(), false);
  assert.equal(dialogs, 1);
});

test('unlock is a no-op without a Secret Service', async t => {
  const state = await mkdtemp(join(tmpdir(), 'dojo-credentials-unlock-none-')); t.after(() => rm(state, { recursive: true, force: true }));
  assert.equal(await new DojoCredentials(state, { command: async () => { throw noSecretService(); } }).unlock(), false);
});

test('an imported subscription snapshot is available without writing it to the keyring', async t => {
  const state = await mkdtemp(join(tmpdir(), 'dojo-credentials-external-borrowed-')); t.after(() => rm(state, { recursive: true, force: true }));
  const borrowed = { type: 'oauth', access: 'external-snapshot', refresh: '', expires: Date.now() + 3600_000, borrowedSubscription: true };
  const calls = [];
  const store = new DojoCredentials(state, { command: async (_exe, args) => {
    calls.push(args);
    if (args.some(arg => arg.endsWith('Secret.Service.ReadAlias'))) return "(objectpath '/collection',)";
    if (args.at(-1) === 'Locked') return '(<false>,)';
    if (args[0] === 'lookup') throw Object.assign(new Error('no such item'), { code: 1 });
    throw new Error('unexpected write');
  } });
  store.subscriptionSnapshot = async () => borrowed;
  assert.deepEqual(await store.read('openai-codex'), borrowed);
  assert.equal(calls.some(args => args[0] === 'store'), false);
  await assert.rejects(readFile(store.file), { code: 'ENOENT' });
});

test('a locked keyring is reported and is not silently downgraded to the file', async t => {
  const state = await mkdtemp(join(tmpdir(), 'dojo-credentials-locked-')); t.after(() => rm(state, { recursive: true, force: true }));
  await import('node:fs/promises').then(fs => fs.writeFile(join(state, 'credentials.json'), JSON.stringify({ openai: { type: 'api_key', key: 'file-key' } })));
  const store = new DojoCredentials(state, { command: async (_exe, args) => args.some(arg => arg.endsWith('Secret.Service.ReadAlias')) ? "(objectpath '/collection',)" : '(<true>,)' });
  await assert.rejects(store.read('openai'), /keyring is locked/i);
  assert.equal(store.backend, 'unknown');
});

test('migration retains the file when keyring read-back does not verify', async t => {
  const state = await mkdtemp(join(tmpdir(), 'dojo-credentials-unverified-')); t.after(() => rm(state, { recursive: true, force: true }));
  const secret = { type: 'api_key', key: 'private' };
  const source = join(state, 'credentials.json');
  await import('node:fs/promises').then(fs => fs.writeFile(source, JSON.stringify({ openai: secret }), { mode: 0o600 }));
  let lookup = 0;
  const command = async (_exe, args, options = {}) => {
    if (args.some(arg => arg.endsWith('Secret.Service.ReadAlias'))) return "(objectpath '/collection',)";
    if (args.at(-1) === 'Locked') return '(<false>,)';
    if (args[0] === 'store') return '';
    if (args[0] === 'lookup') { lookup++; if (lookup === 1) throw Object.assign(new Error('no such item'), { code: 1 }); return JSON.stringify({ type: 'api_key', key: 'wrong' }); }
    throw new Error('unexpected command');
  };
  const store = new DojoCredentials(state, { command });
  await assert.rejects(store.read('openai'), /source file was kept/);
  assert.deepEqual(JSON.parse(await readFile(source, 'utf8')), { openai: secret });
});

test('gated live Secret Service round trip removes its temporary item', { skip: process.env.DOJO_TEST_LIVE_KEYRING !== '1' }, async t => {
  const state = await mkdtemp(join(tmpdir(), 'dojo-live-keyring-')); t.after(() => rm(state, { recursive: true, force: true }));
  const account = `test-${randomUUID()}`;
  const store = new DojoCredentials(state);
  try {
    assert.equal(await store.backendName(), 'keyring');
    const value = { type: 'api_key', key: randomUUID() };
    await store.modify(account, async () => value);
    assert.deepEqual(await store.read(account), value);
  } finally { await store.delete(account); }
});

test('a missing keyring item reads as absent when secret-tool exits silently, as the real tool does', async t => {
  const state = await mkdtemp(join(tmpdir(), 'dojo-credentials-missing-')); t.after(() => rm(state, { recursive: true, force: true }));
  const command = async (name, args) => {
    if (name === 'gdbus') return args.some(arg => arg.endsWith('ReadAlias')) ? "(objectpath '/org/freedesktop/secrets/collection/login',)" : '(<false>,)';
    throw Object.assign(new Error('secret-tool exited (1).'), { code: 1, stdout: '', stderr: '' });
  };
  const store = new DojoCredentials(state, { command });
  assert.equal(await store.backendName(), 'keyring');
  assert.equal(await store.read('opencode-go'), undefined);
});

test('concurrent writes are serialized and timed-out probes fail visibly', async t => {
  const state = await mkdtemp(join(tmpdir(), 'dojo-credentials-chain-')); t.after(() => rm(state, { recursive: true, force: true }));
  const store = new DojoCredentials(state, { command: async () => { throw noSecretService(); } });
  await Promise.all(['a', 'b', 'c'].map((key, index) => store.modify(key, async () => ({ type: 'api_key', key: String(index) }))));
  assert.deepEqual((await store.list()).map(row => row.providerId).sort(), ['a', 'b', 'c']);
  const timeoutStore = new DojoCredentials(state, { command: async () => { throw Object.assign(new Error('timed out'), { code: 'ETIMEDOUT' }); } });
  await assert.rejects(timeoutStore.read('openai'), /timed out/i);
});
