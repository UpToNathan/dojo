#!/usr/bin/env node
import assert from 'node:assert/strict';
import { execFileSync, spawn } from 'node:child_process';
import { copyFile, mkdir, mkdtemp, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { createInterface } from 'node:readline';

const [fixture, bridge] = process.argv.slice(2);
assert(fixture?.startsWith('/') && bridge?.startsWith('/nix/store/'), 'Supply absolute fixture executable and built bridge store path');
if (process.env.DOJO_ATK_PRIVATE !== '1') {
  const root = await mkdtemp(join(tmpdir(), 'dojo-atk-endpoint.'));
  const runtime = join(root, 'runtime'), services = join(root, 'services');
  await mkdir(runtime, { mode: 0o700 }); await mkdir(services);
  await copyFile(join(bridge, 'share/dbus-1/services/org.a11y.Bus.service'), join(services, 'org.a11y.Bus.service'));
  const config = join(root, 'session.conf');
  await writeFile(config, `<busconfig><type>session</type><listen>unix:tmpdir=${root}</listen><auth>EXTERNAL</auth><servicedir>${services}</servicedir><policy context="default"><allow own="*"/><allow send_destination="*"/><allow receive_sender="*"/></policy></busconfig>`);
  const env = { ...process.env, DOJO_ATK_PRIVATE: '1', DOJO_ATK_HOST_BUS: process.env.DBUS_SESSION_BUS_ADDRESS ?? '', XDG_RUNTIME_DIR: runtime,
    GSETTINGS_BACKEND: 'memory', ATSPI_DBUS_IMPLEMENTATION: 'dbus-daemon' };
  for (const key of ['AT_SPI_BUS_ADDRESS', 'DBUS_SESSION_BUS_ADDRESS', 'DBUS_STARTER_ADDRESS', 'DBUS_STARTER_BUS_TYPE', 'DISPLAY', 'WAYLAND_DISPLAY', 'LD_LIBRARY_PATH', 'LD_PRELOAD']) delete env[key];
  const child = spawn('dbus-run-session', ['--config-file', config, '--', process.execPath, import.meta.filename, fixture, bridge], { env, stdio: 'inherit' });
  child.on('error', error => { throw error; });
  process.exitCode = await new Promise(resolve => child.once('exit', code => resolve(code ?? 1)));
} else {
  assert.notEqual(process.env.DBUS_SESSION_BUS_ADDRESS, process.env.DOJO_ATK_HOST_BUS);
  assert.equal(process.env.GSETTINGS_BACKEND, 'memory');
  const bus = args => execFileSync('busctl', ['--address', process.env.DBUS_SESSION_BUS_ADDRESS, ...args], { encoding: 'utf8', timeout: 10000 }).trim();
  bus(['set-property', 'org.a11y.Bus', '/org/a11y/bus', 'org.a11y.Status', 'IsEnabled', 'b', 'true']);
  const address = JSON.parse(bus(['call', 'org.a11y.Bus', '/org/a11y/bus', 'org.a11y.Bus', 'GetAddress']).slice(2));
  const app = spawn(fixture, [], { env: { ...process.env, NO_AT_BRIDGE: '0' }, stdio: ['pipe', 'pipe', 'inherit'] });
  app.on('error', error => { throw error; });
  const ended = new Promise(resolve => app.once('exit', code => resolve(code)));
  const lines = createInterface({ input: app.stdout })[Symbol.asyncIterator]();
  const next = async () => {
    let timer;
    try {
      const line = await Promise.race([lines.next(), new Promise((_, reject) => { timer = setTimeout(() => reject(new Error('Fixture response timed out')), 10000); })]);
      assert(!line.done, 'Fixture exited before responding');
      return JSON.parse(line.value);
    } finally { clearTimeout(timer); }
  };
  const property = (endpoint, key) => execFileSync('busctl', ['--address', address, 'get-property', endpoint.owner, endpoint[key], 'org.a11y.atspi.Accessible', 'Name'], { encoding: 'utf8', timeout: 10000 }).trim();
  try {
    const first = await next();
    assert(first.foreign_thread_refused);
    assert.notEqual(first.first, first.second);
    assert.equal(property(first, 'first'), 's "Dojo endpoint first"');
    assert.equal(property(first, 'second'), 's "Dojo endpoint second"');
    app.stdin.write('defunct\n');
    assert.equal((await next()).defunct_refused, true);
    app.stdin.write('cleanup\n');
    assert.equal((await next()).cleanup_refused, true);
    const owned = execFileSync('busctl', ['--address', address, 'call', 'org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus', 'NameHasOwner', 's', first.owner], { encoding: 'utf8', timeout: 10000 }).trim();
    assert.equal(owned, 'b false');
    app.stdin.write('init\n');
    const second = await next();
    assert.notEqual(first.owner, second.owner);
    assert.equal(property(second, 'first'), 's "Dojo endpoint first"');
    assert.equal(property(second, 'second'), 's "Dojo endpoint second"');
    app.stdin.write('quit\n');
    assert.equal(await ended, 0);
    console.log(JSON.stringify({ passed: true, exact_object_names: true, distinct_object_paths: true, foreign_thread_refused: true,
      uninitialized_refused: true, defunct_refused: true, cleanup_refused: true, retired_owner_unavailable: true, reinitialized_owner_changed: true,
      private_session_bus: true, private_runtime: true, memory_settings: true, no_gui: true, fixture, bridge }));
  } finally {
    if (app.exitCode === null) app.kill('SIGTERM');
  }
}
