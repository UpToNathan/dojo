#!/usr/bin/env node
import assert from 'node:assert/strict';
import { createServer } from 'node:net';
import { spawn } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const directory = await mkdtemp(join(tmpdir(), 'dojo-binding-audit-'));
try {
  for (const mode of ['annotated', 'unbound', 'unsupported', 'retired']) {
    const path = join(directory, `${mode}.sock`);
    let snapshots = 0;
    const server = createServer({ allowHalfOpen: true }, socket => {
      let input = '';
      socket.on('data', chunk => { input += chunk; });
      socket.on('end', () => {
        const request = JSON.parse(input);
        let result;
        if (request === 'Windows') {
          snapshots++;
          result = { Ok: { Windows: mode === 'retired' && snapshots > 1 ? [] : [
            { id: 42, pid: process.pid, app_id: 'fixture.application', title: 'PRIVATE_TITLE_MUST_NOT_LEAK' },
          ] } };
        } else if (mode === 'unsupported') {
          result = { Err: 'error parsing request' };
        } else {
          assert.deepEqual(request, { AccessibilityTarget: { window_id: 42 } });
          result = { Ok: { AccessibilityTarget: { window_id: 42, pid: process.pid, generation: 7,
            annotation_generation: 3, endpoint: mode === 'unbound' ? null : { bus_name: ':1.4', object_path: '/fixture' } } } };
        }
        socket.end(`${JSON.stringify(result)}\n`);
      });
    });
    await new Promise(resolve => server.listen(path, resolve));
    try {
      const child = spawn(process.execPath, ['tools/accessibility/audit-native-bindings.mjs'], {
        env: { ...process.env, NIRI_SOCKET: path }, stdio: ['ignore', 'pipe', 'pipe'],
      });
      let output = '', errors = '';
      child.stdout.on('data', chunk => { output += chunk; });
      child.stderr.on('data', chunk => { errors += chunk; });
      const status = await new Promise(resolve => child.on('exit', resolve));
      assert.equal(status, 0, errors);
      assert(!output.includes('PRIVATE_TITLE_MUST_NOT_LEAK'));
      const row = JSON.parse(output).windows[0];
      assert.equal(row.binding, { annotated: 'surface_annotation_present', unbound: 'no_surface_annotation', unsupported: 'unknown', retired: 'unknown' }[mode]);
      assert.equal(row.app_id, 'fixture.application');
      if (mode === 'unsupported') assert.equal(row.error, 'error parsing request');
      if (mode === 'retired') assert.equal(row.error, 'Window retired during inventory');
      if (mode === 'annotated') assert.match(row.process_start_ticks, /^\d+$/);
    } finally {
      await new Promise(resolve => server.close(resolve));
    }
  }
  console.log('Passed four read-only binding inventory cases.');
} finally {
  await rm(directory, { recursive: true, force: true });
}
