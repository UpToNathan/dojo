import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { toSnapshot } from './snapshot.js';

test('a snapshot lists each window with its tabs in order, and only what Dojo needs', () => {
  const windows = [
    { id: 7, focused: true, tabs: [
      { id: 12, index: 1, active: true, pinned: false, url: 'https://b.example/', title: 'B', favIconUrl: 'x', status: 'complete' },
      { id: 11, index: 0, active: false, pinned: true, url: 'https://a.example/', title: 'A' },
    ] },
    { id: 9, tabs: [{ id: 20, index: 0, active: true, pendingUrl: 'https://loading.example/' }] },
    { tabs: [{ id: 1 }] },
  ];
  assert.deepEqual(toSnapshot(windows, 1000), {
    v: 1, at: 1000,
    windows: [
      { id: 7, focused: true, tabs: [
        { id: 11, active: false, pinned: true, url: 'https://a.example/', title: 'A' },
        { id: 12, active: true, pinned: false, url: 'https://b.example/', title: 'B' },
      ] },
      { id: 9, focused: false, tabs: [{ id: 20, active: true, pinned: false, url: 'https://loading.example/', title: '' }] },
    ],
  });
});

test('the extension asks for the tabs permission and nothing that lets it change a tab', async () => {
  const manifest = JSON.parse(await readFile(new URL('./manifest.json', import.meta.url), 'utf8'));
  assert.deepEqual(manifest.permissions.sort(), ['nativeMessaging', 'tabs']);
  assert.equal(manifest.incognito, 'not_allowed');
  assert.equal(manifest.manifest_version, 3);
});
