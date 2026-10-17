#!/usr/bin/env node
// Read-only inventory. It never traverses app content or authorizes actions.
import { connect } from 'node:net';
import { readFile, readlink } from 'node:fs/promises';
import { basename } from 'node:path';

const socketPath = process.env.NIRI_SOCKET;
if (!socketPath) throw new Error('NIRI_SOCKET is required');
function request(value) {
  return new Promise((resolve, reject) => {
    const socket = connect(socketPath);
    let response = '';
    socket.setTimeout(3000, () => socket.destroy(new Error('Niri IPC deadline')));
    socket.on('error', reject);
    socket.on('connect', () => socket.end(`${JSON.stringify(value)}\n`));
    socket.on('data', chunk => {
      response += chunk;
      if (response.length > 4 * 1024 * 1024) socket.destroy(new Error('Niri IPC size limit'));
    });
    socket.on('end', () => {
      try {
        const result = JSON.parse(response.trim());
        if ('Err' in result) throw new Error(result.Err);
        resolve(result.Ok);
      } catch (error) { reject(error); }
    });
  });
}
const generation = async pid => {
  const stat = await readFile(`/proc/${pid}/stat`, 'utf8');
  return stat.slice(stat.lastIndexOf(')') + 2).split(' ')[19];
};
const snapshot = (await request('Windows')).Windows;
const windows = [];
for (const window of snapshot) {
  const row = { window_id: window.id, app_id: window.app_id, pid: window.pid, binding: 'unknown' };
  try {
    const before = window.pid ? await generation(window.pid) : null;
    if (window.pid) {
      row.executable = await readlink(`/proc/${window.pid}/exe`);
      const maps = await readFile(`/proc/${window.pid}/maps`, 'utf8');
      row.accessibility_libraries = [...new Set(maps.split('\n').map(line => line.slice(line.indexOf('/')))
        .filter(path => path.startsWith('/') && /\/(lib(?:gtk|Qt[56]|atk|atspi|X11|wayland-client)|[^/]*accesskit)/i.test(path))
        .map(path => basename(path)))].sort();
    }
    const target = (await request({ AccessibilityTarget: { window_id: window.id } })).AccessibilityTarget;
    const after = window.pid ? await generation(window.pid) : null;
    if (!target || target.window_id !== window.id || target.pid !== window.pid || before !== after) throw new Error('Target changed during inventory');
    row.binding = target.endpoint ? 'surface_annotation_present' : 'no_surface_annotation';
    row.generation = target.generation;
    row.annotation_generation = target.annotation_generation;
    row.process_start_ticks = after;
  } catch (error) {
    row.error = error.code ?? error.message;
  }
  windows.push(row);
}
const current = (await request('Windows')).Windows;
for (const row of windows) {
  if (!current.some(window => window.id === row.window_id && window.pid === row.pid)) {
    row.binding = 'unknown'; row.error = 'Window retired during inventory';
  }
}
console.log(JSON.stringify({ schema_version: 1, observed_at: new Date().toISOString(),
  evidence_limit: 'Window and process metadata only. An annotation does not prove bus credentials, a usable tree or semantic action support. No window titles or application content are collected.', windows }, null, 2));
